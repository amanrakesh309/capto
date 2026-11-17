//! Small always-on-top confirmation window.
//!
//! A real desktop notification would need another plugin and depends on a
//! notification daemon. The toast window works on every X11 desktop and can be
//! styled to match the rest of the app.

use crate::state::AppState;
use capture_core::CaptureOutcome;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

const SUCCESS_MS: u64 = 2600;
const ERROR_MS: u64 = 6000;
const MARGIN: i32 = 18;

#[derive(Debug, Clone, Serialize)]
pub struct ToastPayload {
    /// `"success"`, `"error"` or `"info"`.
    pub tone: String,
    pub title: String,
    pub message: String,
    pub detail: Option<String>,
}

pub fn captured(app: &AppHandle, outcome: &CaptureOutcome) {
    let detail = match &outcome.warning {
        Some(warning) => Some(warning.clone()),
        None => Some(format!(
            "{}×{} · {} KB · {}",
            outcome.saved.width,
            outcome.saved.height,
            outcome.saved.bytes / 1024,
            outcome.target_label
        )),
    };
    show(
        app,
        ToastPayload {
            tone: "success".into(),
            title: "Screenshot saved".into(),
            message: outcome.saved.filename.clone(),
            detail,
        },
    );
}

pub fn info(app: &AppHandle, title: impl Into<String>, message: impl Into<String>) {
    show(
        app,
        ToastPayload {
            tone: "info".into(),
            title: title.into(),
            message: message.into(),
            detail: None,
        },
    );
}

pub fn failed(app: &AppHandle, error: &crate::state::UiError) {
    show(
        app,
        ToastPayload {
            tone: "error".into(),
            title: "No screenshot taken".into(),
            message: error.message.clone(),
            detail: error.hint.clone(),
        },
    );
}

pub fn show(app: &AppHandle, payload: ToastPayload) {
    let Some(window) = app.get_webview_window("toast") else {
        log::debug!("toast window is missing; message was: {}", payload.message);
        return;
    };

    position(app, &window);
    if let Err(err) = window.emit("toast", payload.clone()) {
        log::warn!("could not send the toast message: {err}");
    }
    if let Err(err) = window.show() {
        log::warn!("could not show the toast window: {err}");
    }

    let generation = {
        let state = app.state::<Mutex<AppState>>();
        let state = state.inner().lock().unwrap_or_else(|err| err.into_inner());
        state.toast_generation.fetch_add(1, Ordering::SeqCst) + 1
    };

    let duration = if payload.tone == "error" {
        ERROR_MS
    } else {
        SUCCESS_MS
    };
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("toast-hide".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_millis(duration));
            let current = {
                let state = app.state::<Mutex<AppState>>();
                let state = state.inner().lock().unwrap_or_else(|err| err.into_inner());
                state.toast_generation.load(Ordering::SeqCst)
            };
            // A newer toast owns the window now; leave it alone.
            if current == generation {
                if let Some(window) = app.get_webview_window("toast") {
                    let _ = window.hide();
                }
            }
        });
    if let Err(err) = spawned {
        log::warn!("could not start the toast timer: {err}");
    }
}

fn position(app: &AppHandle, window: &tauri::WebviewWindow) {
    let size = window
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(440, 104));
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let area = monitor.work_area();
    let right = area.position.x + area.size.width as i32;
    let bottom = area.position.y + area.size.height as i32;
    // Clamp against the monitor's own top-left, not global zero: a monitor to
    // the left of or above the primary one has negative coordinates and the
    // old `max(0)` moved its toast onto another screen.
    let x = (right - size.width as i32 - MARGIN).max(area.position.x);
    let y = (bottom - size.height as i32 - MARGIN).max(area.position.y);
    if let Err(err) = window.set_position(PhysicalPosition::new(x, y)) {
        log::debug!("could not position the toast: {err}");
    }
}
