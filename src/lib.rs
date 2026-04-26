pub mod autostart;
pub mod clipboard;
pub mod config;
pub mod frame;
pub mod icons;
pub mod pairing;
pub mod secrets;
pub mod sync;
pub mod system_fonts;
pub mod transport;
pub mod tray;
pub mod ui;
#[cfg(target_os = "windows")]
pub mod windows_theme;

pub const APP_NAME: &str = "rclippy";
pub const PROTOCOL_VERSION: u16 = 1;
