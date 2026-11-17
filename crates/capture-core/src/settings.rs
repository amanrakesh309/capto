//! Persisted preferences.
//!
//! Settings are stored as JSON in the app config directory. The loader is
//! deliberately forgiving: a corrupt file is backed up and defaults are used,
//! and a saved window/region that no longer exists is kept in the file but
//! reported as unusable by [`Settings::validate_for_capture`].

use crate::error::{CaptureError, Result};
use crate::geometry::Region;
use crate::naming::{pattern_has_timestamp_token, DEFAULT_PATTERN};
use crate::storage::ImageFormat;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SETTINGS_VERSION: u32 = 1;
pub const DEFAULT_SHORTCUT: &str = "Ctrl+Alt+S";

/// What a single "Capture now" press should capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    /// The remembered rectangle, re-captured on every press.
    Region,
    /// Prompts to drag an area and captures it immediately upon release.
    SelectArea,
    /// A window chosen once; captured even while another app is focused.
    PinnedWindow,
    /// Whichever window is focused at the moment of the press.
    ForegroundWindow,
    /// The entire virtual screen / desktop.
    FullScreen,
}

impl CaptureMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Region => "region",
            Self::SelectArea => "select_area",
            Self::PinnedWindow => "pinned_window",
            Self::ForegroundWindow => "foreground_window",
            Self::FullScreen => "full_screen",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Region => "Remembered region",
            Self::SelectArea => "Select area on capture",
            Self::PinnedWindow => "Pinned window",
            Self::ForegroundWindow => "Foreground window",
            Self::FullScreen => "Full screen",
        }
    }

    /// Short explanation shown under the mode selector. Keeps the distinction
    /// between modes explicit.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Region => {
                "Every capture uses the same rectangle. Use \"Select region\" to change it."
            }
            Self::SelectArea => {
                "Prompts you to select an area on the screen and captures it immediately."
            }
            Self::PinnedWindow => {
                "Every capture uses this window, even while another application is focused."
            }
            Self::ForegroundWindow => {
                "Every capture uses whichever window is focused when you press the shortcut."
            }
            Self::FullScreen => {
                "Every capture takes the entire screen."
            }
        }
    }
}

impl Default for CaptureMode {
    fn default() -> Self {
        Self::Region
    }
}

/// Identity of a chosen X11 window, persisted so it can be re-captured later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowTarget {
    /// X11 window id (client window, not the window manager frame).
    pub window_id: u32,
    /// Title at selection time, shown in the UI.
    pub title: String,
    /// `WM_CLASS` instance/class at selection time, used to detect id reuse.
    pub class: String,
    /// `_NET_WM_PID` at selection time, used to detect id reuse.
    pub pid: Option<u32>,
}

impl WindowTarget {
    /// Compact label for menus and status lines.
    pub fn label(&self) -> String {
        let title = self.title.trim();
        if !title.is_empty() {
            let mut short: String = title.chars().take(48).collect();
            if title.chars().count() > 48 {
                short.push('…');
            }
            short
        } else if !self.class.is_empty() {
            self.class.clone()
        } else {
            format!("window {:#x}", self.window_id)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub mode: CaptureMode,
    pub region: Region,
    pub window: Option<WindowTarget>,
    pub output_dir: PathBuf,
    pub format: ImageFormat,
    pub jpeg_quality: u8,
    pub filename_pattern: String,
    /// Global shortcut in Tauri accelerator syntax, e.g. `Ctrl+Alt+S`.
    /// An empty string disables the global shortcut.
    pub shortcut: String,
    pub launch_at_login: bool,
    pub show_toast: bool,
    pub paused: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            mode: CaptureMode::default(),
            region: Region::empty(),
            window: None,
            output_dir: default_output_dir(),
            format: ImageFormat::Png,
            jpeg_quality: 90,
            filename_pattern: DEFAULT_PATTERN.to_string(),
            shortcut: DEFAULT_SHORTCUT.to_string(),
            launch_at_login: false,
            show_toast: true,
            paused: false,
        }
    }
}

impl Settings {
    /// Checks that do not depend on a selected target being present.
    ///
    /// The settings window saves values one group at a time, so it must be
    /// possible to change the output folder (or the shortcut) before a region
    /// or window has been chosen.
    pub fn validate_shape(&self) -> Result<()> {
        if self.filename_pattern.trim().is_empty() {
            return Err(CaptureError::Settings(
                "the filename pattern must not be empty".into(),
            ));
        }
        if !(1..=100).contains(&self.jpeg_quality) {
            return Err(CaptureError::Settings(
                "JPEG quality must be between 1 and 100".into(),
            ));
        }
        if self.output_dir.as_os_str().is_empty() {
            return Err(CaptureError::Settings(
                "choose an output folder for screenshots".into(),
            ));
        }
        Ok(())
    }

    /// Validation applied whenever settings are saved with a usable target.
    pub fn validate(&self) -> Result<()> {
        self.validate_shape()?;
        if self.mode == CaptureMode::PinnedWindow && self.window.is_none() {
            return Err(CaptureError::Settings(
                "select a window before switching to pinned-window capture".into(),
            ));
        }
        if self.mode == CaptureMode::Region && !self.region.is_usable() {
            return Err(CaptureError::Settings(
                "select a region before switching to region capture".into(),
            ));
        }
        Ok(())
    }

    /// Validation applied right before a capture. This is intentionally more
    /// forgiving than [`Settings::validate`] so a stale saved target produces a
    /// clear runtime error instead of blocking the app from starting.
    pub fn validate_for_capture(&self) -> Result<()> {
        if self.paused {
            return Err(CaptureError::Paused);
        }
        match self.mode {
            CaptureMode::Region => {
                if !self.region.is_usable() {
                    return Err(CaptureError::InvalidRegion(
                        "no region has been selected yet".into(),
                    ));
                }
            }
            CaptureMode::PinnedWindow => {
                if self.window.is_none() {
                    return Err(CaptureError::TargetUnavailable {
                        detail: "no window has been pinned yet".into(),
                    });
                }
            }
            CaptureMode::ForegroundWindow | CaptureMode::SelectArea | CaptureMode::FullScreen => {}
        }
        if self.filename_pattern.trim().is_empty() {
            return Err(CaptureError::Settings(
                "the filename pattern must not be empty".into(),
            ));
        }
        Ok(())
    }

    /// Warning when the saved pattern has no timestamp, because repeated
    /// captures rely on collision suffixes only.
    pub fn filename_pattern_warning(&self) -> Option<String> {
        if pattern_has_timestamp_token(&self.filename_pattern) {
            None
        } else {
            Some(
                "The filename pattern has no {date}/{time} token, so names rely on numbered suffixes."
                    .into(),
            )
        }
    }

    /// Read settings from `path`. A missing file yields defaults; a corrupt
    /// file is moved aside so the user does not lose it silently.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(err) => (
                Self::default(),
                Some(format!(
                    "Could not read settings ({}); using defaults.",
                    err
                )),
            ),
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(mut settings) => {
                    let mut warning = None;
                    if settings.version != SETTINGS_VERSION {
                        settings.version = SETTINGS_VERSION;
                        warning = Some("Settings were upgraded to the current format.".into());
                    }
                    if settings.jpeg_quality == 0 || settings.jpeg_quality > 100 {
                        settings.jpeg_quality = 90;
                    }
                    (settings, warning)
                }
                Err(err) => {
                    let backup = path.with_extension("json.bak");
                    let _ = std::fs::rename(path, &backup);
                    (
                        Self::default(),
                        Some(format!(
                            "Settings could not be read ({}). The old file was kept as {}.",
                            err,
                            backup.display()
                        )),
                    )
                }
            },
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| CaptureError::OutputDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|err| {
            CaptureError::Settings(format!("could not serialise settings: {err}"))
        })?;
        // Write then rename so a crash cannot leave a half-written file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|source| CaptureError::Save {
            path: tmp.clone(),
            source,
        })?;
        std::fs::rename(&tmp, path).map_err(|source| CaptureError::Save {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }

    /// Where settings live for a given config dir.
    pub fn default_path(config_dir: &Path) -> PathBuf {
        config_dir.join("settings.json")
    }
}

/// `$XDG_PICTURES_DIR`, else `~/Pictures`, then `Screenshots/` inside it.
pub fn default_output_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let pictures = std::env::var_os("XDG_PICTURES_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            let home = home.as_ref()?;
            let dirs_file = home.join(".config/user-dirs.dirs");
            let content = std::fs::read_to_string(dirs_file).ok()?;
            pictures_dir_from_user_dirs(&content, home)
        })
        .or_else(|| home.as_ref().map(|h| h.join("Pictures")))
        .unwrap_or_else(|| PathBuf::from("."));
    pictures.join("Screenshots")
}

/// Parse `XDG_PICTURES_DIR="$HOME/Pictures"` out of `user-dirs.dirs`.
pub fn pictures_dir_from_user_dirs(content: &str, home: &Path) -> Option<PathBuf> {
    for line in content.lines() {
        let line = line.trim();
        if !line.starts_with("XDG_PICTURES_DIR=") {
            continue;
        }
        let raw = line.trim_start_matches("XDG_PICTURES_DIR=").trim();
        let raw = raw.trim_matches('"').trim_matches('\'');
        if raw.is_empty() {
            return None;
        }
        let expanded = if let Some(rest) = raw.strip_prefix("$HOME") {
            home.join(rest.trim_start_matches('/'))
        } else if raw.starts_with('/') {
            PathBuf::from(raw)
        } else {
            return None;
        };
        return Some(expanded);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_through_json() {
        let mut s = Settings::default();
        s.mode = CaptureMode::PinnedWindow;
        s.region = Region::new(10, 20, 300, 200);
        s.window = Some(WindowTarget {
            window_id: 0xdead_beef,
            title: "Editor".into(),
            class: "code".into(),
            pid: Some(42),
        });
        s.validate().unwrap();
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn defaults_are_valid_after_region_selection() {
        let mut s = Settings::default();
        assert!(s.validate().is_err(), "region mode needs a region");
        s.region = Region::new(0, 0, 100, 100);
        s.validate().unwrap();
    }

    #[test]
    fn pinned_mode_requires_window() {
        let mut s = Settings::default();
        s.mode = CaptureMode::PinnedWindow;
        assert!(s.validate().is_err());
        s.window = Some(WindowTarget {
            window_id: 1,
            title: "t".into(),
            class: "c".into(),
            pid: None,
        });
        s.validate().unwrap();
    }

    #[test]
    fn invalid_quality_and_pattern_are_rejected() {
        let mut s = Settings::default();
        s.region = Region::new(0, 0, 50, 50);
        s.jpeg_quality = 0;
        assert!(s.validate().is_err());
        s.jpeg_quality = 101;
        assert!(s.validate().is_err());
        s.jpeg_quality = 80;
        s.filename_pattern = "   ".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn capture_validation_reports_paused() {
        let mut s = Settings::default();
        s.region = Region::new(0, 0, 50, 50);
        s.paused = true;
        assert!(matches!(
            s.validate_for_capture(),
            Err(CaptureError::Paused)
        ));
    }

    #[test]
    fn corrupt_file_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        let (settings, warning) = Settings::load(&path);
        assert_eq!(settings, Settings::default());
        assert!(warning.is_some());
        assert!(dir.path().join("settings.json.bak").exists());
    }

    #[test]
    fn missing_file_uses_defaults_without_warning() {
        let dir = tempfile::tempdir().unwrap();
        let (settings, warning) = Settings::load(&dir.path().join("nope.json"));
        assert_eq!(settings, Settings::default());
        assert!(warning.is_none());
    }

    #[test]
    fn save_then_load_keeps_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        let mut s = Settings::default();
        s.region = Region::new(1, 2, 30, 40);
        s.format = ImageFormat::Jpeg;
        s.save(&path).unwrap();
        let (loaded, warning) = Settings::load(&path);
        assert!(warning.is_none());
        assert_eq!(loaded, s);
    }

    #[test]
    fn parses_user_dirs() {
        let home = Path::new("/home/aman");
        let content = "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_PICTURES_DIR=\"$HOME/Bilder\"\n";
        assert_eq!(
            pictures_dir_from_user_dirs(content, home),
            Some(PathBuf::from("/home/aman/Bilder"))
        );
        assert_eq!(pictures_dir_from_user_dirs("", home), None);
    }

    #[test]
    fn pattern_warning_for_static_names() {
        let mut s = Settings::default();
        assert!(s.filename_pattern_warning().is_none());
        s.filename_pattern = "shot".into();
        assert!(s.filename_pattern_warning().is_some());
    }
}
