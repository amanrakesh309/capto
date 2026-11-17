//! Application state shared by commands, the tray and the global shortcut.

use capture_core::{CaptureError, CaptureService, ServiceStatus, Settings};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

pub struct AppState {
    /// `None` when no capture backend could be initialised (for example a
    /// Wayland-only session). The window still opens and explains why.
    pub service: Option<CaptureService>,
    /// The settings shown in the window. Kept next to the service so the
    /// window can still display and edit them when there is no backend.
    pub settings: Settings,
    pub startup_error: Option<UiError>,
    pub config_path: PathBuf,
    pub load_warning: Option<String>,
    /// Why the global shortcut could not be registered, if it failed.
    pub shortcut_error: Option<String>,
    /// Why the autostart entry could not be written, if it failed.
    pub autostart_error: Option<String>,
    /// Accelerator currently registered, if any.
    pub shortcut_active: Option<String>,
    /// Serialises shortcut registration, which must not run twice at once.
    pub shortcut_lock: Arc<Mutex<()>>,
    /// Bumped on every toast so an older auto-hide timer does not hide a newer
    /// toast.
    pub toast_generation: Arc<AtomicU64>,
}

impl AppState {
    pub fn service(&mut self) -> Result<&mut CaptureService, UiError> {
        match self.service.as_mut() {
            Some(service) => Ok(service),
            None => Err(self.startup_error.clone().unwrap_or_else(|| {
                UiError::new(
                    "no_backend",
                    "Capturing is not available on this system.",
                    Some("See the compatibility notes in the settings window.".into()),
                )
            })),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.clone()
    }

    /// Apply settings to both the window state and the capture service, then
    /// persist them.
    pub fn apply_settings(&mut self, settings: Settings) -> Result<(), UiError> {
        if let Some(service) = self.service.as_mut() {
            service.update_settings(settings.clone())?;
        }
        self.settings = settings;
        self.persist()
    }

    /// Persist the current settings without changing them.
    pub fn persist(&self) -> Result<(), UiError> {
        self.settings.save(&self.config_path).map_err(UiError::from)
    }
}

/// Error shape understood by the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct UiError {
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
}

impl UiError {
    pub fn new(code: &str, message: impl Into<String>, hint: Option<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            hint,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message, None)
    }
}

impl From<CaptureError> for UiError {
    fn from(err: CaptureError) -> Self {
        Self {
            code: err.code().to_string(),
            message: err.user_message(),
            hint: err.hint(),
        }
    }
}

/// Everything the settings window renders.
#[derive(Debug, Clone, Serialize)]
pub struct AppSnapshot {
    pub settings: Settings,
    pub status: Option<ServiceStatus>,
    pub config_path: String,
    pub load_warning: Option<String>,
    pub shortcut_error: Option<String>,
    pub autostart_error: Option<String>,
    pub startup_error: Option<UiError>,
    pub autostart_enabled: bool,
    pub version: String,
}

pub fn snapshot(state: &mut AppState) -> AppSnapshot {
    let status = match state.service.as_mut() {
        Some(service) => service.status().ok(),
        None => None,
    };
    AppSnapshot {
        settings: state.settings(),
        status,
        config_path: state.config_path.display().to_string(),
        load_warning: state.load_warning.clone(),
        shortcut_error: state.shortcut_error.clone(),
        autostart_error: state.autostart_error.clone(),
        startup_error: state.startup_error.clone(),
        autostart_enabled: crate::autostart::is_enabled(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaphot_serialises_without_a_backend() {
        let state = AppState {
            service: None,
            settings: Settings::default(),
            startup_error: Some(UiError::new("no_backend", "nope", None)),
            config_path: PathBuf::from("/tmp/settings.json"),
            load_warning: None,
            shortcut_error: None,
            autostart_error: None,
            shortcut_active: None,
            shortcut_lock: Arc::new(Mutex::new(())),
            toast_generation: Arc::new(AtomicU64::new(0)),
        };
        let mut state = state;
        let snapshot = snapshot(&mut state);
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["startup_error"]["code"], "no_backend");
        assert!(json["status"].is_null());
        assert_eq!(json["settings"]["format"], "png");
    }
}
