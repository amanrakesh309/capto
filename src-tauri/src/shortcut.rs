//! Global shortcut registration.
//!
//! The plugin dispatches every register/unregister call to the main thread and
//! waits for it to finish, so these functions must never run *on* the main
//! thread. [`spawn_apply`] is the entry point used at startup; the settings
//! command calls [`apply`] from its blocking worker.

use crate::commands;
use crate::state::AppState;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

/// Register the shortcut from the stored settings on a background thread.
pub fn spawn_apply(app: AppHandle) {
    let result = std::thread::Builder::new()
        .name("shortcut-register".into())
        .spawn(move || {
            if let Err(err) = apply(&app) {
                log::warn!("global shortcut: {err}");
            }
        });
    if let Err(err) = result {
        log::warn!("could not start the shortcut thread: {err}");
    }
}

/// Make the registered shortcut match the stored setting.
///
/// An empty accelerator means "no global shortcut"; the tray menu always stays
/// available as a fallback.
pub fn apply(app: &AppHandle) -> Result<(), String> {
    let shortcut_lock = {
        let state = app.state::<Mutex<AppState>>();
        state.inner().lock().unwrap().shortcut_lock.clone()
    };
    let _guard = shortcut_lock.lock().unwrap_or_else(|err| err.into_inner());

    let (desired, previous) = {
        let state = app.state::<Mutex<AppState>>();
        let state = state.inner().lock().unwrap_or_else(|err| err.into_inner());
        (
            state.settings.shortcut.trim().to_string(),
            state.shortcut_active.clone(),
        )
    };

    if desired.is_empty() {
        if let Some(previous) = previous.as_deref() {
            let _ = app.global_shortcut().unregister(previous);
        }
        let state = app.state::<Mutex<AppState>>();
        let mut state = state.inner().lock().unwrap_or_else(|err| err.into_inner());
        state.shortcut_active = None;
        state.shortcut_error = None;
        log::info!("global shortcut disabled");
        return Ok(());
    }

    if previous.as_deref() != Some(desired.as_str())
        || !app.global_shortcut().is_registered(desired.as_str())
    {
        if let Some(previous) = previous.as_deref() {
            let _ = app.global_shortcut().unregister(previous);
        }
        match app
            .global_shortcut()
            .on_shortcut(desired.as_str(), on_shortcut_pressed)
        {
            Ok(()) => {
                let state = app.state::<Mutex<AppState>>();
                let mut state = state.inner().lock().unwrap_or_else(|err| err.into_inner());
                state.shortcut_active = Some(desired.clone());
                state.shortcut_error = None;
                log::info!("global shortcut registered: {desired}");
                return Ok(());
            }
            Err(err) => {
                let message = friendly_error(&desired, &err.to_string());
                // Put the previous accelerator back so a failed edit does not
                // leave the user without any shortcut at all.
                let restored = previous.as_deref().and_then(|previous| {
                    app.global_shortcut()
                        .on_shortcut(previous, on_shortcut_pressed)
                        .ok()
                        .map(|()| previous.to_string())
                });
                let state = app.state::<Mutex<AppState>>();
                let mut state = state.inner().lock().unwrap_or_else(|err| err.into_inner());
                state.shortcut_active = restored;
                state.shortcut_error = Some(message.clone());
                return Err(message);
            }
        }
    }

    let state = app.state::<Mutex<AppState>>();
    let mut state = state.inner().lock().unwrap_or_else(|err| err.into_inner());
    state.shortcut_active = Some(desired);
    state.shortcut_error = None;
    Ok(())
}

fn on_shortcut_pressed(app: &AppHandle, _shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state == ShortcutState::Pressed {
        commands::capture_from_trigger(app);
    }
}

fn friendly_error(accelerator: &str, raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let invalid = lower.contains("parse")
        || lower.contains("recognize")
        || lower.contains("unexpected")
        || lower.contains("token");
    let looks_taken = lower.contains("already")
        || lower.contains("badaccess")
        || lower.contains("bad access")
        || lower.contains("register");
    if invalid {
        format!("\"{accelerator}\" is not a valid shortcut. Use a combination such as Ctrl+Alt+S.")
    } else if looks_taken {
        format!(
            "\"{accelerator}\" could not be registered because another application may already use it. Choose a different combination; the tray menu still works."
        )
    } else {
        format!("\"{accelerator}\" could not be registered: {raw}. The tray menu still works.")
    }
}

#[cfg(test)]
mod tests {
    use super::friendly_error;

    #[test]
    fn taken_shortcut_mentions_the_tray_fallback() {
        let message = friendly_error("Ctrl+Alt+S", "HotKey already registered");
        assert!(message.contains("Ctrl+Alt+S"));
        assert!(message.contains("tray menu still works"));
    }

    #[test]
    fn unknown_errors_stay_plain_language() {
        let message = friendly_error("Ctrl+Alt+S", "weird failure");
        assert!(message.contains("could not be registered"));
        assert!(!message.contains("panic"));
    }
}
