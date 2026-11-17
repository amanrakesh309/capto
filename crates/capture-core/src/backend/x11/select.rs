//! Interactive selection: a dimmed full-screen overlay with a rubber band for
//! regions, and a crosshair click-to-pick for windows.
//!
//! Both are implemented with plain X11 primitives (no GTK dialog in the capture
//! path), so they behave the same on every X11 desktop, including tiling window
//! managers and multi-monitor layouts.

use super::conn::X11Connection;
use super::windows;
use crate::error::{CaptureError, Result};
use crate::geometry::Region;
use crate::settings::WindowTarget;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

/// How much the desktop is darkened behind the rubber band.
const DIM_FACTOR: f32 = 0.45;
/// Extra pixels restored around the previous rubber band to erase its border.
const BORDER_PAD: i32 = 4;
const BORDER_WIDTH: u16 = 2;

fn escape_keycode(conn: &RustConnection) -> Option<u8> {
    let setup = conn.setup();
    let min = setup.min_keycode;
    let count = setup.max_keycode.saturating_sub(min).saturating_add(1);
    let reply = conn.get_keyboard_mapping(min, count).ok()?.reply().ok()?;
    let per = reply.keysyms_per_keycode as usize;
    if per == 0 {
        return None;
    }
    for (index, keysyms) in reply.keysyms.chunks(per).enumerate() {
        if keysyms.iter().any(|keysym| *keysym == 0xff1b) {
            return Some(min.saturating_add(index as u8));
        }
    }
    None
}

fn make_crosshair_cursor(conn: &X11Connection) -> Result<Cursor> {
    let font = conn
        .conn
        .generate_id()
        .map_err(|err| CaptureError::Backend(err.to_string()))?;
    conn.conn
        .open_font(font, b"cursor")
        .map_err(|err| CaptureError::Backend(err.to_string()))?
        .check()
        .map_err(|err| CaptureError::Backend(format!("could not open the cursor font: {err}")))?;
    let cursor = conn
        .conn
        .generate_id()
        .map_err(|err| CaptureError::Backend(err.to_string()))?;
    // XC_crosshair (34) and its mask (35).
    conn.conn
        .create_glyph_cursor(
            cursor, font, font, 34, 35, 0xFFFF, 0xFFFF, 0xFFFF, 0x0000, 0x0000, 0x0000,
        )
        .map_err(|err| CaptureError::Backend(err.to_string()))?;
    let _ = conn.conn.close_font(font);
    Ok(cursor)
}

/// Show the region overlay and return the rectangle the user drew.
///
/// `Ok(None)` means "cancelled" (Escape, right click, or a too-small drag).
pub fn select_region(conn: &X11Connection) -> Result<Option<Region>> {
    let screen = super::capture_root(conn)?;
    let cursor = make_crosshair_cursor(conn)?;
    let mut overlay = Overlay::new(conn, &screen, cursor)?;
    overlay.run()
}

struct Overlay<'a> {
    conn: &'a X11Connection,
    window: Window,
    gc: Gcontext,
    original: Pixmap,
    dimmed: Pixmap,
    cursor: Cursor,
    pointer_grabbed: bool,
    keyboard_grabbed: bool,
    escape: Option<u8>,
}

impl<'a> Overlay<'a> {
    fn new(
        conn: &'a X11Connection,
        screen: &crate::image::RawImage,
        cursor: Cursor,
    ) -> Result<Self> {
        let width = screen.width.min(u16::MAX as u32);
        let height = screen.height.min(u16::MAX as u32);
        if width == 0 || height == 0 {
            let _ = conn.conn.free_cursor(cursor);
            return Err(CaptureError::Backend("the screen has no area".into()));
        }
        let generate = |what: &str| {
            conn.conn
                .generate_id()
                .map_err(|err| CaptureError::Backend(format!("could not allocate {what}: {err}")))
        };

        let original = generate("a pixmap")?;
        conn.conn
            .create_pixmap(
                conn.root_depth,
                original,
                conn.root,
                width as u16,
                height as u16,
            )
            .map_err(|err| CaptureError::Backend(format!("could not create a pixmap: {err}")))?;
        let gc = generate("a graphics context")?;
        conn.conn
            .create_gc(gc, original, &CreateGCAux::new())
            .map_err(|err| CaptureError::Backend(format!("could not create a GC: {err}")))?;
        let encoded = conn.root_format.encode(&screen.rgba, width, height);
        conn.conn
            .put_image(
                ImageFormat::Z_PIXMAP,
                original,
                gc,
                width as u16,
                height as u16,
                0,
                0,
                0,
                conn.root_depth,
                &encoded,
            )
            .map_err(|err| {
                CaptureError::Backend(format!("could not upload the screenshot: {err}"))
            })?;

        let dimmed = generate("a pixmap")?;
        conn.conn
            .create_pixmap(
                conn.root_depth,
                dimmed,
                conn.root,
                width as u16,
                height as u16,
            )
            .map_err(|err| CaptureError::Backend(format!("could not create a pixmap: {err}")))?;
        let mut dim_rgba = screen.rgba.clone();
        super::conn::VisualFormat::dim(&mut dim_rgba, DIM_FACTOR);
        let dim_encoded = conn.root_format.encode(&dim_rgba, width, height);
        conn.conn
            .put_image(
                ImageFormat::Z_PIXMAP,
                dimmed,
                gc,
                width as u16,
                height as u16,
                0,
                0,
                0,
                conn.root_depth,
                &dim_encoded,
            )
            .map_err(|err| {
                CaptureError::Backend(format!("could not upload the backdrop: {err}"))
            })?;

        let window = generate("a window")?;
        conn.conn
            .create_window(
                conn.root_depth,
                window,
                conn.root,
                0,
                0,
                width as u16,
                height as u16,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new()
                    .background_pixmap(dimmed)
                    .override_redirect(1)
                    .cursor(cursor)
                    .event_mask(
                        EventMask::EXPOSURE
                            | EventMask::BUTTON_PRESS
                            | EventMask::BUTTON_RELEASE
                            | EventMask::POINTER_MOTION
                            | EventMask::KEY_PRESS,
                    ),
            )
            .map_err(|err| CaptureError::Backend(format!("could not create the overlay: {err}")))?;

        // Grab input before mapping so the click that starts the selection is
        // not delivered to the window underneath.
        let pointer_grabbed = conn
            .conn
            .grab_pointer(
                false,
                conn.root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                x11rb::CURRENT_TIME,
            )
            .map(|cookie| {
                cookie
                    .reply()
                    .map(|reply| reply.status == GrabStatus::SUCCESS)
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        let keyboard_grabbed = conn
            .conn
            .grab_keyboard(
                false,
                conn.root,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .map(|cookie| {
                cookie
                    .reply()
                    .map(|reply| reply.status == GrabStatus::SUCCESS)
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        let mut overlay = Self {
            conn,
            window,
            gc,
            original,
            dimmed,
            cursor,
            pointer_grabbed,
            keyboard_grabbed,
            escape: escape_keycode(&conn.conn),
        };
        if !overlay.pointer_grabbed {
            overlay.cleanup();
            return Err(CaptureError::Backend(
                "another application is holding the pointer; close menus or drag operations and try again"
                    .into(),
            ));
        }
        conn.conn
            .map_window(window)
            .map_err(|err| CaptureError::Backend(format!("could not show the overlay: {err}")))?;
        conn.conn
            .flush()
            .map_err(|err| CaptureError::Backend(err.to_string()))?;
        Ok(overlay)
    }

    fn restore_dim(&self, region: &Region) {
        let padded = Region::new(
            region.x - BORDER_PAD,
            region.y - BORDER_PAD,
            region.width + (BORDER_PAD as u32) * 2,
            region.height + (BORDER_PAD as u32) * 2,
        );
        let Some(clipped) = padded.clamp_to(&self.conn.bounds) else {
            return;
        };
        let _ = self.conn.conn.copy_area(
            self.dimmed,
            self.window,
            self.gc,
            clipped.x as i16,
            clipped.y as i16,
            clipped.x as i16,
            clipped.y as i16,
            clipped.width.min(u16::MAX as u32) as u16,
            clipped.height.min(u16::MAX as u32) as u16,
        );
    }

    fn reveal(&self, region: &Region) {
        let Some(clipped) = region.clamp_to(&self.conn.bounds) else {
            return;
        };
        if clipped.is_empty() {
            return;
        }
        let _ = self.conn.conn.copy_area(
            self.original,
            self.window,
            self.gc,
            clipped.x as i16,
            clipped.y as i16,
            clipped.x as i16,
            clipped.y as i16,
            clipped.width.min(u16::MAX as u32) as u16,
            clipped.height.min(u16::MAX as u32) as u16,
        );
    }

    fn draw_border(&self, region: &Region) {
        let (x, y, w, h) = region.to_x11_rect();
        if w < 2 || h < 2 {
            return;
        }
        // Dark outline first, then a bright inner one so the selection stays
        // visible on both light and dark content.
        let _ = self.conn.conn.change_gc(
            self.gc,
            &ChangeGCAux::new()
                .foreground(0x000000)
                .line_width(BORDER_WIDTH as u32),
        );
        let _ = self.conn.conn.poly_rectangle(
            self.window,
            self.gc,
            &[Rectangle {
                x,
                y,
                width: w.saturating_sub(1),
                height: h.saturating_sub(1),
            }],
        );
        let _ = self.conn.conn.change_gc(
            self.gc,
            &ChangeGCAux::new().foreground(0xFFFFFF).line_width(1),
        );
        let _ = self.conn.conn.poly_rectangle(
            self.window,
            self.gc,
            &[Rectangle {
                x,
                y,
                width: w.saturating_sub(1),
                height: h.saturating_sub(1),
            }],
        );
    }

    fn run(&mut self) -> Result<Option<Region>> {
        let bounds = self.conn.bounds;
        let mut start: Option<(i32, i32)> = None;
        let mut current = Region::empty();
        let mut previous = Region::empty();

        loop {
            let event =
                self.conn.conn.wait_for_event().map_err(|err| {
                    CaptureError::Backend(format!("lost the X11 connection: {err}"))
                })?;
            match event {
                Event::Expose(_) => {
                    let _ = self.conn.conn.clear_area(false, self.window, 0, 0, 0, 0);
                    let _ = self.conn.conn.flush();
                }
                Event::ButtonPress(event) if event.detail == 1 => {
                    let point = (event.root_x as i32, event.root_y as i32);
                    start = Some(point);
                    previous = Region::empty();
                    current = Region::new(point.0, point.1, 0, 0);
                }
                Event::MotionNotify(event) => {
                    if let Some(start) = start {
                        let point = (event.root_x as i32, event.root_y as i32);
                        let next = Region::from_corners(start.0, start.1, point.0, point.1);
                        if next != current {
                            if !previous.is_empty() {
                                self.restore_dim(&previous);
                            }
                            current = next;
                            current = current.clamp_to(&bounds).unwrap_or(Region::empty());
                            if !current.is_empty() {
                                self.reveal(&current);
                                self.draw_border(&current);
                            }
                            previous = current;
                            let _ = self.conn.conn.flush();
                        }
                    }
                }
                Event::ButtonRelease(event) if event.detail == 1 => {
                    if let Some(start) = start.take() {
                        let point = (event.root_x as i32, event.root_y as i32);
                        let region = Region::from_corners(start.0, start.1, point.0, point.1)
                            .clamp_to(&bounds)
                            .unwrap_or(Region::empty());
                        return if region.is_usable() {
                            Ok(Some(region))
                        } else {
                            Ok(None)
                        };
                    }
                }
                Event::ButtonPress(_) => {
                    // Any other button cancels, matching most screenshot tools.
                    return Ok(None);
                }
                Event::KeyPress(event) => {
                    if Some(event.detail) == self.escape {
                        return Ok(None);
                    }
                }
                Event::Error(err) => {
                    return Err(CaptureError::Backend(format!(
                        "the overlay lost its connection to X11: {err:?}"
                    )))
                }
                _ => {}
            }
        }
    }

    fn cleanup(&mut self) {
        if self.pointer_grabbed {
            let _ = self.conn.conn.ungrab_pointer(x11rb::CURRENT_TIME);
            self.pointer_grabbed = false;
        }
        if self.keyboard_grabbed {
            let _ = self.conn.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
            self.keyboard_grabbed = false;
        }
        let _ = self.conn.conn.unmap_window(self.window);
        let _ = self.conn.conn.destroy_window(self.window);
        let _ = self.conn.conn.free_pixmap(self.original);
        let _ = self.conn.conn.free_pixmap(self.dimmed);
        let _ = self.conn.conn.free_gc(self.gc);
        let _ = self.conn.conn.free_cursor(self.cursor);
        let _ = self.conn.conn.flush();
    }
}

impl Drop for Overlay<'_> {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Pick a window by clicking it. `Ok(None)` means cancelled.
pub fn select_window(conn: &X11Connection) -> Result<Option<WindowTarget>> {
    let cursor = make_crosshair_cursor(conn)?;
    let grab = conn.conn.grab_pointer(
        false,
        conn.root,
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
        x11rb::NONE,
        cursor,
        x11rb::CURRENT_TIME,
    );
    let grabbed = match grab {
        Ok(cookie) => cookie
            .reply()
            .map(|reply| reply.status == GrabStatus::SUCCESS)
            .unwrap_or(false),
        Err(_) => false,
    };
    if !grabbed {
        let _ = conn.conn.free_cursor(cursor);
        return Err(CaptureError::Backend(
            "another application is holding the pointer; close menus or drag operations and try again"
                .into(),
        ));
    }
    let keyboard = conn
        .conn
        .grab_keyboard(
            false,
            conn.root,
            x11rb::CURRENT_TIME,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )
        .map(|cookie| {
            cookie
                .reply()
                .map(|reply| reply.status == GrabStatus::SUCCESS)
                .unwrap_or(false)
        })
        .unwrap_or(false);
    let escape = escape_keycode(&conn.conn);

    let mut point = None;
    let mut cancelled = false;
    loop {
        let event = match conn.conn.wait_for_event() {
            Ok(event) => event,
            Err(err) => {
                let _ = conn.conn.free_cursor(cursor);
                if keyboard {
                    let _ = conn.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
                }
                return Err(CaptureError::Backend(format!(
                    "lost the X11 connection while picking a window: {err}"
                )));
            }
        };
        match event {
            Event::ButtonPress(event) if event.detail == 1 => {
                point = Some((event.root_x as i32, event.root_y as i32));
                break;
            }
            Event::ButtonPress(_) | Event::ButtonRelease(_) => {
                cancelled = true;
                break;
            }
            Event::KeyPress(event) => {
                if Some(event.detail) == escape {
                    cancelled = true;
                    break;
                }
            }
            _ => {}
        }
    }

    let _ = conn.conn.ungrab_pointer(x11rb::CURRENT_TIME);
    if keyboard {
        let _ = conn.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
    }
    let _ = conn.conn.free_cursor(cursor);
    let _ = conn.conn.flush();

    if cancelled {
        return Ok(None);
    }
    let Some((x, y)) = point else {
        return Ok(None);
    };
    match windows::window_at_point(conn, x, y) {
        Some(window) => Ok(Some(windows::target_for(conn, window)?)),
        None => Err(CaptureError::TargetUnavailable {
            detail: "nothing selectable was found under the pointer".into(),
        }),
    }
}
