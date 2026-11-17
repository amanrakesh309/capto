//! Tauri commands plus the workflows shared by the window, the tray and the
//! global shortcut.
//!
//! Every blocking operation (X11 calls, file IO, the folder picker, shortcut
//! registration) runs on a worker thread so the window never freezes. State is
//! protected by one mutex, which also keeps two captures from racing.

use crate::state::{snapshot, AppSnapshot, AppState, UiError};
use crate::{autostart, shortcut, toast, tray};
use capture_core::{
    CaptureError, CaptureMode, CaptureOutcome, Region, Settings, WindowInfo, WindowTarget,
};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

pub type SharedState = Mutex<AppState>;

fn lock(app: &AppHandle) -> MutexGuard<'_, AppState> {
    app.state::<SharedState>()
        .inner()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn blocking<T, F>(work: F) -> Result<T, UiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, UiError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|err| {
            UiError::internal(format!("The background task stopped unexpectedly: {err}"))
        })?
}

fn spawn_worker(name: &str, work: impl FnOnce() + Send + 'static) {
    let result = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(work);
    if let Err(err) = result {
        log::warn!("could not start the {name} worker: {err}");
    }
}

/// A single FIFO worker for every capture request.
///
/// Creating a new thread per key press used to work, but it made the app
/// depend on the thread limit and allowed bursts of presses to fail silently
/// when a spawn was refused. One persistent worker also guarantees that every
/// press is processed in order, even when the previous capture is still
/// encoding or writing the file. The settings window uses the same queue, so
/// two captures can never show or update a toast at the same time.
enum CaptureJob {
    Trigger,
    Command(Sender<Result<CaptureOutcome, UiError>>),
}

pub struct CaptureQueue {
    tx: Sender<CaptureJob>,
}

/// Start the persistent capture worker. Must be called once during setup, after
/// the shared state has been managed.
pub fn start_capture_worker(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<CaptureJob>();
    app.manage(CaptureQueue { tx });

    let spawn = std::thread::Builder::new()
        .name("capture-queue".into())
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                let result = do_capture(&app);
                if let CaptureJob::Command(reply) = job {
                    let _ = reply.send(result);
                }
            }
            log::debug!("capture queue closed");
        });
    if let Err(err) = spawn {
        log::error!("could not start the capture queue: {err}");
    }
}

/// Run a window-initiated capture through the same queue as the shortcut and
/// tray. The direct fallback only matters while the app is shutting down.
fn capture_through_queue(app: &AppHandle) -> Result<CaptureOutcome, UiError> {
    let Some(queue) = app.try_state::<CaptureQueue>() else {
        return do_capture(app);
    };
    let (tx, rx) = mpsc::channel();
    if queue.tx.send(CaptureJob::Command(tx)).is_err() {
        return do_capture(app);
    }
    rx.recv()
        .map_err(|_| UiError::internal("The capture worker stopped unexpectedly."))?
}

fn current_snapshot(app: &AppHandle) -> AppSnapshot {
    let mut state = lock(app);
    snapshot(&mut state)
}

fn emit_state(app: &AppHandle) {
    let state = current_snapshot(app);
    if let Err(err) = app.emit("state", state) {
        log::debug!("could not emit the state update: {err}");
    }
}

// ---------------------------------------------------------------------------
// Commands called from the window
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_state(app: AppHandle) -> Result<AppSnapshot, UiError> {
    blocking(move || Ok(current_snapshot(&app))).await
}

#[tauri::command]
pub async fn capture_now(app: AppHandle) -> Result<CaptureOutcome, UiError> {
    log::info!("capture requested from the settings window");
    blocking(move || capture_through_queue(&app)).await
}

#[tauri::command]
pub async fn capture_snip(app: AppHandle) -> Result<CaptureOutcome, UiError> {
    log::info!("snip requested from settings window");
    blocking(move || do_capture_snip(&app)).await
}

#[tauri::command]
pub async fn capture_fullscreen(app: AppHandle) -> Result<CaptureOutcome, UiError> {
    log::info!("fullscreen capture requested from settings window");
    blocking(move || do_capture_fullscreen(&app)).await
}

#[tauri::command]
pub async fn select_region(app: AppHandle) -> Result<Option<Region>, UiError> {
    blocking(move || {
        let selected = do_select_region(&app)?;
        if let Some(region) = selected {
            toast::info(
                &app,
                "Region selected",
                format!("{region} will be captured from now on."),
            );
        } else {
            toast::info(&app, "Region unchanged", "Selection was cancelled.");
        }
        Ok(selected)
    })
    .await
}

#[tauri::command]
pub async fn select_window(app: AppHandle) -> Result<Option<WindowTarget>, UiError> {
    blocking(move || {
        let selected = do_select_window(&app)?;
        match &selected {
            Some(target) => toast::info(
                &app,
                "Window pinned",
                format!("{} is now the capture target.", target.label()),
            ),
            None => toast::info(&app, "Window unchanged", "Selection was cancelled."),
        }
        Ok(selected)
    })
    .await
}

#[tauri::command]
pub async fn list_windows(app: AppHandle) -> Result<Vec<WindowInfo>, UiError> {
    blocking(move || {
        let mut state = lock(&app);
        state.service()?.list_windows().map_err(UiError::from)
    })
    .await
}

#[tauri::command]
pub async fn pin_window(app: AppHandle, target: WindowTarget) -> Result<AppSnapshot, UiError> {
    blocking(move || {
        {
            let mut state = lock(&app);
            let settings = {
                let service = state.service()?;
                service.pin_window(target).map_err(UiError::from)?;
                service.settings().clone()
            };
            state.settings = settings;
            state.persist()?;
        }
        emit_state(&app);
        tray::refresh(&app);
        Ok(current_snapshot(&app))
    })
    .await
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, settings: Settings) -> Result<AppSnapshot, UiError> {
    blocking(move || {
        let mut settings = settings;

        // Register the shortcut before saving it, so a combination that is
        // already taken does not become the stored value.
        let old_shortcut = lock(&app).settings.shortcut.clone();
        if settings.shortcut.trim() != old_shortcut.trim() {
            {
                let mut state = lock(&app);
                state.settings.shortcut = settings.shortcut.clone();
            }
            if shortcut::apply(&app).is_err() {
                settings.shortcut = old_shortcut;
                let mut state = lock(&app);
                state.settings.shortcut = settings.shortcut.clone();
            }
        } else {
            // Saving unrelated settings is also a chance to retry a shortcut
            // that failed to register at startup (for example because another
            // program held the combination).
            let _ = shortcut::apply(&app);
        }

        {
            let mut state = lock(&app);
            state.apply_settings(settings.clone())?;
        }

        let autostart_error = if autostart::is_enabled() != settings.launch_at_login {
            autostart::set_enabled(settings.launch_at_login).err()
        } else {
            None
        };
        {
            let mut state = lock(&app);
            state.autostart_error = autostart_error;
        }

        emit_state(&app);
        tray::refresh(&app);

        let state = current_snapshot(&app);
        if let Some(error) = &state.shortcut_error {
            toast::info(&app, "Shortcut not changed", error.clone());
        } else if let Some(error) = &state.autostart_error {
            toast::info(&app, "Start at login not changed", error.clone());
        }
        Ok(state)
    })
    .await
}

#[tauri::command]
pub async fn set_paused(app: AppHandle, paused: bool) -> Result<AppSnapshot, UiError> {
    blocking(move || do_set_paused(&app, paused)).await
}

#[tauri::command]
pub async fn choose_folder(
    app: AppHandle,
    current: Option<String>,
) -> Result<Option<String>, UiError> {
    blocking(move || {
        let mut builder = app
            .dialog()
            .file()
            .set_title("Choose a folder for screenshots");
        if let Some(current) = current.as_deref().filter(|dir| !dir.trim().is_empty()) {
            builder = builder.set_directory(current);
        }
        let Some(chosen) = builder.blocking_pick_folder() else {
            return Ok(None);
        };
        let path = chosen.into_path().map_err(|err| {
            UiError::new(
                "folder",
                format!("That folder could not be used: {err}"),
                None,
            )
        })?;
        Ok(Some(path.display().to_string()))
    })
    .await
}

#[tauri::command]
pub async fn open_folder(app: AppHandle) -> Result<(), UiError> {
    blocking(move || do_open_folder(&app)).await
}

#[tauri::command]
pub fn hide_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    spawn_worker("quit", move || {
        // Give the IPC reply a moment to reach the window before exiting.
        std::thread::sleep(Duration::from_millis(120));
        app.exit(0);
    });
}

// ---------------------------------------------------------------------------
// Entry points used by the tray and the global shortcut
// ---------------------------------------------------------------------------

pub fn capture_from_trigger(app: &AppHandle) {
    log::info!("capture requested from the tray or the global shortcut");
    if let Some(queue) = app.try_state::<CaptureQueue>() {
        if queue.tx.send(CaptureJob::Trigger).is_ok() {
            return;
        }
    }

    // The queue only disappears while the app is shutting down. Keep a direct
    // fallback so a press is never dropped silently.
    log::warn!("capture queue unavailable; running the capture directly");
    let app = app.clone();
    spawn_worker("capture", move || {
        let _ = do_capture(&app);
    });
}

pub fn snip_from_tray(app: &AppHandle) {
    let app = app.clone();
    spawn_worker("snip-tray", move || {
        let _ = do_capture_snip(&app);
    });
}

pub fn fullscreen_from_tray(app: &AppHandle) {
    let app = app.clone();
    spawn_worker("fullscreen-tray", move || {
        let _ = do_capture_fullscreen(&app);
    });
}

pub fn select_region_from_tray(app: &AppHandle) {
    let app = app.clone();
    spawn_worker("select-region", move || match do_select_region(&app) {
        Ok(Some(region)) => toast::info(
            &app,
            "Region selected",
            format!("{region} will be captured from now on."),
        ),
        Ok(None) => {}
        Err(error) => toast::failed(&app, &error),
    });
}

pub fn select_window_from_tray(app: &AppHandle) {
    let app = app.clone();
    spawn_worker("select-window", move || match do_select_window(&app) {
        Ok(Some(target)) => toast::info(
            &app,
            "Window pinned",
            format!("{} is now the capture target.", target.label()),
        ),
        Ok(None) => {}
        Err(error) => toast::failed(&app, &error),
    });
}

pub fn open_folder_from_tray(app: &AppHandle) {
    let app = app.clone();
    spawn_worker("open-folder", move || {
        if let Err(error) = do_open_folder(&app) {
            toast::failed(&app, &error);
        }
    });
}

pub fn toggle_pause_from_tray(app: &AppHandle) {
    let app = app.clone();
    spawn_worker("toggle-pause", move || {
        let was_paused = lock(&app).settings.paused;
        match do_set_paused(&app, !was_paused) {
            Ok(_) if was_paused => toast::info(
                &app,
                "Capturing resumed",
                "The shortcut and tray menu take screenshots again.",
            ),
            Ok(_) => toast::info(&app, "Capturing paused", "Use the tray menu to resume."),
            Err(error) => toast::failed(&app, &error),
        }
    });
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

// ---------------------------------------------------------------------------
// Shared workflows
// ---------------------------------------------------------------------------

fn handle_capture_result(app: &AppHandle, result: &Result<CaptureOutcome, UiError>) {
    match result {
        Ok(outcome) => {
            log::info!(
                "saved {} ({}x{}, {} bytes, {}, {})",
                outcome.saved.path.display(),
                outcome.saved.width,
                outcome.saved.height,
                outcome.saved.bytes,
                outcome.saved.format.label(),
                outcome.target_label
            );
            toast::captured(app, outcome);
        }
        Err(error) if error.code == "cancelled" => {}
        Err(error) if error.code == "paused" => {
            toast::info(app, "Capturing is paused", "Resume from the tray menu.");
        }
        Err(error) => {
            log::warn!(
                "capture failed ({}): {}{}",
                error.code,
                error.message,
                error
                    .hint
                    .as_deref()
                    .map(|hint| format!(" — {hint}"))
                    .unwrap_or_default()
            );
            toast::failed(app, error);
        }
    }

    emit_state(app);
    tray::refresh(app);
}

fn do_capture(app: &AppHandle) -> Result<CaptureOutcome, UiError> {
    let mode = lock(app).settings.mode;
    let result = if mode == CaptureMode::SelectArea {
        run_with_main_hidden(app, || {
            let mut state = lock(app);
            state
                .service()
                .and_then(|service| service.capture_now().map_err(UiError::from))
        })
    } else {
        let mut state = lock(app);
        state
            .service()
            .and_then(|service| service.capture_now().map_err(UiError::from))
    };

    handle_capture_result(app, &result);
    result
}

fn do_capture_snip(app: &AppHandle) -> Result<CaptureOutcome, UiError> {
    let result = run_with_main_hidden(app, || {
        let mut state = lock(app);
        state
            .service()
            .and_then(|service| service.capture_snip().map_err(UiError::from))
    });

    handle_capture_result(app, &result);
    result
}

fn do_capture_fullscreen(app: &AppHandle) -> Result<CaptureOutcome, UiError> {
    let result = {
        let mut state = lock(app);
        state
            .service()
            .and_then(|service| service.capture_fullscreen().map_err(UiError::from))
    };

    handle_capture_result(app, &result);
    result
}

fn do_select_region(app: &AppHandle) -> Result<Option<Region>, UiError> {
    let selected = run_with_main_hidden(app, || {
        let mut state = lock(app);
        match state.service()?.select_region() {
            Ok(region) => Ok(region),
            Err(CaptureError::Cancelled) => Ok(None),
            Err(err) => Err(UiError::from(err)),
        }
    })?;

    if let Some(region) = selected {
        {
            let mut state = lock(app);
            let mut settings = state.settings.clone();
            settings.region = region;
            settings.mode = CaptureMode::Region;
            state.apply_settings(settings)?;
        }
        emit_state(app);
        tray::refresh(app);
    }
    Ok(selected)
}

fn do_select_window(app: &AppHandle) -> Result<Option<WindowTarget>, UiError> {
    let selected = run_with_main_hidden(app, || {
        let mut state = lock(app);
        match state.service()?.select_window() {
            Ok(target) => Ok(target),
            Err(CaptureError::Cancelled) => Ok(None),
            Err(err) => Err(UiError::from(err)),
        }
    })?;

    if let Some(target) = &selected {
        {
            let mut state = lock(app);
            let mut settings = state.settings.clone();
            settings.window = Some(target.clone());
            settings.mode = CaptureMode::PinnedWindow;
            state.apply_settings(settings)?;
        }
        emit_state(app);
        tray::refresh(app);
    }
    Ok(selected)
}

fn do_set_paused(app: &AppHandle, paused: bool) -> Result<AppSnapshot, UiError> {
    {
        let mut state = lock(app);
        state.settings.paused = paused;
        if let Some(service) = state.service.as_mut() {
            service.set_paused(paused);
        }
        state.persist()?;
    }
    emit_state(app);
    tray::refresh(app);
    Ok(current_snapshot(app))
}

fn do_open_folder(app: &AppHandle) -> Result<(), UiError> {
    let dir = lock(app).settings.output_dir.clone();
    std::fs::create_dir_all(&dir).map_err(|source| {
        UiError::from(CaptureError::OutputDirectory {
            path: dir.clone(),
            source,
        })
    })?;
    std::process::Command::new("xdg-open")
        .arg(&dir)
        .spawn()
        .map(|_| ())
        .map_err(|err| {
            UiError::new(
                "open_folder",
                format!("Could not open {}: {err}", dir.display()),
                Some("Open the folder from your file manager instead.".into()),
            )
        })
}

/// Hide the settings window while a selection overlay is on screen. The window
/// is shown again only when it was visible to begin with.
fn run_with_main_hidden<T>(app: &AppHandle, work: impl FnOnce() -> T) -> T {
    let window = app.get_webview_window("main");
    let was_visible = window
        .as_ref()
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    if was_visible {
        if let Some(window) = &window {
            let _ = window.hide();
        }
        // Let the window manager unmap the window before the backdrop
        // screenshot is taken, otherwise the overlay captures the app itself.
        std::thread::sleep(Duration::from_millis(180));
    }

    let result = work();

    if was_visible {
        if let Some(window) = &window {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
    result
}
