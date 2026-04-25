use std::{sync::mpsc::Sender, thread, time::Duration};

use anyhow::{Context, Result};
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ShowSettings,
    Quit,
}

pub fn spawn_tray_thread(tx: Sender<TrayCommand>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(err) = run_tray(tx) {
            eprintln!("rclippy tray failed: {err:#}");
        }
    })
}

fn run_tray(tx: Sender<TrayCommand>) -> Result<()> {
    let menu = Menu::new();
    let settings = MenuItem::new("Settings", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let separator = PredefinedMenuItem::separator();
    menu.append_items(&[&settings, &separator, &quit])
        .context("build tray menu")?;

    let settings_id = settings.id().clone();
    let quit_id = quit.id().clone();
    let icon = tray_icon()?;
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("rclippy")
        .with_icon(icon)
        .build()
        .context("create tray icon")?;

    loop {
        if let Ok(event) = MenuEvent::receiver().recv_timeout(Duration::from_millis(250)) {
            if event.id == settings_id {
                let _ = tx.send(TrayCommand::ShowSettings);
            } else if event.id == quit_id {
                let _ = tx.send(TrayCommand::Quit);
                break;
            }
        }
    }

    Ok(())
}

fn tray_icon() -> Result<Icon> {
    let size = 32_u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let in_mark = (8..=23).contains(&x) && (8..=23).contains(&y);
            let border = x == 8 || x == 23 || y == 8 || y == 23;
            let accent = (x > 12 && x < 20 && y > 11 && y < 22) || (y > 17 && x > 10 && x < 22);
            let (r, g, b, a) = if border {
                (36, 38, 44, 255)
            } else if accent {
                (35, 136, 235, 255)
            } else if in_mark {
                (244, 246, 248, 255)
            } else {
                (0, 0, 0, 0)
            };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    Icon::from_rgba(rgba, size, size).context("build tray icon")
}
