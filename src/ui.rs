use std::sync::{Arc, mpsc};

use eframe::egui;
use tokio::runtime::Runtime;

use crate::{
    APP_NAME, autostart,
    config::{Config, ConfigStore},
    icons::{self, TrayIconVariant},
    pairing::{generate_pairing_code, host_pairing_once, join_pairing},
    secrets::{
        Identity, KeychainSecretStore, PeerIdentity, SecretStore, delete_peer, load_peer,
        store_peer,
    },
    sync::{SyncHandle, SyncStatusSnapshot, start_background_sync},
    tray::{TrayCommand, TrayController, TrayHandle},
};

enum UiEvent {
    PairingFinished(Result<PeerIdentity, String>),
    Tray(TrayCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairingMode {
    Host,
    Client,
}

impl PairingMode {
    fn label(self) -> &'static str {
        match self {
            Self::Host => "Host: show pairing code",
            Self::Client => "Client: enter pairing code",
        }
    }
}

pub struct RclippyApp {
    runtime: Arc<Runtime>,
    config_store: ConfigStore,
    config: Config,
    secrets: Arc<dyn SecretStore>,
    identity: Identity,
    sync: Option<SyncHandle>,
    tx: mpsc::Sender<UiEvent>,
    rx: mpsc::Receiver<UiEvent>,
    tray_rx: mpsc::Receiver<TrayCommand>,
    tray_controller: Option<TrayController>,
    _tray_handle: Option<TrayHandle>,
    last_tray_icon: Option<TrayIconVariant>,
    status_message: String,
    pairing_mode: PairingMode,
    pair_code: Option<String>,
    join_addr: String,
    join_code: String,
    pairing_busy: bool,
}

impl RclippyApp {
    pub fn new(
        runtime: Arc<Runtime>,
        config_store: ConfigStore,
        config: Config,
        identity: Identity,
        tray_rx: mpsc::Receiver<TrayCommand>,
        tray_controller: Option<TrayController>,
        tray_handle: Option<TrayHandle>,
    ) -> Self {
        let secrets: Arc<dyn SecretStore> = Arc::new(KeychainSecretStore);
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            runtime,
            config_store,
            join_addr: config.peer_addr.clone(),
            config,
            secrets,
            identity,
            sync: None,
            tx,
            rx,
            tray_rx,
            tray_controller,
            _tray_handle: tray_handle,
            last_tray_icon: None,
            status_message: String::new(),
            pairing_mode: PairingMode::Host,
            pair_code: None,
            join_code: String::new(),
            pairing_busy: false,
        };
        app.restart_sync();
        app
    }

    fn restart_sync(&mut self) {
        if let Some(mut sync) = self.sync.take() {
            sync.stop();
        }

        let peer = match load_peer(self.secrets.as_ref()) {
            Ok(peer) => peer,
            Err(err) => {
                self.status_message = format!("Load peer failed: {err}");
                None
            }
        };

        self.sync = Some(start_background_sync(
            self.runtime.handle(),
            self.config.clone(),
            self.identity.clone(),
            peer,
        ));
    }

    fn save_settings(&mut self, update_autostart: bool, restart_sync: bool) {
        match self.config_store.save(&self.config) {
            Ok(()) => {
                if update_autostart
                    && let Err(err) = autostart::set_enabled(self.config.start_on_login)
                {
                    self.status_message = format!("Saved, but start-on-login failed: {err}");
                } else {
                    self.status_message = "Saved".to_owned();
                }
                if restart_sync {
                    self.restart_sync();
                }
            }
            Err(err) => self.status_message = format!("Save failed: {err}"),
        }
    }

    fn start_host_pairing(&mut self) {
        if self.pairing_busy {
            return;
        }

        let code = generate_pairing_code();
        let addr = match self.config.listen_socket_addr() {
            Ok(addr) => addr,
            Err(err) => {
                self.status_message = format!("Bad listen address: {err}");
                return;
            }
        };

        self.pairing_busy = true;
        self.pair_code = Some(code.clone());
        self.status_message = "Waiting for peer".to_owned();
        if let Some(sync) = &mut self.sync {
            sync.stop();
        }

        let tx = self.tx.clone();
        let identity = self.identity.clone();
        self.runtime.spawn(async move {
            let result = host_pairing_once(addr, &code, &identity)
                .await
                .map_err(|err| err.to_string());
            let _ = tx.send(UiEvent::PairingFinished(result));
        });
    }

    fn start_join_pairing(&mut self) {
        if self.pairing_busy {
            return;
        }

        let addr = match self.join_addr.parse() {
            Ok(addr) => addr,
            Err(err) => {
                self.status_message = format!("Bad peer address: {err}");
                return;
            }
        };

        let code = self.join_code.trim().to_owned();
        if code.is_empty() {
            self.status_message = "Pairing code required".to_owned();
            return;
        }

        self.pairing_busy = true;
        self.status_message = "Pairing".to_owned();
        if let Some(sync) = &mut self.sync {
            sync.stop();
        }

        let tx = self.tx.clone();
        let identity = self.identity.clone();
        self.runtime.spawn(async move {
            let result = join_pairing(addr, &code, &identity)
                .await
                .map_err(|err| err.to_string());
            let _ = tx.send(UiEvent::PairingFinished(result));
        });
    }

    fn handle_pairing_result(&mut self, result: Result<PeerIdentity, String>) {
        self.pairing_busy = false;
        self.pair_code = None;

        match result {
            Ok(peer) => {
                if let Err(err) = store_peer(self.secrets.as_ref(), &peer) {
                    self.status_message = format!("Store peer failed: {err}");
                    return;
                }
                if !self.join_addr.trim().is_empty() {
                    self.config.peer_addr = self.join_addr.trim().to_owned();
                    let _ = self.config_store.save(&self.config);
                }
                self.status_message = format!(
                    "Paired with {} ({})",
                    peer.device_id,
                    &peer.cert_fingerprint()[..12]
                );
                self.restart_sync();
            }
            Err(err) => {
                self.status_message = format!("Pairing failed: {err}");
                self.restart_sync();
            }
        }
    }

    fn unpair(&mut self) {
        match delete_peer(self.secrets.as_ref()) {
            Ok(()) => {
                self.status_message = "Unpaired".to_owned();
                self.restart_sync();
            }
            Err(err) => self.status_message = format!("Unpair failed: {err}"),
        }
    }

    fn poll_events(&mut self, ctx: &egui::Context) {
        while let Ok(command) = self.tray_rx.try_recv() {
            let _ = self.tx.send(UiEvent::Tray(command));
        }

        while let Ok(event) = self.rx.try_recv() {
            match event {
                UiEvent::PairingFinished(result) => self.handle_pairing_result(result),
                UiEvent::Tray(TrayCommand::ShowSettings) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                UiEvent::Tray(TrayCommand::Quit) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn sync_status(&self) -> SyncStatusSnapshot {
        self.sync
            .as_ref()
            .map(|sync| sync.status().snapshot())
            .unwrap_or_default()
    }

    fn update_tray_icon(&mut self, ctx: &egui::Context) {
        let theme = ctx.system_theme().unwrap_or_else(|| ctx.theme());
        let variant = icons::tray_icon_variant(self.config.monochrome_tray_icon, theme);
        if self.last_tray_icon == Some(variant) {
            return;
        }

        if let Some(handle) = &self._tray_handle
            && let Err(err) = handle.set_icon(variant)
        {
            self.status_message = format!("Tray icon failed: {err}");
            return;
        }

        if let Some(controller) = &self.tray_controller
            && let Err(err) = controller.set_icon(variant)
        {
            self.status_message = format!("Tray icon failed: {err}");
            return;
        }

        self.last_tray_icon = Some(variant);
    }
}

impl eframe::App for RclippyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_events(&ctx);
        self.update_tray_icon(&ctx);

        egui::Frame::NONE.inner_margin(12).show(ui, |ui| {
            ui.heading(APP_NAME);
            ui.separator();

            let status = self.sync_status();
            ui.horizontal(|ui| {
                ui.label("Status");
                ui.strong(if status.connected {
                    "Connected"
                } else if status.paired {
                    "Paired"
                } else {
                    "Not paired"
                });
            });
            ui.label(status.message);
            if let Some(err) = status.last_error {
                ui.colored_label(egui::Color32::from_rgb(180, 40, 40), err);
            }
            if let Some(peer) = status.peer_device_id {
                ui.label(format!("Peer: {peer}"));
            }
            if let Some(fingerprint) = status.peer_fingerprint {
                ui.label(format!("Peer cert: {}", &fingerprint[..16]));
            }

            ui.add_space(12.0);
            ui.heading("Settings");
            let mut settings_changed = false;
            let mut restart_sync = false;
            let mut update_autostart = false;

            ui.horizontal(|ui| {
                ui.label("Listen");
                let response = ui.text_edit_singleline(&mut self.config.listen_addr);
                if response.changed() {
                    settings_changed = true;
                    restart_sync = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Peer");
                let response = ui.text_edit_singleline(&mut self.config.peer_addr);
                if response.changed() {
                    self.join_addr = self.config.peer_addr.clone();
                    settings_changed = true;
                    restart_sync = true;
                }
            });
            if ui
                .checkbox(&mut self.config.start_on_login, "Start on login")
                .changed()
            {
                settings_changed = true;
                update_autostart = true;
            }
            if ui
                .checkbox(
                    &mut self.config.monochrome_tray_icon,
                    "Monochrome tray icon",
                )
                .changed()
            {
                settings_changed = true;
            }
            ui.horizontal(|ui| {
                ui.label("Poll ms");
                if ui
                    .add(egui::DragValue::new(&mut self.config.poll_ms).range(100..=10_000))
                    .changed()
                {
                    settings_changed = true;
                    restart_sync = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Max bytes");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.config.max_text_bytes).range(1..=16_777_216),
                    )
                    .changed()
                {
                    settings_changed = true;
                    restart_sync = true;
                }
            });
            if settings_changed {
                self.save_settings(update_autostart, restart_sync);
            }

            ui.add_space(12.0);
            ui.heading("Pairing");
            ui.horizontal(|ui| {
                ui.label("Role");
                egui::ComboBox::from_id_salt("pairing_mode")
                    .selected_text(self.pairing_mode.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.pairing_mode,
                            PairingMode::Host,
                            PairingMode::Host.label(),
                        );
                        ui.selectable_value(
                            &mut self.pairing_mode,
                            PairingMode::Client,
                            PairingMode::Client.label(),
                        );
                    });
            });
            ui.horizontal(|ui| {
                if ui.button("Unpair").clicked() {
                    self.unpair();
                }
            });

            match self.pairing_mode {
                PairingMode::Host => {
                    ui.horizontal(|ui| {
                        ui.label("Listen");
                        ui.monospace(&self.config.listen_addr);
                    });
                    if ui
                        .add_enabled(!self.pairing_busy, egui::Button::new("Show pairing code"))
                        .clicked()
                    {
                        self.start_host_pairing();
                    }
                    if let Some(code) = &self.pair_code {
                        ui.monospace(format!("Code: {code}"));
                    }
                }
                PairingMode::Client => {
                    ui.horizontal(|ui| {
                        ui.label("Host addr");
                        ui.text_edit_singleline(&mut self.join_addr);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Code");
                        ui.text_edit_singleline(&mut self.join_code);
                        if ui
                            .add_enabled(!self.pairing_busy, egui::Button::new("Pair"))
                            .clicked()
                        {
                            self.start_join_pairing();
                        }
                    });
                }
            }

            if !self.status_message.is_empty() {
                ui.add_space(8.0);
                ui.label(&self.status_message);
            }
        });

        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
    }
}
