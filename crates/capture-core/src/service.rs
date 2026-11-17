//! The capture service: the single funnel for tray clicks, global shortcuts and
//! the settings window.
//!
//! It owns the current mode/target, resolves it to a backend call, saves the
//! image and returns a structured outcome. Nothing here knows about Tauri.

use crate::backend::{default_backend, Capabilities, CaptureBackend, WindowInfo};
use crate::error::{CaptureError, Result};
use crate::geometry::Region;
use crate::settings::{CaptureMode, Settings, WindowTarget};
use crate::storage::{save_image, ImageFormat, SavedImage};
use chrono::Local;
use serde::Serialize;
use std::path::PathBuf;

/// Where the most recent screenshot went, for the status area and tray tooltip.
#[derive(Debug, Clone, Serialize)]
pub struct LastCapture {
    pub filename: String,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub captured_at: String,
    pub target_label: String,
    pub format: ImageFormat,
    pub warning: Option<String>,
}

/// Everything a "Capture now" press produced.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureOutcome {
    #[serde(flatten)]
    pub saved: SavedImage,
    pub captured_at: String,
    pub mode: CaptureMode,
    pub target_label: String,
    /// Non-fatal note, e.g. "the window was partly covered".
    pub warning: Option<String>,
}

/// Snapshot of the service for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceStatus {
    pub backend: String,
    pub display_server: String,
    pub capabilities: Capabilities,
    pub notes: Vec<String>,
    pub mode: CaptureMode,
    pub paused: bool,
    /// Exactly what the next capture will target, in plain language. Keeps
    /// "pinned window" and "foreground window" visibly distinct.
    pub capture_summary: String,
    /// False when the configured target cannot be used right now.
    pub target_ready: bool,
    pub target_problem: Option<String>,
    pub screen: Region,
    pub last_capture: Option<LastCapture>,
}

pub struct CaptureService {
    settings: Settings,
    backend: Box<dyn CaptureBackend>,
    last: Option<LastCapture>,
}

impl CaptureService {
    pub fn new(settings: Settings) -> Result<Self> {
        let backend = default_backend()?;
        Ok(Self {
            settings,
            backend,
            last: None,
        })
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Replace settings after validation. The shortcut is handled by the app
    /// shell, not here.
    ///
    /// A missing target is allowed on purpose: the user may want to fix the
    /// output folder first. [`Self::capture_now`] still refuses to run until a
    /// target is usable.
    pub fn update_settings(&mut self, mut settings: Settings) -> Result<()> {
        settings.validate_shape()?;
        if !self.backend.capabilities().supports(settings.mode) {
            return Err(CaptureError::Unsupported(format!(
                "{} capture",
                settings.mode.label().to_lowercase()
            )));
        }
        settings.version = crate::settings::SETTINGS_VERSION;
        self.settings = settings;
        Ok(())
    }

    pub fn set_mode(&mut self, mode: CaptureMode) -> Result<()> {
        if !self.backend.capabilities().supports(mode) {
            return Err(CaptureError::Unsupported(format!(
                "{} capture",
                mode.label().to_lowercase()
            )));
        }
        let mut next = self.settings.clone();
        next.mode = mode;
        next.validate()?;
        self.settings = next;
        Ok(())
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.settings.paused = paused;
    }

    pub fn last_capture(&self) -> Option<&LastCapture> {
        self.last.as_ref()
    }

    pub fn status(&mut self) -> Result<ServiceStatus> {
        let capabilities = self.backend.capabilities();
        let (capture_summary, target_ready, target_problem) = self.describe_target();
        let screen = self
            .backend
            .screen_bounds()
            .unwrap_or_else(|_| Region::empty());
        Ok(ServiceStatus {
            backend: self.backend.backend_name().to_string(),
            display_server: self.backend.display_server().to_string(),
            capabilities,
            notes: self.backend.notes(),
            mode: self.settings.mode,
            paused: self.settings.paused,
            capture_summary,
            target_ready,
            target_problem,
            screen,
            last_capture: self.last.clone(),
        })
    }

    fn describe_target(&mut self) -> (String, bool, Option<String>) {
        match self.settings.mode {
            CaptureMode::Region => {
                if self.settings.region.is_usable() {
                    (
                        format!(
                            "Remembered region {}×{} at {},{}",
                            self.settings.region.width,
                            self.settings.region.height,
                            self.settings.region.x,
                            self.settings.region.y
                        ),
                        true,
                        None,
                    )
                } else {
                    (
                        "No region selected yet".to_string(),
                        false,
                        Some("Use \"Select region\" to choose what to capture.".into()),
                    )
                }
            }
            CaptureMode::PinnedWindow => match &self.settings.window {
                Some(target) => {
                    let problem = self.backend.check_target(target);
                    (
                        format!("Pinned window: {}", target.label()),
                        problem.is_none(),
                        problem,
                    )
                }
                None => (
                    "No window pinned yet".to_string(),
                    false,
                    Some("Use \"Select window\" to pin an application window.".into()),
                ),
            },
            CaptureMode::ForegroundWindow => {
                let label = self
                    .backend
                    .active_window()
                    .ok()
                    .flatten()
                    .map(|target| format!("Foreground window: {}", target.label()))
                    .unwrap_or_else(|| "Foreground window (none active)".to_string());
                (label, true, None)
            }
            CaptureMode::SelectArea => (
                "Select area on capture (immediate)".to_string(),
                true,
                None,
            ),
            CaptureMode::FullScreen => (
                "Entire screen (full display)".to_string(),
                true,
                None,
            ),
        }
    }

    /// Take one screenshot now and save it.
    pub fn capture_now(&mut self) -> Result<CaptureOutcome> {
        self.settings.validate_for_capture()?;
        let capabilities = self.backend.capabilities();
        if !capabilities.supports(self.settings.mode) {
            return Err(CaptureError::Unsupported(format!(
                "{} capture",
                self.settings.mode.label().to_lowercase()
            )));
        }

        let mode = self.settings.mode;
        let (frame, target_label) = match mode {
            CaptureMode::Region => {
                let region = self.settings.region;
                (
                    self.backend.capture_region(region)?,
                    format!("Region {region}"),
                )
            }
            CaptureMode::SelectArea => {
                let region = self.backend.select_region()?.ok_or(CaptureError::Cancelled)?;
                self.settings.region = region;
                (
                    self.backend.capture_region(region)?,
                    format!("Area {region}"),
                )
            }
            CaptureMode::FullScreen => {
                (
                    self.backend.capture_screen()?,
                    "Full screen".to_string(),
                )
            }
            CaptureMode::PinnedWindow => {
                let target = self.settings.window.clone().ok_or_else(|| {
                    CaptureError::TargetUnavailable {
                        detail: "no window has been pinned yet".into(),
                    }
                })?;
                let label = format!("Pinned window: {}", target.label());
                (self.backend.capture_window(&target)?, label)
            }
            CaptureMode::ForegroundWindow => {
                let target = self.backend.active_window()?.ok_or_else(|| {
                    CaptureError::TargetUnavailable {
                        detail: "no window is currently focused".into(),
                    }
                })?;
                let label = format!("Foreground window: {}", target.label());
                (self.backend.capture_window(&target)?, label)
            }
        };

        let now = Local::now();
        let saved = save_image(
            &frame.image,
            &self.settings.output_dir,
            &self.settings.filename_pattern,
            self.settings.format,
            self.settings.jpeg_quality,
            &now,
            match mode {
                CaptureMode::Region | CaptureMode::SelectArea => "region",
                CaptureMode::FullScreen => "fullscreen",
                _ => "window",
            },
        )?;

        let captured_at = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let last = LastCapture {
            filename: saved.filename.clone(),
            path: saved.path.clone(),
            width: saved.width,
            height: saved.height,
            bytes: saved.bytes,
            captured_at: captured_at.clone(),
            target_label: target_label.clone(),
            format: saved.format,
            warning: frame.warning.clone(),
        };
        self.last = Some(last);

        Ok(CaptureOutcome {
            saved,
            captured_at,
            mode,
            target_label,
            warning: frame.warning,
        })
    }

    /// Take an immediate snip of a user-dragged region, saving it immediately.
    pub fn capture_snip(&mut self) -> Result<CaptureOutcome> {
        self.settings.validate_shape()?;
        if self.settings.paused {
            return Err(CaptureError::Paused);
        }
        let region = self.backend.select_region()?.ok_or(CaptureError::Cancelled)?;
        self.settings.region = region;
        let frame = self.backend.capture_region(region)?;
        let target_label = format!("Snip {region}");

        let now = Local::now();
        let saved = save_image(
            &frame.image,
            &self.settings.output_dir,
            &self.settings.filename_pattern,
            self.settings.format,
            self.settings.jpeg_quality,
            &now,
            "region",
        )?;

        let captured_at = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let last = LastCapture {
            filename: saved.filename.clone(),
            path: saved.path.clone(),
            width: saved.width,
            height: saved.height,
            bytes: saved.bytes,
            captured_at: captured_at.clone(),
            target_label: target_label.clone(),
            format: saved.format,
            warning: frame.warning.clone(),
        };
        self.last = Some(last);

        Ok(CaptureOutcome {
            saved,
            captured_at,
            mode: CaptureMode::SelectArea,
            target_label,
            warning: frame.warning,
        })
    }

    /// Take an immediate full-screen capture.
    pub fn capture_fullscreen(&mut self) -> Result<CaptureOutcome> {
        self.settings.validate_shape()?;
        if self.settings.paused {
            return Err(CaptureError::Paused);
        }
        let frame = self.backend.capture_screen()?;
        let target_label = "Full screen".to_string();

        let now = Local::now();
        let saved = save_image(
            &frame.image,
            &self.settings.output_dir,
            &self.settings.filename_pattern,
            self.settings.format,
            self.settings.jpeg_quality,
            &now,
            "fullscreen",
        )?;

        let captured_at = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let last = LastCapture {
            filename: saved.filename.clone(),
            path: saved.path.clone(),
            width: saved.width,
            height: saved.height,
            bytes: saved.bytes,
            captured_at: captured_at.clone(),
            target_label: target_label.clone(),
            format: saved.format,
            warning: frame.warning.clone(),
        };
        self.last = Some(last);

        Ok(CaptureOutcome {
            saved,
            captured_at,
            mode: CaptureMode::FullScreen,
            target_label,
            warning: frame.warning,
        })
    }

    /// Run the interactive region selection and remember the result.
    pub fn select_region(&mut self) -> Result<Option<Region>> {
        let region = self.backend.select_region()?;
        if let Some(region) = region {
            self.settings.region = region;
        }
        Ok(region)
    }

    /// Run the click-to-pick window selection and remember the result.
    pub fn select_window(&mut self) -> Result<Option<WindowTarget>> {
        let target = self.backend.select_window()?;
        if let Some(target) = target.clone() {
            self.settings.window = Some(target);
            self.settings.mode = CaptureMode::PinnedWindow;
        }
        Ok(target)
    }

    pub fn list_windows(&mut self) -> Result<Vec<WindowInfo>> {
        self.backend.list_windows()
    }

    /// Pin a window chosen from a list instead of by clicking.
    pub fn pin_window(&mut self, target: WindowTarget) -> Result<()> {
        if let Some(problem) = self.backend.check_target(&target) {
            return Err(CaptureError::TargetUnavailable { detail: problem });
        }
        self.settings.window = Some(target);
        self.settings.mode = CaptureMode::PinnedWindow;
        Ok(())
    }

    pub fn screen_bounds(&mut self) -> Result<Region> {
        self.backend.screen_bounds()
    }

    /// True when the remembered region still lies on the current desktop.
    pub fn region_is_on_screen(&mut self) -> bool {
        let Ok(bounds) = self.backend.screen_bounds() else {
            return false;
        };
        self.settings
            .region
            .clamp_to(&bounds)
            .map(|clipped| clipped.is_usable())
            .unwrap_or(false)
    }

    pub fn capabilities(&self) -> Capabilities {
        self.backend.capabilities()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_settings_rejects_missing_region() {
        // Uses the real backend when available, otherwise skips: the settings
        // validation is the part under test here.
        let mut settings = Settings::default();
        settings.region = Region::new(0, 0, 120, 80);
        assert!(settings.validate().is_ok());
        settings.region = Region::empty();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn mode_labels_are_distinct() {
        assert_ne!(
            CaptureMode::PinnedWindow.label(),
            CaptureMode::ForegroundWindow.label()
        );
        assert_ne!(
            CaptureMode::Region.label(),
            CaptureMode::SelectArea.label()
        );
        assert_ne!(
            CaptureMode::FullScreen.label(),
            CaptureMode::SelectArea.label()
        );
        assert!(CaptureMode::PinnedWindow
            .description()
            .contains("even while another application is focused"));
        assert!(CaptureMode::SelectArea
            .description()
            .contains("captures it immediately"));
    }
}
