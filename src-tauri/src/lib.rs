//! Application shell: window, tray icon, global shortcut and wiring.
//!
//! The actual capture logic lives in the `capture-core` crate. This layer only
//! turns desktop events into calls on [`capture_core::CaptureService`] and
//! presents the result.

mod autostart;
mod commands;
mod logging;
mod shortcut;
mod state;
mod toast;
mod tray;

use capture_core::{CaptureMode, CaptureService, Settings};
use state::{AppState, UiError};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::capture_now,
            commands::capture_snip,
            commands::capture_fullscreen,
            commands::select_region,
            commands::select_window,
            commands::list_windows,
            commands::pin_window,
            commands::save_settings,
            commands::set_paused,
            commands::choose_folder,
            commands::open_folder,
            commands::hide_window,
            commands::quit_app,
        ])
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            logging::init(&config_dir);
            log::info!(
                "Continuous Screenshot {} starting ({})",
                env!("CARGO_PKG_VERSION"),
                std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown session".into())
            );

            let config_path = Settings::default_path(&config_dir);
            let first_run = !config_path.exists();
            let (settings, load_warning) = Settings::load(&config_path);
            if let Some(warning) = &load_warning {
                log::warn!("{warning}");
            }

            let (service, startup_error) = match CaptureService::new(settings.clone()) {
                Ok(mut service) => {
                    match service.status() {
                        Ok(status) => log::info!(
                            "capture backend: {} on {}",
                            status.backend,
                            status.display_server
                        ),
                        Err(err) => log::warn!("could not query the capture status: {err}"),
                    }
                    (Some(service), None)
                }
                Err(err) => {
                    log::warn!("capture backend unavailable: {err}");
                    (None, Some(UiError::from(err)))
                }
            };

            // Show the window when the user still has to choose a target (or
            // when something is wrong). Otherwise start quietly in the tray.
            let target_selected = match settings.mode {
                CaptureMode::Region => settings.region.is_usable(),
                CaptureMode::PinnedWindow => settings.window.is_some(),
                CaptureMode::ForegroundWindow | CaptureMode::SelectArea | CaptureMode::FullScreen => true,
            };
            let show_on_start = first_run || startup_error.is_some() || !target_selected;

            app.manage(Mutex::new(AppState {
                service,
                settings,
                startup_error,
                config_path,
                load_warning,
                shortcut_error: None,
                autostart_error: None,
                shortcut_active: None,
                shortcut_lock: Arc::new(Mutex::new(())),
                toast_generation: Arc::new(AtomicU64::new(0)),
            }));

            commands::start_capture_worker(app.handle().clone());
            tray::create(app)?;
            shortcut::spawn_apply(app.handle().clone());

            if show_on_start {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    // Closing the settings window only hides it; the tray icon
                    // stays and keeps the shortcut registered.
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Continuous Screenshot");
}
