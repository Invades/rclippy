use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};

use crate::icons::TrayIconVariant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Unpair,
    ShowSettings,
    Quit,
}

enum TrayControl {
    SetIcon(TrayIconVariant),
    SetUnpairEnabled(bool),
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

    pub fn set_unpair_enabled(&self, enabled: bool) -> Result<()> {
        self.tx
            .send(TrayControl::SetUnpairEnabled(enabled))
            .context("send tray unpair state update")
    }
}

pub struct TrayHandle {
    tray: TrayIcon,
    unpair: MenuItem,
    _event_thread: thread::JoinHandle<()>,
}

impl TrayHandle {
    pub fn set_icon(&self, variant: TrayIconVariant) -> Result<()> {
        let icon = crate::icons::tray_icon(variant)?;
        self.tray.set_icon(Some(icon)).context("set tray icon")
    }

    pub fn set_unpair_enabled(&self, enabled: bool) {
        self.unpair.set_enabled(enabled);
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
    let tray_menu = build_tray(initial_icon)?;
    let _event_thread = spawn_tray_event_thread(tx, tray_menu.ids.clone());
    Ok(TrayHandle {
        tray: tray_menu.tray,
        unpair: tray_menu.unpair,
        _event_thread,
    })
}

fn run_tray_thread(
    tx: Sender<TrayCommand>,
    control_rx: Receiver<TrayControl>,
    initial_icon: TrayIconVariant,
) -> Result<()> {
    let tray_menu = build_tray(initial_icon)?;

    loop {
        pump_platform_events();

        while let Ok(control) = control_rx.try_recv() {
            match control {
                TrayControl::SetIcon(variant) => {
                    let icon = crate::icons::tray_icon(variant)?;
                    tray_menu
                        .tray
                        .set_icon(Some(icon))
                        .context("set tray icon")?;
                }
                TrayControl::SetUnpairEnabled(enabled) => {
                    tray_menu.unpair.set_enabled(enabled);
                }
            }
        }

        if poll_tray_events(&tx) {
            continue;
        }

        if let Ok(event) = MenuEvent::receiver().recv_timeout(Duration::from_millis(250))
            && handle_menu_event(&tx, &tray_menu.ids, event)
        {
            break;
        }
    }

    Ok(())
}

struct TrayMenu {
    tray: TrayIcon,
    unpair: MenuItem,
    ids: TrayMenuIds,
}

#[derive(Clone)]
struct TrayMenuIds {
    unpair: MenuId,
    settings: MenuId,
    quit: MenuId,
}

fn build_tray(initial_icon: TrayIconVariant) -> Result<TrayMenu> {
    #[cfg(target_os = "windows")]
    crate::windows_theme::enable_dark_menus_if_supported();

    let menu = Menu::new();
    let unpair = MenuItem::new("Unpair", false, None);
    let settings = MenuItem::new("Settings", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let top_separator = PredefinedMenuItem::separator();
    let bottom_separator = PredefinedMenuItem::separator();
    menu.append_items(&[&unpair, &top_separator, &settings, &bottom_separator, &quit])
        .context("build tray menu")?;

    let unpair_id = unpair.id().clone();
    let settings_id = settings.id().clone();
    let quit_id = quit.id().clone();
    let icon = crate::icons::tray_icon(initial_icon)?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(cfg!(target_os = "macos"))
        .with_menu_on_right_click(true)
        .with_tooltip("rclippy")
        .with_icon(icon)
        .build()
        .context("create tray icon")?;

    Ok(TrayMenu {
        tray,
        unpair,
        ids: TrayMenuIds {
            unpair: unpair_id,
            settings: settings_id,
            quit: quit_id,
        },
    })
}

fn spawn_tray_event_thread(
    tx: Sender<TrayCommand>,
    menu_ids: TrayMenuIds,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            let _ = poll_tray_events(&tx);

            if let Ok(event) = MenuEvent::receiver().recv_timeout(Duration::from_millis(250))
                && handle_menu_event(&tx, &menu_ids, event)
            {
                break;
            }
        }
    })
}

fn handle_menu_event(
    tx: &Sender<TrayCommand>,
    menu_ids: &TrayMenuIds,
    event: tray_icon::menu::MenuEvent,
) -> bool {
    if event.id == menu_ids.unpair {
        let _ = tx.send(TrayCommand::Unpair);
        false
    } else if event.id == menu_ids.settings {
        let _ = tx.send(TrayCommand::ShowSettings);
        false
    } else if event.id == menu_ids.quit {
        let _ = tx.send(TrayCommand::Quit);
        true
    } else {
        false
    }
}

fn poll_tray_events(tx: &Sender<TrayCommand>) -> bool {
    let mut handled = false;
    while let Ok(event) = TrayIconEvent::receiver().try_recv() {
        handled = true;
        if is_left_click_release(event) {
            let _ = tx.send(TrayCommand::ShowSettings);
        }
    }
    handled
}

fn is_left_click_release(event: TrayIconEvent) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = event;
        false
    }

    #[cfg(not(target_os = "macos"))]
    matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    )
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
