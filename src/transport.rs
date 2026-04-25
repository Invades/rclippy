use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result, bail};
use rustls::{
    ClientConfig, RootCertStore, ServerConfig, server::WebPkiClientVerifier, version::TLS13,
};
use rustls_pki_types::ServerName;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::{
    PROTOCOL_VERSION,
    frame::{Frame, read_frame, write_frame},
    secrets::{Identity, PeerIdentity},
};

const SERVER_NAME: &str = "rclippy.local";
const ALPN: &[u8] = b"rclippy-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionSide {
    Incoming,
    Outgoing,
}

pub type ClientTlsStream = tokio_rustls::client::TlsStream<TcpStream>;
pub type ServerTlsStream = tokio_rustls::server::TlsStream<TcpStream>;

pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub fn client_config(identity: &Identity, peer: &PeerIdentity) -> Result<ClientConfig> {
    install_crypto_provider();
    let mut roots = RootCertStore::empty();
    roots.add(peer.cert()).context("add pinned peer cert")?;

    let mut config = ClientConfig::builder_with_protocol_versions(&[&TLS13])
        .with_root_certificates(roots)
        .with_client_auth_cert(vec![identity.cert()], identity.private_key())
        .context("build client TLS config")?;
    config.alpn_protocols.push(ALPN.to_vec());
    Ok(config)
}

pub fn server_config(identity: &Identity, peer: &PeerIdentity) -> Result<ServerConfig> {
    install_crypto_provider();
    let mut roots = RootCertStore::empty();
    roots.add(peer.cert()).context("add pinned client cert")?;
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .context("build client cert verifier")?;

    let mut config = ServerConfig::builder_with_protocol_versions(&[&TLS13])
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![identity.cert()], identity.private_key())
        .context("build server TLS config")?;
    config.alpn_protocols.push(ALPN.to_vec());
    Ok(config)
}

pub async fn connect_tls(
    addr: SocketAddr,
    identity: &Identity,
    peer: &PeerIdentity,
) -> Result<ClientTlsStream> {
    let stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("connect peer {addr}"))?;
    let connector = TlsConnector::from(Arc::new(client_config(identity, peer)?));
    let server_name = ServerName::try_from(SERVER_NAME)
        .context("parse TLS server name")?
        .to_owned();
    connector
        .connect(server_name, stream)
        .await
        .context("TLS client handshake")
}

pub async fn accept_tls(
    listener: &TcpListener,
    identity: &Identity,
    peer: &PeerIdentity,
) -> Result<ServerTlsStream> {
    let (stream, _) = listener.accept().await.context("accept peer TCP")?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config(identity, peer)?));
    acceptor
        .accept(stream)
        .await
        .context("TLS server handshake")
}

pub async fn client_hello(stream: &mut ClientTlsStream, identity: &Identity) -> Result<String> {
    write_frame(stream, &Frame::hello(&identity.device_id)).await?;
    read_peer_hello(stream).await
}

pub async fn server_hello(stream: &mut ServerTlsStream, identity: &Identity) -> Result<String> {
    let peer_id = read_peer_hello(stream).await?;
    write_frame(stream, &Frame::hello(&identity.device_id)).await?;
    Ok(peer_id)
}

pub async fn read_peer_hello<S>(stream: &mut S) -> Result<String>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let frame = read_frame(stream).await?;
    match frame {
        Frame::Hello {
            protocol_version,
            device_id,
        } => {
            if protocol_version != PROTOCOL_VERSION {
                bail!("unsupported peer protocol version {protocol_version}");
            }
            Ok(device_id)
        }
        _ => bail!("expected peer hello"),
    }
}

pub fn should_keep_connection(
    local_device_id: &str,
    peer_device_id: &str,
    side: ConnectionSide,
) -> bool {
    match local_device_id.cmp(peer_device_id) {
        std::cmp::Ordering::Less => side == ConnectionSide::Outgoing,
        std::cmp::Ordering::Greater => side == ConnectionSide::Incoming,
        std::cmp::Ordering::Equal => false,
    }
}

pub fn verify_expected_peer(actual: &str, expected: &PeerIdentity) -> Result<()> {
    if actual != expected.device_id {
        bail!("peer device id mismatch");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::generate_identity;

    #[test]
    fn deterministic_connection_winner() {
        assert!(should_keep_connection(
            "aaa",
            "bbb",
            ConnectionSide::Outgoing
        ));
        assert!(!should_keep_connection(
            "aaa",
            "bbb",
            ConnectionSide::Incoming
        ));
        assert!(should_keep_connection(
            "bbb",
            "aaa",
            ConnectionSide::Incoming
        ));
        assert!(!should_keep_connection(
            "bbb",
            "aaa",
            ConnectionSide::Outgoing
        ));
    }

    #[test]
    fn builds_tls_configs_with_pinned_cert() {
        let a = generate_identity().unwrap();
        let b = generate_identity().unwrap();
        let peer_b = PeerIdentity {
            device_id: b.device_id.clone(),
            cert_der: b.cert_der.clone(),
        };
        let peer_a = PeerIdentity {
            device_id: a.device_id.clone(),
            cert_der: a.cert_der.clone(),
        };

        client_config(&a, &peer_b).unwrap();
        server_config(&b, &peer_a).unwrap();
    }
}
