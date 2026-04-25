use std::sync::Arc;

use anyhow::{Context, Result};
use eframe::egui;
use tray_icon::Icon;

const WINDOW_SIZE: u32 = 256;

struct TrayIconAsset {
    size: u32,
    color: &'static [u8],
    mono_light: &'static [u8],
    mono_dark: &'static [u8],
}

#[cfg(target_os = "windows")]
const TRAY_ICON_ASSETS: &[TrayIconAsset] = &[
    TrayIconAsset {
        size: 32,
        color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-32.rgba")),
        mono_light: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-32-mono-light.rgba")),
        mono_dark: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-32-mono-dark.rgba")),
    },
    TrayIconAsset {
        size: 40,
        color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-40.rgba")),
        mono_light: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-40-mono-light.rgba")),
        mono_dark: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-40-mono-dark.rgba")),
    },
    TrayIconAsset {
        size: 48,
        color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-48.rgba")),
        mono_light: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-48-mono-light.rgba")),
        mono_dark: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-48-mono-dark.rgba")),
    },
    TrayIconAsset {
        size: 64,
        color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-64.rgba")),
        mono_light: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-64-mono-light.rgba")),
        mono_dark: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-64-mono-dark.rgba")),
    },
    TrayIconAsset {
        size: 96,
        color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-96.rgba")),
        mono_light: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-96-mono-light.rgba")),
        mono_dark: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-96-mono-dark.rgba")),
    },
];

#[cfg(not(target_os = "windows"))]
const TRAY_ICON_ASSETS: &[TrayIconAsset] = &[TrayIconAsset {
    size: 32,
    color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-32.rgba")),
    mono_light: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-32-mono-light.rgba")),
    mono_dark: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-32-mono-dark.rgba")),
}];

const WINDOW_ICON_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-256.rgba"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIconVariant {
    Color,
    MonochromeLight,
    MonochromeDark,
}

pub fn tray_icon_variant(monochrome: bool, system_theme: egui::Theme) -> TrayIconVariant {
    if !monochrome {
        return TrayIconVariant::Color;
    }

    match system_theme {
        egui::Theme::Dark => TrayIconVariant::MonochromeLight,
        egui::Theme::Light => TrayIconVariant::MonochromeDark,
    }
}

pub fn tray_icon(variant: TrayIconVariant) -> Result<Icon> {
    let asset = tray_icon_asset();
    let rgba = match variant {
        TrayIconVariant::Color => asset.color,
        TrayIconVariant::MonochromeLight => asset.mono_light,
        TrayIconVariant::MonochromeDark => asset.mono_dark,
    };
    Icon::from_rgba(rgba.to_vec(), asset.size, asset.size).context("build tray icon")
}

pub fn window_icon() -> Arc<egui::IconData> {
    Arc::new(egui::IconData {
        rgba: WINDOW_ICON_RGBA.to_vec(),
        width: WINDOW_SIZE,
        height: WINDOW_SIZE,
    })
}

fn tray_icon_asset() -> &'static TrayIconAsset {
    let desired = desired_tray_source_size();
    TRAY_ICON_ASSETS
        .iter()
        .min_by_key(|asset| asset.size.abs_diff(desired))
        .unwrap_or(&TRAY_ICON_ASSETS[0])
}

#[cfg(target_os = "windows")]
fn desired_tray_source_size() -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};

    let slot_size = unsafe { GetSystemMetrics(SM_CXSMICON) };
    if slot_size <= 0 {
        return 32;
    }

    (slot_size as u32).saturating_mul(2)
}

#[cfg(not(target_os = "windows"))]
fn desired_tray_source_size() -> u32 {
    32
}
