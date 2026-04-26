use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use tray_icon::{
    TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};

use crate::icons::TrayIconVariant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ShowSettings,
    Quit,
}

enum TrayControl {
    SetIcon(TrayIconVariant),
}

#[derive(Clone)]
pub struct TrayController {
    tx: Sender<TrayControl>,
}

impl TrayController {
    pub fn set_icon(&self, variant: TrayIconVariant) -> Result<()> {
        self.tx
            .send(TrayControl::SetIcon(variant))
            .context("send tray icon update")
    }
}

pub struct TrayHandle {
    tray: TrayIcon,
    _event_thread: thread::JoinHandle<()>,
}

impl TrayHandle {
    pub fn set_icon(&self, variant: TrayIconVariant) -> Result<()> {
        let icon = crate::icons::tray_icon(variant)?;
        self.tray.set_icon(Some(icon)).context("set tray icon")
    }
}

pub fn spawn_tray_thread(
    tx: Sender<TrayCommand>,
    initial_icon: TrayIconVariant,
) -> (thread::JoinHandle<()>, TrayController) {
    let (control_tx, control_rx) = mpsc::channel();
    let controller = TrayController { tx: control_tx };
    let handle = thread::spawn(move || {
        if let Err(err) = run_tray_thread(tx, control_rx, initial_icon) {
            eprintln!("rclippy tray failed: {err:#}");
        }
    });
    (handle, controller)
}

pub fn create_tray(tx: Sender<TrayCommand>, initial_icon: TrayIconVariant) -> Result<TrayHandle> {
    let (tray, settings_id, quit_id) = build_tray(initial_icon)?;
    let _event_thread = spawn_menu_event_thread(tx, settings_id, quit_id);
    Ok(TrayHandle {
        tray,
        _event_thread,
    })
}

fn run_tray_thread(
    tx: Sender<TrayCommand>,
    control_rx: Receiver<TrayControl>,
    initial_icon: TrayIconVariant,
) -> Result<()> {
    let (tray, settings_id, quit_id) = build_tray(initial_icon)?;

    loop {
        pump_platform_events();

        while let Ok(control) = control_rx.try_recv() {
            match control {
                TrayControl::SetIcon(variant) => {
                    let icon = crate::icons::tray_icon(variant)?;
                    tray.set_icon(Some(icon)).context("set tray icon")?;
                }
            }
        }

        if let Ok(event) = MenuEvent::receiver().recv_timeout(Duration::from_millis(250))
            && handle_menu_event(&tx, &settings_id, &quit_id, event)
        {
            break;
        }
    }

    Ok(())
}

fn build_tray(initial_icon: TrayIconVariant) -> Result<(TrayIcon, MenuId, MenuId)> {
    let menu = Menu::new();
    let settings = MenuItem::new("Settings", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let separator = PredefinedMenuItem::separator();
    menu.append_items(&[&settings, &separator, &quit])
        .context("build tray menu")?;

    let settings_id = settings.id().clone();
    let quit_id = quit.id().clone();
    let icon = crate::icons::tray_icon(initial_icon)?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_right_click(true)
        .with_tooltip("rclippy")
        .with_icon(icon)
        .build()
        .context("create tray icon")?;

    Ok((tray, settings_id, quit_id))
}

fn spawn_menu_event_thread(
    tx: Sender<TrayCommand>,
    settings_id: MenuId,
    quit_id: MenuId,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            if let Ok(event) = MenuEvent::receiver().recv_timeout(Duration::from_millis(250))
                && handle_menu_event(&tx, &settings_id, &quit_id, event)
            {
                break;
            }
        }
    })
}

fn handle_menu_event(
    tx: &Sender<TrayCommand>,
    settings_id: &MenuId,
    quit_id: &MenuId,
    event: tray_icon::menu::MenuEvent,
) -> bool {
    if event.id == *settings_id {
        let _ = tx.send(TrayCommand::ShowSettings);
        false
    } else if event.id == *quit_id {
        let _ = tx.send(TrayCommand::Quit);
        true
    } else {
        false
    }
}

#[cfg(target_os = "windows")]
fn pump_platform_events() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };

    unsafe {
        let mut msg = std::mem::zeroed::<MSG>();
        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn pump_platform_events() {}
