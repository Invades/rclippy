use std::{sync::Arc, sync::mpsc};

use anyhow::Context;
use eframe::egui;
use rclippy::{
    config::ConfigStore,
    icons,
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
    let initial_icon = icons::tray_icon_variant(config.monochrome_tray_icon, egui::Theme::Dark);
    #[cfg(not(target_os = "macos"))]
    let (_tray_thread, tray_controller) = rclippy::tray::spawn_tray_thread(tray_tx, initial_icon);
    #[cfg(not(target_os = "macos"))]
    let tray_controller = Some(tray_controller);
    #[cfg(target_os = "macos")]
    let tray_controller = None;
    #[cfg(target_os = "macos")]
    let mut tray_tx = Some(tray_tx);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 560.0])
            .with_icon(rclippy::icons::window_icon())
            .with_visible(!minimized),
        ..Default::default()
    };

    eframe::run_native(
        "rclippy",
        options,
        Box::new(move |_cc| {
            #[cfg(target_os = "macos")]
            let tray_handle = tray_tx.take().and_then(|tx| {
                match rclippy::tray::create_tray(
                    tx,
                    icons::tray_icon_variant(
                        config.monochrome_tray_icon,
                        _cc.egui_ctx
                            .system_theme()
                            .unwrap_or_else(|| _cc.egui_ctx.theme()),
                    ),
                ) {
                    Ok(handle) => Some(handle),
                    Err(err) => {
                        eprintln!("rclippy tray failed: {err:#}");
                        None
                    }
                }
            });
            #[cfg(not(target_os = "macos"))]
            let tray_handle = None;

            Ok(Box::new(ui::RclippyApp::new(
                runtime,
                config_store,
                config,
                identity,
                tray_rx,
                tray_controller,
                tray_handle,
            )))
        }),
    )
    .map_err(|err| anyhow::anyhow!(err.to_string()))
}
