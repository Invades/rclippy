#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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

const WINDOW_WIDTH: f32 = 320.0;
const WINDOW_HEIGHT: f32 = 410.0;
const WINDOW_SIZE: [f32; 2] = [WINDOW_WIDTH, WINDOW_HEIGHT];

fn main() -> anyhow::Result<()> {
    prefer_x11_for_hide_to_tray();
    rclippy::transport::install_crypto_provider();

    let minimized = std::env::args().any(|arg| arg == "--minimized");
    let runtime = Arc::new(Runtime::new().context("create Tokio runtime")?);
    let config_store = ConfigStore::new()?;
    let config = config_store.load()?;
    let keychain = KeychainSecretStore;
    let identity = ensure_identity(&keychain).context("load local identity from keychain")?;

    let result = run_with_renderer(
        initial_renderer(),
        minimized,
        runtime.clone(),
        config_store.clone(),
        config.clone(),
        identity.clone(),
    );

    #[cfg(target_os = "linux")]
    let result = match result {
        Ok(()) => Ok(()),
        Err(err) => {
            eprintln!("rclippy renderer failed, retrying with OpenGL: {err}");
            run_with_renderer(
                fallback_renderer(),
                minimized,
                runtime,
                config_store,
                config,
                identity,
            )
        }
    };

    result.map_err(|err| anyhow::anyhow!(err.to_string()))
}

fn prefer_x11_for_hide_to_tray() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WINIT_UNIX_BACKEND").is_none() && std::env::var_os("DISPLAY").is_some() {
        // hacky workaround as winit's Wayland backend cannot hide an already-created window.
        unsafe {
            std::env::set_var("WINIT_UNIX_BACKEND", "x11");
        }
    }
}

fn run_with_renderer(
    renderer: eframe::Renderer,
    minimized: bool,
    runtime: Arc<Runtime>,
    config_store: ConfigStore,
    config: rclippy::config::Config,
    identity: rclippy::secrets::Identity,
) -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(WINDOW_SIZE)
            .with_min_inner_size(WINDOW_SIZE)
            .with_max_inner_size(WINDOW_SIZE)
            .with_resizable(false)
            .with_maximize_button(false)
            .with_maximized(false)
            .with_icon(rclippy::icons::window_icon())
            .with_visible(!minimized),
        ..Default::default()
    };

    eframe::run_native(
        "rclippy",
        options,
        Box::new(move |_cc| {
            let (tray_tx, tray_rx) = mpsc::channel();
            #[cfg(not(target_os = "macos"))]
            let initial_icon =
                icons::tray_icon_variant(config.monochrome_tray_icon, egui::Theme::Dark);
            #[cfg(not(target_os = "macos"))]
            let (_tray_thread, tray_controller) =
                rclippy::tray::spawn_tray_thread(tray_tx, initial_icon);
            #[cfg(not(target_os = "macos"))]
            let tray_controller = Some(tray_controller);
            #[cfg(target_os = "macos")]
            let tray_controller = None;
            #[cfg(target_os = "macos")]
            let mut tray_tx = Some(tray_tx);

            if let Err(err) = rclippy::system_fonts::install(&_cc.egui_ctx) {
                eprintln!("rclippy system font setup failed: {err:#}");
            }

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
                runtime.clone(),
                config_store.clone(),
                config.clone(),
                identity.clone(),
                tray_rx,
                tray_controller,
                tray_handle,
            )))
        }),
    )
}

#[cfg(target_os = "linux")]
fn initial_renderer() -> eframe::Renderer {
    eframe::Renderer::Wgpu
}

#[cfg(not(target_os = "linux"))]
fn initial_renderer() -> eframe::Renderer {
    eframe::Renderer::default()
}

#[cfg(target_os = "linux")]
fn fallback_renderer() -> eframe::Renderer {
    eframe::Renderer::Glow
}
