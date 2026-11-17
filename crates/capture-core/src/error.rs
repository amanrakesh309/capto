//! Errors that the UI can present to the user.
//!
//! Every variant carries enough context to say *what failed* and *what to do
//! next*. The Tauri layer only prints [`CaptureError::user_message`] and
//! [`CaptureError::hint`], it never invents its own wording, so the messages can
//! be unit tested here.

use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, CaptureError>;

#[derive(Debug, Error)]
pub enum CaptureError {
    /// The backend exists but the requested operation is not implemented for
    /// this display server / desktop environment.
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// No usable display connection (for example `DISPLAY` is unset, or the
    /// session is a Wayland session without XWayland).
    #[error("no usable display server: {0}")]
    NoDisplay(String),

    /// The desktop refused the capture (portal veto, sandbox, access control).
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// The pinned window is gone, minimised, or its id now belongs to a
    /// different window.
    #[error("target unavailable: {detail}")]
    TargetUnavailable { detail: String },

    /// A remembered region is missing or outside the current screen layout.
    #[error("invalid region: {0}")]
    InvalidRegion(String),

    /// The user aborted an interactive selection.
    #[error("capture cancelled")]
    Cancelled,

    /// Captures are paused from the tray menu or the settings window.
    #[error("capturing is paused")]
    Paused,

    #[error("invalid settings: {0}")]
    Settings(String),

    #[error("could not use output folder {path}: {source}")]
    OutputDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not save the screenshot to {path}: {source}")]
    Save {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not encode the screenshot: {0}")]
    Encode(String),

    /// Anything the backend itself reported.
    #[error("capture failed: {0}")]
    Backend(String),
}

impl CaptureError {
    /// Stable machine readable code, used by the frontend for styling.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "unsupported",
            Self::NoDisplay(_) => "no_display",
            Self::PermissionDenied(_) => "permission_denied",
            Self::TargetUnavailable { .. } => "target_unavailable",
            Self::InvalidRegion(_) => "invalid_region",
            Self::Cancelled => "cancelled",
            Self::Paused => "paused",
            Self::Settings(_) => "settings",
            Self::OutputDirectory { .. } => "output_directory",
            Self::Save { .. } => "save",
            Self::Encode(_) => "encode",
            Self::Backend(_) => "backend",
        }
    }

    /// Short, plain-language sentence shown in the app and in the toast.
    pub fn user_message(&self) -> String {
        match self {
            Self::Unsupported(what) => {
                format!("This system does not support {what} with the current capture backend.")
            }
            Self::NoDisplay(detail) => {
                format!("Cannot reach the desktop session: {detail}")
            }
            Self::PermissionDenied(detail) => {
                format!("The desktop did not allow the screenshot: {detail}")
            }
            Self::TargetUnavailable { detail } => {
                format!("The pinned window is not available: {detail}")
            }
            Self::InvalidRegion(detail) => format!("The saved region cannot be used: {detail}"),
            Self::Cancelled => "Screenshot cancelled.".to_string(),
            Self::Paused => "Capturing is paused. Resume it to take screenshots.".to_string(),
            Self::Settings(detail) => format!("That setting is not valid: {detail}"),
            Self::OutputDirectory { path, source } => format!(
                "Cannot write to the screenshot folder {}: {source}",
                path.display()
            ),
            Self::Save { path, source } => format!(
                "Could not save the screenshot to {}: {source}",
                path.display()
            ),
            Self::Encode(detail) => format!("Could not encode the screenshot: {detail}"),
            Self::Backend(detail) => format!("The screenshot could not be taken: {detail}"),
        }
    }

    /// Optional follow-up explaining how to recover.
    pub fn hint(&self) -> Option<String> {
        match self {
            Self::Unsupported(_) => {
                Some("Choose a capture mode this backend supports, or use the tray menu.".into())
            }
            Self::NoDisplay(_) => Some(
                "Continuous Screenshot currently captures X11 windows. Start it inside an X11 session (or with XWayland running)."
                    .into(),
            ),
            Self::PermissionDenied(_) => {
                Some("Allow screen capture in the desktop's permission prompt, then retry.".into())
            }
            Self::TargetUnavailable { .. } => {
                Some("Select the window again from the settings window or the tray menu.".into())
            }
            Self::InvalidRegion(_) => {
                Some("Draw the region again with \"Select region\".".into())
            }
            Self::Cancelled => None,
            Self::Paused => Some("Tray menu → Resume capturing.".into()),
            Self::Settings(_) => Some("Correct the value and save again.".into()),
            Self::OutputDirectory { .. } => {
                Some("Pick a folder you can write to, or create it first.".into())
            }
            Self::Save { .. } => Some("Check free space and folder permissions.".into()),
            Self::Encode(_) => Some("Try the other image format (PNG or JPEG).".into()),
            Self::Backend(_) => None,
        }
    }

    /// True when the user should simply try again (no configuration change).
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::TargetUnavailable { .. } | Self::Backend(_)
        )
    }
}
