//! Capture core for **Continuous Screenshot**.
//!
//! This crate contains everything that is independent of the desktop shell:
//!
//! * [`backend`] - platform capture backends behind a single trait
//! * [`service`] - mode/target resolution, capture and save workflow
//! * [`settings`] - persisted preferences with validation
//! * [`storage`] - PNG/JPEG encoding and collision-free filenames
//! * [`geometry`] - rectangles and screen clamping
//! * [`naming`] - timestamped filename patterns
//!
//! The Tauri app in `src-tauri` is a thin shell over [`service::CaptureService`].

pub mod backend;
pub mod error;
pub mod geometry;
pub mod image;
pub mod naming;
pub mod service;
pub mod settings;
pub mod storage;

pub use backend::{Capabilities, CaptureBackend, CaptureFrame, WindowInfo};
pub use error::{CaptureError, Result};
pub use geometry::Region;
pub use image::RawImage;
pub use service::{CaptureOutcome, CaptureService, LastCapture, ServiceStatus};
pub use settings::{CaptureMode, Settings, WindowTarget};
pub use storage::{ImageFormat, SavedImage};
