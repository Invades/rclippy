use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
    sync::broadcast,
    task::JoinHandle as TokioJoinHandle,
    time::{sleep, timeout},
};

use crate::{
    clipboard::{ClipboardProvider, SystemClipboard},
    config::Config,
    frame::{Frame, read_frame, sha256_text, validate_clipboard_frame, write_frame},
    secrets::{Identity, PeerIdentity},
    transport::{
        ConnectionSide, accept_tls, client_hello, connect_tls, server_hello,
        should_keep_connection, verify_expected_peer,
    },
};

#[derive(Debug, Clone)]
pub struct SyncStatusSnapshot {
    pub paired: bool,
    pub connected: bool,
    pub message: String,
    pub last_error: Option<String>,
    pub peer_device_id: Option<String>,
    pub peer_fingerprint: Option<String>,
}

impl Default for SyncStatusSnapshot {
    fn default() -> Self {
        Self {
            paired: false,
            connected: false,
            message: "Not paired".to_owned(),
            last_error: None,
            peer_device_id: None,
            peer_fingerprint: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct SyncStatus {
    inner: Arc<Mutex<SyncStatusSnapshot>>,
}

impl SyncStatus {
    pub fn snapshot(&self) -> SyncStatusSnapshot {
        self.inner.lock().expect("sync status poisoned").clone()
    }

    pub fn set_paired(&self, peer: &PeerIdentity) {
        let mut status = self.inner.lock().expect("sync status poisoned");
        status.paired = true;
        status.peer_device_id = Some(peer.device_id.clone());
        status.peer_fingerprint = Some(peer.cert_fingerprint());
    }

    pub fn set_connected(&self, connected: bool, message: impl Into<String>) {
        let mut status = self.inner.lock().expect("sync status poisoned");
        status.connected = connected;
        status.message = message.into();
        if connected {
            status.last_error = None;
        }
    }

    pub fn set_error(&self, message: impl Into<String>) {
        let message = message.into();
        let mut status = self.inner.lock().expect("sync status poisoned");
        status.connected = false;
        status.message = message.clone();
        status.last_error = Some(message);
    }
}

#[derive(Debug, Default)]
pub struct EchoSuppressor {
    last_local_hash: Option<String>,
    last_remote_hash: Option<String>,
    next_seq: u64,
}

impl EchoSuppressor {
    pub fn local_frame_for_text(&mut self, text: String, max_text_bytes: usize) -> Option<Frame> {
        if text.len() > max_text_bytes {
            return None;
        }

        let hash = sha256_text(&text);
        if self.last_remote_hash.as_deref() == Some(hash.as_str()) {
            self.last_local_hash = Some(hash);
            return None;
        }

        if self.last_local_hash.as_deref() == Some(hash.as_str()) {
            return None;
        }

        self.next_seq += 1;
        self.last_local_hash = Some(hash.clone());
        Some(Frame::ClipboardText {
            seq: self.next_seq,
            sha256: hash,
            text,
        })
    }

    pub fn remote_text_to_apply(
        &mut self,
        frame: &Frame,
        max_text_bytes: usize,
    ) -> Result<Option<String>> {
        validate_clipboard_frame(frame, max_text_bytes).map_err(anyhow::Error::msg)?;
        let Frame::ClipboardText { sha256, text, .. } = frame else {
            return Ok(None);
        };

        if self.last_remote_hash.as_deref() == Some(sha256.as_str()) {
            return Ok(None);
        }

        if self.last_local_hash.as_deref() == Some(sha256.as_str()) {
            self.last_remote_hash = Some(sha256.clone());
            return Ok(None);
        }

        self.last_remote_hash = Some(sha256.clone());
        self.last_local_hash = Some(sha256.clone());
        Ok(Some(text.clone()))
    }
}

pub struct SyncHandle {
    shutdown: Arc<AtomicBool>,
    tasks: Vec<TokioJoinHandle<()>>,
    clipboard_thread: Option<thread::JoinHandle<()>>,
    status: SyncStatus,
}

impl SyncHandle {
    pub fn idle(message: impl Into<String>) -> Self {
        let status = SyncStatus::default();
        status.set_connected(false, message);
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            tasks: Vec::new(),
            clipboard_thread: None,
            status,
        }
    }

    pub fn status(&self) -> SyncStatus {
        self.status.clone()
    }

    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        for task in &self.tasks {
            task.abort();
        }
        if let Some(thread) = self.clipboard_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for SyncHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start_background_sync(
    runtime: &tokio::runtime::Handle,
    config: Config,
    identity: Identity,
    peer: Option<PeerIdentity>,
) -> SyncHandle {
    let status = SyncStatus::default();
    let shutdown = Arc::new(AtomicBool::new(false));

    let Some(peer) = peer else {
        status.set_connected(false, "Not paired");
        return SyncHandle {
            shutdown,
            tasks: Vec::new(),
            clipboard_thread: None,
            status,
        };
    };

    status.set_paired(&peer);
    if !config.has_peer_addr() {
        status.set_connected(false, "Peer address not configured");
        return SyncHandle {
            shutdown,
            tasks: Vec::new(),
            clipboard_thread: None,
            status,
        };
    }

    let (outbound_tx, _) = broadcast::channel::<Frame>(64);
    let echo = Arc::new(Mutex::new(EchoSuppressor::default()));

    let clipboard_thread = spawn_clipboard_thread(
        config.clone(),
        shutdown.clone(),
        echo.clone(),
        outbound_tx.clone(),
        status.clone(),
    );

    let incoming_task = runtime.spawn(incoming_loop(
        config.clone(),
        identity.clone(),
        peer.clone(),
        shutdown.clone(),
        outbound_tx.clone(),
        echo.clone(),
        status.clone(),
    ));

    let outgoing_task = runtime.spawn(outgoing_loop(
        config,
        identity,
        peer,
        shutdown.clone(),
        outbound_tx,
        echo,
        status.clone(),
    ));

    SyncHandle {
        shutdown,
        tasks: vec![incoming_task, outgoing_task],
        clipboard_thread: Some(clipboard_thread),
        status,
    }
}

fn spawn_clipboard_thread(
    config: Config,
    shutdown: Arc<AtomicBool>,
    echo: Arc<Mutex<EchoSuppressor>>,
    outbound_tx: broadcast::Sender<Frame>,
    status: SyncStatus,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut clipboard = match SystemClipboard::new() {
            Ok(clipboard) => clipboard,
            Err(err) => {
                status.set_error(format!("Clipboard unavailable: {err}"));
                return;
            }
        };

        while !shutdown.load(Ordering::Relaxed) {
            match clipboard.get_text() {
                Ok(Some(text)) => {
                    let frame = echo
                        .lock()
                        .expect("echo suppressor poisoned")
                        .local_frame_for_text(text, config.max_text_bytes);
                    if let Some(frame) = frame {
                        let _ = outbound_tx.send(frame);
                    }
                }
                Ok(None) => {}
                Err(err) => status.set_error(format!("Clipboard read failed: {err}")),
            }
            thread::sleep(config.poll_interval());
        }
    })
}

async fn incoming_loop(
    config: Config,
    identity: Identity,
    peer: PeerIdentity,
    shutdown: Arc<AtomicBool>,
    outbound_tx: broadcast::Sender<Frame>,
    echo: Arc<Mutex<EchoSuppressor>>,
    status: SyncStatus,
) {
    let listener = match TcpListener::bind(&config.listen_addr).await {
        Ok(listener) => listener,
        Err(err) => {
            status.set_error(format!("Listen failed: {err:#}"));
            return;
        }
    };

    while !shutdown.load(Ordering::Relaxed) {
        match timeout(
            Duration::from_secs(1),
            accept_tls(&listener, &identity, &peer),
        )
        .await
        {
            Ok(Ok(mut stream)) => {
                let peer_id = match server_hello(&mut stream, &identity).await {
                    Ok(peer_id) => peer_id,
                    Err(err) => {
                        status.set_error(format!("Incoming hello failed: {err:#}"));
                        continue;
                    }
                };
                if let Err(err) = verify_expected_peer(&peer_id, &peer) {
                    status.set_error(format!("Incoming peer rejected: {err:#}"));
                    continue;
                }
                if !should_keep_connection(
                    &identity.device_id,
                    &peer.device_id,
                    ConnectionSide::Incoming,
                ) {
                    continue;
                }
                let rx = outbound_tx.subscribe();
                run_connection(
                    stream,
                    config.clone(),
                    shutdown.clone(),
                    rx,
                    echo.clone(),
                    status.clone(),
                    "Connected inbound",
                )
                .await;
            }
            Ok(Err(err)) => status.set_error(format!("Incoming TLS failed: {err:#}")),
            Err(_) => {}
        }
    }
}

async fn outgoing_loop(
    config: Config,
    identity: Identity,
    peer: PeerIdentity,
    shutdown: Arc<AtomicBool>,
    outbound_tx: broadcast::Sender<Frame>,
    echo: Arc<Mutex<EchoSuppressor>>,
    status: SyncStatus,
) {
    let peer_addr = match config.peer_socket_addr() {
        Ok(addr) => addr,
        Err(err) => {
            status.set_error(format!("Bad peer address: {err:#}"));
            return;
        }
    };

    let mut backoff = Duration::from_secs(1);
    while !shutdown.load(Ordering::Relaxed) {
        match connect_tls(peer_addr, &identity, &peer).await {
            Ok(mut stream) => {
                let peer_id = match client_hello(&mut stream, &identity).await {
                    Ok(peer_id) => peer_id,
                    Err(err) => {
                        status.set_error(format!("Outgoing hello failed: {err:#}"));
                        sleep(backoff).await;
                        continue;
                    }
                };
                if let Err(err) = verify_expected_peer(&peer_id, &peer) {
                    status.set_error(format!("Outgoing peer rejected: {err:#}"));
                    sleep(backoff).await;
                    continue;
                }
                if !should_keep_connection(
                    &identity.device_id,
                    &peer.device_id,
                    ConnectionSide::Outgoing,
                ) {
                    sleep(backoff).await;
                    continue;
                }
                backoff = Duration::from_secs(1);
                let rx = outbound_tx.subscribe();
                run_connection(
                    stream,
                    config.clone(),
                    shutdown.clone(),
                    rx,
                    echo.clone(),
                    status.clone(),
                    "Connected outbound",
                )
                .await;
            }
            Err(err) => {
                status.set_connected(false, format!("Waiting for peer: {err:#}"));
                sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

async fn run_connection<S>(
    stream: S,
    config: Config,
    shutdown: Arc<AtomicBool>,
    mut outbound_rx: broadcast::Receiver<Frame>,
    echo: Arc<Mutex<EchoSuppressor>>,
    status: SyncStatus,
    connected_message: &'static str,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    status.set_connected(true, connected_message);
    let (mut reader, mut writer) = tokio::io::split(stream);

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        tokio::select! {
            frame = outbound_rx.recv() => {
                match frame {
                    Ok(frame) => {
                        if let Err(err) = write_frame(&mut writer, &frame).await {
                            status.set_error(format!("Send failed: {err:#}"));
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            frame = read_frame(&mut reader) => {
                match frame {
                    Ok(frame @ Frame::ClipboardText { .. }) => {
                        let text = {
                            let mut echo = echo.lock().expect("echo suppressor poisoned");
                            match echo.remote_text_to_apply(&frame, config.max_text_bytes) {
                                Ok(text) => text,
                                Err(err) => {
                                    status.set_error(format!("Remote clipboard rejected: {err:#}"));
                                    None
                                }
                            }
                        };
                        if let Some(text) = text
                            && let Err(err) = set_system_clipboard_text(text).await
                        {
                            status.set_error(format!("Clipboard write failed: {err:#}"));
                        }
                    }
                    Ok(Frame::Ping) | Ok(Frame::Pong) => {}
                    Ok(Frame::Error { message }) => status.set_error(format!("Peer error: {message}")),
                    Ok(Frame::Hello { .. }) => status.set_error("Unexpected hello"),
                    Err(err) => {
                        status.set_connected(false, format!("Disconnected: {err:#}"));
                        break;
                    }
                }
            }
        }
    }
    status.set_connected(false, "Disconnected");
}

async fn set_system_clipboard_text(text: String) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut clipboard = SystemClipboard::new()?;
        clipboard.set_text(&text)
    })
    .await
    .context("clipboard task failed")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_duplicate_does_not_emit() {
        let mut suppressor = EchoSuppressor::default();

        assert!(
            suppressor
                .local_frame_for_text("hello".to_owned(), 1024)
                .is_some()
        );
        assert!(
            suppressor
                .local_frame_for_text("hello".to_owned(), 1024)
                .is_none()
        );
    }

    #[test]
    fn remote_then_local_echo_does_not_emit() {
        let mut suppressor = EchoSuppressor::default();
        let frame = Frame::clipboard_text(1, "from peer".to_owned());

        let apply = suppressor.remote_text_to_apply(&frame, 1024).unwrap();
        assert_eq!(apply.as_deref(), Some("from peer"));

        assert!(
            suppressor
                .local_frame_for_text("from peer".to_owned(), 1024)
                .is_none()
        );
    }
}
