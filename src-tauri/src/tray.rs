//! The tray icon and its menu.
//!
//! The menu is the fallback for every action in the app: it must keep working
//! when the global shortcut is unavailable or conflicts with another program.

use crate::commands;
use crate::state::AppState;
use std::sync::Mutex;
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuEvent, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{App, AppHandle, Manager, Wry};

/// The tray icon plus the menu entries whose text or enabled state changes.
pub struct TrayHandles {
    icon: TrayIcon<Wry>,
    target: MenuItem<tauri::Wry>,
    capture: MenuItem<tauri::Wry>,
    snip: MenuItem<tauri::Wry>,
    fullscreen: MenuItem<tauri::Wry>,
    select_region: MenuItem<tauri::Wry>,
    select_window: MenuItem<tauri::Wry>,
    open_folder: MenuItem<tauri::Wry>,
    pause: MenuItem<tauri::Wry>,
}

pub fn create(app: &App) -> tauri::Result<()> {
    let summary = target_summary(app.handle());
    let target = MenuItem::with_id(
        app,
        "target",
        format!("Target: {summary}"),
        false,
        None::<&str>,
    )?;
    let pause = MenuItem::with_id(app, "pause", pause_label(app.handle()), true, None::<&str>)?;

    let menu = MenuBuilder::new(app)
        .item(&target)
        .separator()
        .text("capture", "Capture now")
        .text("snip", "Snip area now")
        .text("fullscreen", "Capture full screen")
        .text("select_region", "Select remembered region…")
        .text("select_window", "Pin a window…")
        .text("open_folder", "Open screenshot folder")
        .separator()
        .item(&pause)
        .separator()
        .text("settings", "Settings…")
        .text("quit", "Quit")
        .build()?;

    let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .menu(&menu)
        .tooltip(format!("Continuous Screenshot — {summary}"))
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app)?;

    // Re-fetch the generated items so their text can be updated later.
    let find = |id: &str| match menu.get(id) {
        Some(tauri::menu::MenuItemKind::MenuItem(item)) => item,
        _ => unreachable!("menu item {id} is missing"),
    };
    let handles = TrayHandles {
        icon: tray,
        target: find("target"),
        capture: find("capture"),
        snip: find("snip"),
        fullscreen: find("fullscreen"),
        select_region: find("select_region"),
        select_window: find("select_window"),
        open_folder: find("open_folder"),
        pause,
    };
    app.manage(handles);
    Ok(())
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "capture" => commands::capture_from_trigger(app),
        "snip" => commands::snip_from_tray(app),
        "fullscreen" => commands::fullscreen_from_tray(app),
        "select_region" => commands::select_region_from_tray(app),
        "select_window" => commands::select_window_from_tray(app),
        "open_folder" => commands::open_folder_from_tray(app),
        "pause" => commands::toggle_pause_from_tray(app),
        "settings" => commands::show_main_window(app),
        "quit" => app.exit(0),
        other => log::debug!("unhandled tray menu entry: {other}"),
    }
}

/// Update the live parts of the menu after a state change.
///
/// Must not be called while the app state lock is held.
pub fn refresh(app: &AppHandle) {
    let (summary, paused, ready, capabilities) = {
        let state = app.state::<Mutex<AppState>>();
        let mut state = state.inner().lock().unwrap_or_else(|err| err.into_inner());
        let (summary, ready, capabilities) = match state.service.as_mut() {
            Some(service) => {
                let caps = service.capabilities();
                match service.status() {
                    Ok(status) => (status.capture_summary, status.target_ready, caps),
                    Err(_) => ("Status unavailable".to_string(), false, caps),
                }
            }
            None => (
                "Capturing is not available".to_string(),
                false,
                capture_core::Capabilities::NONE,
            ),
        };
        (summary, state.settings.paused, ready, capabilities)
    };

    let Some(handles) = app.try_state::<TrayHandles>() else {
        return;
    };
    let _ = handles.target.set_text(format!("Target: {summary}"));
    let _ = handles.pause.set_text(if paused {
        "Resume capturing"
    } else {
        "Pause capturing"
    });
    let _ = handles.capture.set_enabled(ready && !paused);
    let _ = handles
        .snip
        .set_enabled(capabilities.interactive_region_selection && !paused);
    let _ = handles
        .fullscreen
        .set_enabled(capabilities.full_screen && !paused);
    let _ = handles
        .select_region
        .set_enabled(capabilities.interactive_region_selection);
    let _ = handles
        .select_window
        .set_enabled(capabilities.interactive_window_selection);
    let _ = handles.open_folder.set_enabled(true);

    let _ = handles.icon.set_tooltip(Some(format!(
        "Continuous Screenshot — {summary}{}",
        if paused { " (paused)" } else { "" }
    )));
}

fn target_summary(app: &AppHandle) -> String {
    let state = app.state::<Mutex<AppState>>();
    let mut state = state.inner().lock().unwrap_or_else(|err| err.into_inner());
    match state.service.as_mut() {
        Some(service) => service
            .status()
            .map(|status| status.capture_summary)
            .unwrap_or_else(|_| "Status unavailable".into()),
        None => "Capturing is not available".into(),
    }
}

fn pause_label(app: &AppHandle) -> &'static str {
    let state = app.state::<Mutex<AppState>>();
    let state = state.inner().lock().unwrap_or_else(|err| err.into_inner());
    if state.settings.paused {
        "Resume capturing"
    } else {
        "Pause capturing"
    }
}
