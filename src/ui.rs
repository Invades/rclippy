use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use eframe::egui;
use tokio::{runtime::Runtime, task::JoinHandle};

use crate::{
    APP_NAME, autostart,
    config::{Config, ConfigStore, PairingRole},
    icons::{self, TrayIconVariant},
    pairing::{generate_pairing_code, host_pairing_once, join_pairing, normalize_pairing_code},
    secrets::{
        Identity, KeychainSecretStore, PeerIdentity, SecretStore, delete_peer, load_peer,
        store_peer,
    },
    sync::{SyncHandle, SyncStatusSnapshot, send_unpair_notice, start_background_sync},
    tray::{TrayCommand, TrayController, TrayHandle},
};

enum UiEvent {
    PairingFinished(Result<PeerIdentity, String>),
    Tray(TrayCommand),
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
    last_tray_unpair_enabled: Option<bool>,
    title_icon: Option<egui::TextureHandle>,
    quit_requested: bool,
    status_message: String,
    pair_code: Option<String>,
    pair_code_expires_at: Option<Instant>,
    pairing_task: Option<JoinHandle<()>>,
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
            last_tray_unpair_enabled: None,
            title_icon: None,
            quit_requested: false,
            status_message: String::new(),
            pair_code: None,
            pair_code_expires_at: None,
            pairing_task: None,
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
            self.secrets.clone(),
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
        let expires_at = Instant::now() + Duration::from_secs(30);
        let addr = match self.config.listen_socket_addr() {
            Ok(addr) => addr,
            Err(err) => {
                self.status_message = format!("Bad listen address: {err}");
                return;
            }
        };

        self.pairing_busy = true;
        self.pair_code = Some(code.clone());
        self.pair_code_expires_at = Some(expires_at);
        self.status_message = "Waiting for peer".to_owned();
        if let Some(sync) = &mut self.sync {
            sync.stop();
        }

        let tx = self.tx.clone();
        let identity = self.identity.clone();
        self.pairing_task = Some(self.runtime.spawn(async move {
            let result = host_pairing_once(addr, &code, &identity)
                .await
                .map_err(|err| err.to_string());
            let _ = tx.send(UiEvent::PairingFinished(result));
        }));
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

        let code = match normalize_pairing_code(&self.join_code) {
            Ok(code) => code,
            Err(err) => {
                self.status_message = err.to_string();
                return;
            }
        };

        self.pairing_busy = true;
        self.status_message = "Pairing".to_owned();
        if let Some(sync) = &mut self.sync {
            sync.stop();
        }

        let tx = self.tx.clone();
        let identity = self.identity.clone();
        self.pairing_task = Some(self.runtime.spawn(async move {
            let result = join_pairing(addr, &code, &identity)
                .await
                .map_err(|err| err.to_string());
            let _ = tx.send(UiEvent::PairingFinished(result));
        }));
    }

    fn handle_pairing_result(&mut self, result: Result<PeerIdentity, String>) {
        self.pairing_busy = false;
        self.pair_code = None;
        self.pair_code_expires_at = None;
        self.pairing_task = None;

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
                self.status_message = format!("Paired with {}", peer.display_name());
                self.restart_sync();
            }
            Err(err) => {
                self.status_message = format!("Pairing failed: {err}");
                self.restart_sync();
            }
        }
    }

    fn cancel_pairing(&mut self) {
        if let Some(task) = self.pairing_task.take() {
            task.abort();
        }
        self.pairing_busy = false;
        self.pair_code = None;
        self.pair_code_expires_at = None;
        self.status_message = "Pairing cancelled".to_owned();
        self.restart_sync();
    }

    fn unpair(&mut self) {
        let peer = match load_peer(self.secrets.as_ref()) {
            Ok(peer) => peer,
            Err(err) => {
                self.status_message = format!("Load peer failed: {err}");
                None
            }
        };

        if let Some(sync) = &self.sync {
            sync.notify_unpair();
        }

        if let Some(peer) = peer
            && self.config.has_peer_addr()
        {
            let config = self.config.clone();
            let identity = self.identity.clone();
            let _ = self.runtime.block_on(async move {
                tokio::time::timeout(
                    std::time::Duration::from_millis(750),
                    send_unpair_notice(config, identity, peer),
                )
                .await
            });
        }

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
                UiEvent::Tray(TrayCommand::Unpair) => self.unpair(),
                UiEvent::Tray(TrayCommand::ShowSettings) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                UiEvent::Tray(TrayCommand::Quit) => {
                    self.quit_requested = true;
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

    fn update_tray_unpair_state(&mut self, paired: bool) {
        if self.last_tray_unpair_enabled == Some(paired) {
            return;
        }

        if let Some(handle) = &self._tray_handle {
            handle.set_unpair_enabled(paired);
        }

        if let Some(controller) = &self.tray_controller
            && let Err(err) = controller.set_unpair_enabled(paired)
        {
            self.status_message = format!("Tray menu failed: {err}");
            return;
        }

        self.last_tray_unpair_enabled = Some(paired);
    }

    fn title_icon(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        self.title_icon
            .get_or_insert_with(|| {
                ctx.load_texture(
                    "rclippy-title-icon",
                    icons::title_icon_image(),
                    egui::TextureOptions::LINEAR,
                )
            })
            .clone()
    }

    fn hide_on_close_request(&mut self, ctx: &egui::Context) {
        if self.quit_requested {
            return;
        }

        if ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    }
}

impl eframe::App for RclippyApp {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events(ctx);
        self.hide_on_close_request(ctx);
        self.update_tray_icon(ctx);
        self.update_tray_unpair_state(self.sync_status().paired);
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let title_icon = self.title_icon(&ctx);

        egui::Frame::NONE.inner_margin(12).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Image::new(&title_icon).fit_to_exact_size(egui::vec2(28.0, 28.0)));
                ui.heading(APP_NAME);
            });
            ui.separator();

            let status = self.sync_status();
            ui.horizontal(|ui| {
                ui.label("Status");
                ui.strong(if status.connected {
                    "Connected"
                } else if status.paired {
                    "Paired (Disconnected)"
                } else {
                    "Not paired"
                });
            });
            if let Some(err) = status.last_error {
                ui.colored_label(egui::Color32::from_rgb(180, 40, 40), err);
            }

            ui.add_space(12.0);
            ui.heading("Settings");
            let mut settings_changed = false;
            let mut restart_sync = false;
            let mut update_autostart = false;

            ui.horizontal(|ui| {
                ui.label("Peer Address");
                let response = ui.text_edit_singleline(&mut self.config.peer_addr);
                if response.changed() {
                    self.join_addr = self.config.peer_addr.clone();
                    settings_changed = true;
                    restart_sync = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Listen Port");
                let mut listen_port = self.config.listen_port().unwrap_or(38765);
                if ui
                    .add(egui::DragValue::new(&mut listen_port).range(1..=u16::MAX))
                    .changed()
                {
                    self.config.set_listen_port(listen_port);
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
            ui.add_space(12.0);
            ui.heading("Pairing");
            ui.horizontal(|ui| {
                ui.label("Role");
                egui::ComboBox::from_id_salt("pairing_mode")
                    .selected_text(self.config.pairing_role.label())
                    .show_ui(ui, |ui| {
                        let host = ui.selectable_value(
                            &mut self.config.pairing_role,
                            PairingRole::Host,
                            PairingRole::Host.label(),
                        );
                        let client = ui.selectable_value(
                            &mut self.config.pairing_role,
                            PairingRole::Client,
                            PairingRole::Client.label(),
                        );
                        if host.changed() || client.changed() {
                            settings_changed = true;
                        }
                    });
            });
            if settings_changed {
                self.save_settings(update_autostart, restart_sync);
            }
            if status.paired {
                ui.horizontal(|ui| {
                    if ui.button("Unpair").clicked() {
                        self.unpair();
                    }
                });
            }

            match self.config.pairing_role {
                PairingRole::Host => {
                    if !status.paired {
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    !self.pairing_busy,
                                    egui::Button::new("Show pairing code"),
                                )
                                .clicked()
                            {
                                self.start_host_pairing();
                            }
                            if self.pairing_busy && ui.button("Cancel").clicked() {
                                self.cancel_pairing();
                            }
                        });
                        if let Some(code) = &self.pair_code {
                            let seconds = self
                                .pair_code_expires_at
                                .and_then(|deadline| {
                                    deadline.checked_duration_since(Instant::now())
                                })
                                .map(|remaining| remaining.as_secs().saturating_add(1))
                                .unwrap_or(0);
                            ui.monospace(format!("Code: {code} ({seconds}s)"));
                        }
                    }
                }
                PairingRole::Client => {
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

            let status_message = if status.peer_unpaired {
                status.message.as_str()
            } else {
                self.status_message.as_str()
            };
            if !status_message.is_empty() {
                ui.add_space(8.0);
                ui.label(status_message);
            }
        });

        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
    }
}
