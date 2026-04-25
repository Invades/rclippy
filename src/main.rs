use std::{sync::Arc, sync::mpsc};

use anyhow::Context;
use eframe::egui;
use rclippy::{
    config::ConfigStore,
    secrets::{KeychainSecretStore, ensure_identity},
    ui,
};
use tokio::runtime::Runtime;

fn main() -> anyhow::Result<()> {
    rclippy::transport::install_crypto_provider();

    let minimized = std::env::args().any(|arg| arg == "--minimized");
    let runtime = Arc::new(Runtime::new().context("create Tokio runtime")?);
    let config_store = ConfigStore::new()?;
    let config = config_store.load()?;
    let keychain = KeychainSecretStore;
    let identity = ensure_identity(&keychain).context("load local identity from keychain")?;

    let (tray_tx, tray_rx) = mpsc::channel();
    #[cfg(not(target_os = "macos"))]
    let _tray_thread = rclippy::tray::spawn_tray_thread(tray_tx);
    #[cfg(target_os = "macos")]
    let _tray_tx = tray_tx;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 560.0])
            .with_visible(!minimized),
        ..Default::default()
    };

    eframe::run_native(
        "rclippy",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(ui::RclippyApp::new(
                runtime,
                config_store,
                config,
                identity,
                tray_rx,
            )))
        }),
    )
    .map_err(|err| anyhow::anyhow!(err.to_string()))
}
