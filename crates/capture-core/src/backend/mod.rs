//! Platform capture backends.
//!
//! The trait deliberately exposes *only* what the product needs, and every
//! capability is reported up front so the UI can disable controls instead of
//! failing at capture time. Backends must return [`CaptureError::Unsupported`]
//! rather than silently falling back to a different target.

pub mod x11;

use crate::error::{CaptureError, Result};
use crate::geometry::Region;
use crate::image::RawImage;
use crate::settings::WindowTarget;
use serde::Serialize;

/// What a backend can actually do on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    /// Re-capture a fixed rectangle.
    pub region: bool,
    /// Capture a window that was selected earlier, regardless of focus.
    pub pinned_window: bool,
    /// Capture whichever window is focused at press time.
    pub foreground_window: bool,
    /// Show an overlay to draw a rectangle.
    pub interactive_region_selection: bool,
    /// Show a click-to-pick cursor for a window.
    pub interactive_window_selection: bool,
    /// Enumerate candidate windows for a list picker.
    pub window_list: bool,
    /// Whole-desktop capture (deferred in the MVP, kept for completeness).
    pub full_screen: bool,
}

impl Capabilities {
    pub const NONE: Self = Self {
        region: false,
        pinned_window: false,
        foreground_window: false,
        interactive_region_selection: false,
        interactive_window_selection: false,
        window_list: false,
        full_screen: false,
    };

    pub fn supports(&self, mode: crate::settings::CaptureMode) -> bool {
        match mode {
            crate::settings::CaptureMode::Region => self.region,
            crate::settings::CaptureMode::SelectArea => self.interactive_region_selection && self.region,
            crate::settings::CaptureMode::PinnedWindow => self.pinned_window,
            crate::settings::CaptureMode::ForegroundWindow => self.foreground_window,
            crate::settings::CaptureMode::FullScreen => self.full_screen,
        }
    }
}

/// A window that can be pinned, as shown in the settings window.
#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    pub target: WindowTarget,
    pub geometry: Region,
    pub active: bool,
    pub minimized: bool,
}

/// One captured image plus any caveat the user should know about.
#[derive(Debug, Clone)]
pub struct CaptureFrame {
    pub image: RawImage,
    /// Non-fatal note, e.g. "the window is partly covered".
    pub warning: Option<String>,
}

impl CaptureFrame {
    pub fn new(image: RawImage) -> Self {
        Self {
            image,
            warning: None,
        }
    }
}

pub trait CaptureBackend: Send {
    /// Human readable backend name, e.g. "X11 (XGetImage)".
    fn backend_name(&self) -> &'static str;
    /// "X11" or "Wayland" - displayed in the window so the user knows what is
    /// being used.
    fn display_server(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    /// Short compatibility notes shown in the settings window.
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }

    fn capture_region(&mut self, region: Region) -> Result<CaptureFrame>;
    fn capture_window(&mut self, target: &WindowTarget) -> Result<CaptureFrame>;

    fn capture_screen(&mut self) -> Result<CaptureFrame> {
        Err(CaptureError::Unsupported(
            "full-screen capture is not implemented by this backend".into(),
        ))
    }

    /// Interactive rectangle selection. `Ok(None)` means the user cancelled.
    fn select_region(&mut self) -> Result<Option<Region>> {
        Err(CaptureError::Unsupported(
            "region selection is not implemented by this backend".into(),
        ))
    }

    /// Interactive window picking. `Ok(None)` means the user cancelled.
    fn select_window(&mut self) -> Result<Option<WindowTarget>> {
        Err(CaptureError::Unsupported(
            "window selection is not implemented by this backend".into(),
        ))
    }

    fn list_windows(&mut self) -> Result<Vec<WindowInfo>> {
        Err(CaptureError::Unsupported(
            "listing windows is not implemented by this backend".into(),
        ))
    }

    fn active_window(&mut self) -> Result<Option<WindowTarget>> {
        Ok(None)
    }

    /// Check a saved target without capturing. `Some(problem)` means the target
    /// is unusable and the user must select a window again.
    fn check_target(&mut self, _target: &WindowTarget) -> Option<String> {
        None
    }

    /// Current desktop bounds, used to validate a remembered region.
    fn screen_bounds(&mut self) -> Result<Region> {
        Err(CaptureError::Unsupported(
            "querying the screen size is not implemented by this backend".into(),
        ))
    }
}

/// Pick the backend for the current session.
///
/// Only X11 is implemented in this release. A Wayland session without XWayland
/// yields a clear [`CaptureError::NoDisplay`] instead of a broken feature.
pub fn default_backend() -> Result<Box<dyn CaptureBackend>> {
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let on_x11 = std::env::var_os("DISPLAY").is_some();
    if session.eq_ignore_ascii_case("wayland") && !on_x11 {
        return Err(CaptureError::Unsupported(
            "capturing on a Wayland session without XWayland".into(),
        ));
    }
    Ok(Box::new(x11::X11Backend::connect()?))
}
