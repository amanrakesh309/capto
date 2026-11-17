//! X11 capture backend.
//!
//! * Region capture uses `GetImage` on the root window.
//! * Pinned-window capture prefers `CompositeNameWindowPixmap`, which returns
//!   the window's own offscreen contents, so the window can be captured while
//!   another application is focused and covering it.
//! * Without a compositor, a covered window is reported as unavailable instead
//!   of saving a screenshot that actually contains the window on top.

pub mod conn;
pub mod select;
pub mod windows;

use crate::backend::{Capabilities, CaptureBackend, CaptureFrame, WindowInfo};
use crate::error::{CaptureError, Result};
use crate::geometry::Region;
use crate::image::RawImage;
use crate::settings::WindowTarget;
use conn::{VisualFormat, X11Connection};
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::composite::ConnectionExt as CompositeConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::protocol::ErrorKind;

/// Keep each `GetImage` reply well below the smallest possible maximum request
/// size, so large screens and huge windows work without BIG-REQUESTS.
const MAX_CHUNK_BYTES: usize = 1024 * 1024;

pub struct X11Backend {
    x: X11Connection,
}

impl X11Backend {
    pub fn connect() -> Result<Self> {
        Ok(Self {
            x: X11Connection::connect()?,
        })
    }

    fn capture_direct(&self, window: Window, geometry: &Region) -> Result<RawImage> {
        capture_rect(&self.x, window, geometry)
    }

    fn capture_composited(&self, window: Window) -> Result<RawImage> {
        // Compositors redirect toplevel windows (the window manager frame), so
        // the named pixmap must come from there and then be cropped down to the
        // application's client area.
        let frame = windows::toplevel(&self.x, window);
        let (offset_x, offset_y) =
            windows::offset_within(&self.x, window, frame).ok_or_else(|| {
                CaptureError::Backend("could not locate the window inside its frame".into())
            })?;
        let geometry = self
            .x
            .conn
            .get_geometry(window)
            .map_err(|err| CaptureError::Backend(format!("could not read the window size: {err}")))?
            .reply()
            .map_err(|err| CaptureError::TargetUnavailable {
                detail: format!("the window disappeared: {err}"),
            })?;

        let pixmap =
            self.x.conn.generate_id().map_err(|err| {
                CaptureError::Backend(format!("could not allocate a pixmap: {err}"))
            })?;
        let named = self
            .x
            .conn
            .composite_name_window_pixmap(frame, pixmap)
            .map_err(|err| {
                CaptureError::Backend(format!("CompositeNameWindowPixmap failed: {err}"))
            })?
            .check();
        if let Err(err) = named {
            return Err(CaptureError::Backend(format!(
                "the window has no offscreen buffer: {err}"
            )));
        }
        let result = capture_drawable(&self.x, pixmap).and_then(|image| {
            image
                .crop(Region::new(
                    offset_x,
                    offset_y,
                    geometry.width as u32,
                    geometry.height as u32,
                ))
                .ok_or_else(|| {
                    CaptureError::Backend(
                        "the compositor buffer did not contain the whole window".into(),
                    )
                })
        });
        let _ = self.x.conn.free_pixmap(pixmap);
        let _ = self.x.conn.flush();
        result
    }
}

impl CaptureBackend for X11Backend {
    fn backend_name(&self) -> &'static str {
        "X11 (XGetImage)"
    }

    fn display_server(&self) -> &'static str {
        "X11"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            region: true,
            pinned_window: true,
            foreground_window: self.x.active_window().is_some(),
            interactive_region_selection: true,
            interactive_window_selection: true,
            window_list: true,
            full_screen: true,
        }
    }

    fn notes(&self) -> Vec<String> {
        let mut notes = vec![format!(
            "Desktop is {}×{} on X11 screen {}.",
            self.x.bounds.width, self.x.bounds.height, self.x.screen_num
        )];
        if self.x.composited {
            notes.push(
                "A compositing manager is running: covered windows can be captured from their own buffer."
                    .into(),
            );
        } else {
            notes.push(
                "No compositing manager detected: a covered window cannot be captured and will be reported as unavailable."
                    .into(),
            );
        }
        let monitors = self.x.randr_monitors();
        if monitors.len() > 1 {
            notes.push(format!(
                "{} monitors detected; regions use one virtual desktop coordinate space.",
                monitors.len()
            ));
        }
        notes.push(
            "Window screenshots contain the application's client area, not the title bar or borders."
                .into(),
        );
        notes
    }

    fn capture_region(&mut self, region: Region) -> Result<CaptureFrame> {
        if !region.is_usable() {
            return Err(CaptureError::InvalidRegion(format!(
                "{} is smaller than {}×{} pixels",
                region,
                Region::MIN_SIZE,
                Region::MIN_SIZE
            )));
        }
        let clipped = region.clamp_to(&self.x.bounds).ok_or_else(|| {
            CaptureError::InvalidRegion(format!(
                "{region} is outside the {}×{} desktop",
                self.x.bounds.width, self.x.bounds.height
            ))
        })?;
        let image = capture_rect(&self.x, self.x.root, &clipped)?;
        let warning = if clipped != region {
            Some(format!(
                "The saved region was cropped to the desktop ({}) because the screen layout changed.",
                clipped
            ))
        } else {
            None
        };
        Ok(CaptureFrame { image, warning })
    }

    fn capture_window(&mut self, target: &WindowTarget) -> Result<CaptureFrame> {
        let state = windows::validate(&self.x, target)?;
        let on_screen = state.geometry.clamp_to(&self.x.bounds).ok_or_else(|| {
            CaptureError::TargetUnavailable {
                detail: "the window is completely outside the desktop".into(),
            }
        })?;
        let partially_offscreen = on_screen != state.geometry;

        if self.x.composited && self.x.has_composite {
            match self.capture_composited(target.window_id) {
                Ok(image) => {
                    return Ok(CaptureFrame {
                        image,
                        warning: if partially_offscreen {
                            Some("Part of the pinned window is off screen.".into())
                        } else {
                            None
                        },
                    })
                }
                Err(err) => {
                    log::warn!("composited window capture failed, falling back: {err}");
                }
            }
        }

        // Direct readback only shows the window's own pixels when nothing is
        // covering it, so refuse instead of silently capturing another window.
        let overlapping = windows::overlapping_windows(&self.x, target.window_id, &state.geometry);
        if let Some(blocker) = overlapping.first() {
            return Err(CaptureError::TargetUnavailable {
                detail: format!(
                    "it is covered by \"{blocker}\" and this desktop cannot capture covered windows"
                ),
            });
        }
        let image = self.capture_direct(target.window_id, &on_screen)?;
        Ok(CaptureFrame {
            image,
            warning: if partially_offscreen {
                Some("Part of the pinned window is off screen.".into())
            } else {
                None
            },
        })
    }

    fn capture_screen(&mut self) -> Result<CaptureFrame> {
        let image = capture_drawable(&self.x, self.x.root)?;
        let monitors = self.x.randr_monitors().len();
        Ok(CaptureFrame {
            image,
            warning: if monitors > 1 {
                Some(format!("Captured all {monitors} monitors as one image."))
            } else {
                None
            },
        })
    }

    fn select_region(&mut self) -> Result<Option<Region>> {
        select::select_region(&self.x)
    }

    fn select_window(&mut self) -> Result<Option<WindowTarget>> {
        select::select_window(&self.x)
    }

    fn list_windows(&mut self) -> Result<Vec<WindowInfo>> {
        windows::list(&self.x)
    }

    fn active_window(&mut self) -> Result<Option<WindowTarget>> {
        Ok(windows::active_target(&self.x))
    }

    fn check_target(&mut self, target: &WindowTarget) -> Option<String> {
        windows::validate(&self.x, target)
            .err()
            .map(|err| err.user_message())
    }

    fn screen_bounds(&mut self) -> Result<Region> {
        Ok(self.x.bounds)
    }
}

/// Capture the whole root window (used as the backdrop of the region overlay).
pub fn capture_root(conn: &X11Connection) -> Result<RawImage> {
    capture_drawable(conn, conn.root)
}

fn capture_drawable(conn: &X11Connection, drawable: Drawable) -> Result<RawImage> {
    let geometry = conn
        .conn
        .get_geometry(drawable)
        .map_err(|err| CaptureError::Backend(format!("could not read the drawable size: {err}")))?
        .reply()
        .map_err(|err| CaptureError::TargetUnavailable {
            detail: format!("the drawable disappeared: {err}"),
        })?;
    let width = geometry.width as u32;
    let height = geometry.height as u32;
    if width == 0 || height == 0 {
        return Err(CaptureError::TargetUnavailable {
            detail: "the window has no visible area".into(),
        });
    }
    capture_rect(conn, drawable, &Region::new(0, 0, width, height))
}

fn capture_rect(conn: &X11Connection, drawable: Drawable, region: &Region) -> Result<RawImage> {
    if region.width == 0 || region.height == 0 {
        return Err(CaptureError::TargetUnavailable {
            detail: "the captured area is empty".into(),
        });
    }
    let width = region.width.min(u16::MAX as u32);
    let height = region.height.min(u16::MAX as u32);
    // Assume 4 bytes per pixel while sizing chunks; that is the worst case and
    // keeps every reply under the limit even for 32-bit visuals.
    let rows_per_chunk = (MAX_CHUNK_BYTES / (width as usize * 4).max(1)).max(1);
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    let mut row = 0u32;
    while row < height {
        let rows = (rows_per_chunk as u32).min(height - row);
        let reply = conn
            .conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                drawable,
                region.x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                (region.y + row as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                width as u16,
                rows as u16,
                !0,
            )
            .map_err(|err| CaptureError::Backend(format!("GetImage failed: {err}")))?
            .reply()
            .map_err(|err| match err {
                ReplyError::X11Error(error) if error.error_kind == ErrorKind::Match => {
                    CaptureError::TargetUnavailable {
                        detail: "the window is not visible (it may be minimized or covered)".into(),
                    }
                }
                ReplyError::X11Error(error) if error.error_kind == ErrorKind::Access => {
                    CaptureError::PermissionDenied(
                        "the X server refused access to that window".into(),
                    )
                }
                other => CaptureError::Backend(format!("GetImage failed: {other}")),
            })?;
        let format = format_for_reply(conn, reply.visual, reply.depth);
        let stride = if rows == 0 {
            0
        } else {
            reply.data.len() / rows as usize
        };
        let chunk = format.decode_rows(&reply.data, width, rows, stride)?;
        rgba.extend_from_slice(&chunk);
        row += rows;
    }
    RawImage::new(width, height, rgba)
        .ok_or_else(|| CaptureError::Backend("the captured buffer had an unexpected size".into()))
}

fn format_for_reply(conn: &X11Connection, visual: Visualid, depth: u8) -> VisualFormat {
    VisualFormat::for_visual(conn.conn.setup(), visual)
        .or_else(|| VisualFormat::for_depth(conn.conn.setup(), depth))
        .unwrap_or(conn.root_format)
}
