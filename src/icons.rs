use std::sync::Arc;

use anyhow::{Context, Result};
use eframe::egui;
use tray_icon::Icon;

const WINDOW_SIZE: u32 = 256;

struct TrayIconAsset {
    size: u32,
    color: &'static [u8],
    mono: &'static [u8],
}

#[cfg(target_os = "windows")]
const TRAY_ICON_ASSETS: &[TrayIconAsset] = &[
    TrayIconAsset {
        size: 32,
        color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-32.rgba")),
        mono: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-32-mono.rgba")),
    },
    TrayIconAsset {
        size: 40,
        color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-40.rgba")),
        mono: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-40-mono.rgba")),
    },
    TrayIconAsset {
        size: 48,
        color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-48.rgba")),
        mono: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-48-mono.rgba")),
    },
    TrayIconAsset {
        size: 64,
        color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-64.rgba")),
        mono: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-64-mono.rgba")),
    },
    TrayIconAsset {
        size: 96,
        color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-96.rgba")),
        mono: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-96-mono.rgba")),
    },
];

#[cfg(not(target_os = "windows"))]
const TRAY_ICON_ASSETS: &[TrayIconAsset] = &[TrayIconAsset {
    size: 32,
    color: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-32.rgba")),
    mono: include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-32-mono.rgba")),
}];

const WINDOW_ICON_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-256.rgba"));
const TITLE_ICON_SIZE: usize = 64;
const TITLE_ICON_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-64.rgba"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIconVariant {
    Color,
    Monochrome,
}

pub fn tray_icon_variant(monochrome: bool, _system_theme: egui::Theme) -> TrayIconVariant {
    if !monochrome {
        return TrayIconVariant::Color;
    }

    TrayIconVariant::Monochrome
}

pub fn tray_icon(variant: TrayIconVariant) -> Result<Icon> {
    let asset = tray_icon_asset();
    let rgba = match variant {
        TrayIconVariant::Color => asset.color,
        TrayIconVariant::Monochrome => asset.mono,
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

pub fn title_icon_image() -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([TITLE_ICON_SIZE, TITLE_ICON_SIZE], TITLE_ICON_RGBA)
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
