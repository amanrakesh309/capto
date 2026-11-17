//! Window discovery, identity checks and hit testing.
//!
//! Pinned-window capture is only trustworthy if we can tell "the same window"
//! from "a window that happens to reuse the same id". X11 window ids are
//! recycled, so we also remember the title, WM_CLASS and `_NET_WM_PID` and
//! refuse to capture when they no longer match.

use super::conn::X11Connection;
use crate::backend::WindowInfo;
use crate::error::{CaptureError, Result};
use crate::geometry::Region;
use crate::settings::WindowTarget;
use x11rb::protocol::xproto::*;

#[derive(Debug, Clone)]
pub struct WindowState {
    pub geometry: Region,
    pub viewable: bool,
    pub minimized: bool,
    pub title: String,
    pub class: String,
    pub pid: Option<u32>,
}

pub fn state(conn: &X11Connection, window: Window) -> Result<WindowState> {
    let geometry = conn
        .window_geometry(window)
        .ok_or_else(|| CaptureError::TargetUnavailable {
            detail: "the window no longer exists".into(),
        })?;
    let viewable = conn.is_viewable(window)?;
    Ok(WindowState {
        geometry,
        viewable,
        minimized: conn.is_minimized(window),
        title: conn.window_title(window),
        class: conn.window_class(window),
        pid: conn.window_pid(window),
    })
}

/// Build the persisted target description for a window id.
pub fn target_for(conn: &X11Connection, window: Window) -> Result<WindowTarget> {
    let state = state(conn, window)?;
    if !state.viewable {
        return Err(CaptureError::TargetUnavailable {
            detail: "that window is not visible".into(),
        });
    }
    Ok(WindowTarget {
        window_id: window,
        title: state.title,
        class: state.class,
        pid: state.pid,
    })
}

/// Check a saved target immediately before capturing it.
///
/// A mismatch means the id was recycled or the app was restarted; the caller
/// turns this into "select the window again" instead of grabbing the wrong app.
pub fn validate(conn: &X11Connection, target: &WindowTarget) -> Result<WindowState> {
    let state = state(conn, target.window_id)?;
    if !state.viewable || state.minimized {
        return Err(CaptureError::TargetUnavailable {
            detail: format!("{} is minimized or hidden", describe(target)),
        });
    }
    if !target.class.is_empty()
        && !state.class.is_empty()
        && !target.class.eq_ignore_ascii_case(&state.class)
    {
        return Err(CaptureError::TargetUnavailable {
            detail: format!(
                "window id {:#x} now belongs to \"{}\" instead of \"{}\"",
                target.window_id, state.class, target.class
            ),
        });
    }
    if let (Some(saved), Some(current)) = (target.pid, state.pid) {
        if saved != current {
            return Err(CaptureError::TargetUnavailable {
                detail: format!(
                    "window id {:#x} now belongs to process {} instead of process {}",
                    target.window_id, current, saved
                ),
            });
        }
    }
    if state.geometry.is_empty() {
        return Err(CaptureError::TargetUnavailable {
            detail: "the window has no visible area".into(),
        });
    }
    Ok(state)
}

fn describe(target: &WindowTarget) -> String {
    if target.title.trim().is_empty() {
        format!("window {:#x}", target.window_id)
    } else {
        format!("\"{}\"", target.label())
    }
}

/// Windows offered in the settings window, topmost first.
pub fn list(conn: &X11Connection) -> Result<Vec<WindowInfo>> {
    let active = conn.active_window();
    let mut infos = Vec::new();
    for window in conn.client_windows().into_iter().rev() {
        if infos.len() >= 200 {
            break;
        }
        let Ok(state) = state(conn, window) else {
            continue;
        };
        if !state.viewable || state.geometry.is_empty() {
            continue;
        }
        if is_dock_or_desktop(conn, window) {
            continue;
        }
        infos.push(WindowInfo {
            geometry: state.geometry,
            active: active == Some(window),
            minimized: state.minimized,
            target: WindowTarget {
                window_id: window,
                title: state.title,
                class: state.class,
                pid: state.pid,
            },
        });
    }
    Ok(infos)
}

fn is_dock_or_desktop(conn: &X11Connection, window: Window) -> bool {
    let types = conn.property_u32(window, conn.atoms.net_wm_window_type);
    types.contains(&conn.atoms.net_wm_window_type_dock)
        || types.contains(&conn.atoms.net_wm_window_type_desktop)
}

/// Find the client window under a root-coordinate point.
///
/// Primary strategy: hit-test EWMH clients, expanding each client rectangle by
/// its `_NET_FRAME_EXTENTS` so title bars and borders also select the window.
pub fn window_at_point(conn: &X11Connection, x: i32, y: i32) -> Option<Window> {
    for window in conn.client_windows().into_iter().rev() {
        let Some(geometry) = conn.window_geometry(window) else {
            continue;
        };
        let (left, right, top, bottom) = conn.frame_extents(window);
        let frame = Region::new(
            geometry.x - left,
            geometry.y - top,
            (geometry.width as i32 + left + right).max(1) as u32,
            (geometry.height as i32 + top + bottom).max(1) as u32,
        );
        if frame.contains_point(x, y) && conn.is_viewable(window).unwrap_or(false) {
            return Some(window);
        }
    }

    // Fallback for window managers without EWMH: descend the tree and return
    // the top-level window that contains the point.
    let mut current = conn.root;
    for _ in 0..16 {
        let Ok(cookie) = conn.conn.query_tree(current) else {
            break;
        };
        let Ok(tree) = cookie.reply() else {
            break;
        };
        let mut next = None;
        for child in tree.children.iter().rev() {
            if let Some(geometry) = conn.window_geometry(*child) {
                if geometry.contains_point(x, y) {
                    next = Some(*child);
                    break;
                }
            }
        }
        match next {
            Some(child) => current = child,
            None => break,
        }
    }
    if current == conn.root {
        return None;
    }
    // Walk back up to the outermost child of the root window.
    for _ in 0..16 {
        let Ok(cookie) = conn.conn.query_tree(current) else {
            break;
        };
        let Ok(tree) = cookie.reply() else {
            break;
        };
        if tree.parent == conn.root || tree.parent == x11rb::NONE {
            break;
        }
        current = tree.parent;
    }
    let geometry = conn.window_geometry(current)?;
    if geometry.contains_point(x, y) && conn.is_viewable(current).unwrap_or(false) {
        Some(current)
    } else {
        None
    }
}

/// Titles of windows stacked above `window` that overlap its area.
///
/// Used to warn about screenshots taken without a compositor, where overlapping
/// windows bleed into an XGetImage capture.
pub fn overlapping_windows(conn: &X11Connection, window: Window, geometry: &Region) -> Vec<String> {
    let mut overlaps = Vec::new();
    for other in conn.windows_above(window) {
        let Some(other_geometry) = conn.window_geometry(other) else {
            continue;
        };
        if !other_geometry.overlaps(geometry) || !conn.is_viewable(other).unwrap_or(false) {
            continue;
        }
        let title = conn.window_title(other);
        overlaps.push(if title.trim().is_empty() {
            format!("window {other:#x}")
        } else {
            title
        });
    }
    overlaps
}

/// Walk up from a client window to the outermost child of the root window.
///
/// Window managers reparent clients into a decoration frame, and compositors
/// only redirect those toplevels; naming a pixmap for the client itself fails
/// with `BadMatch`.
pub fn toplevel(conn: &X11Connection, window: Window) -> Window {
    let mut current = window;
    for _ in 0..16 {
        let Ok(cookie) = conn.conn.query_tree(current) else {
            break;
        };
        let Ok(tree) = cookie.reply() else {
            break;
        };
        if tree.parent == conn.root || tree.parent == x11rb::NONE {
            break;
        }
        current = tree.parent;
    }
    current
}

/// Position of `window` inside `ancestor` (both on the same screen).
pub fn offset_within(conn: &X11Connection, window: Window, ancestor: Window) -> Option<(i32, i32)> {
    let reply = conn
        .conn
        .translate_coordinates(window, ancestor, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    Some((reply.dst_x as i32, reply.dst_y as i32))
}

pub fn active_target(conn: &X11Connection) -> Option<WindowTarget> {
    let window = conn.active_window()?;
    target_for(conn, window).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_falls_back_to_id() {
        let target = WindowTarget {
            window_id: 0x42,
            title: String::new(),
            class: String::new(),
            pid: None,
        };
        assert_eq!(describe(&target), "window 0x42");
    }
}
