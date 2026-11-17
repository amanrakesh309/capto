//! Thin, testable wrapper around an X11 connection.
//!
//! Everything here is deliberately deterministic (no hidden retries, no
//! fallbacks to other targets) so the capture path stays predictable.

use crate::error::{CaptureError, Result};
use crate::geometry::Region;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::randr::ConnectionExt as RandrExt;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

/// Pixel layout of an X11 visual, used to turn server bytes into RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualFormat {
    pub depth: u8,
    pub bits_per_pixel: u8,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub lsb_first: bool,
}

impl VisualFormat {
    pub fn bytes_per_pixel(&self) -> usize {
        (self.bits_per_pixel as usize).div_ceil(8).max(1)
    }

    /// Look up a visual by id anywhere in the setup (all screens, all depths).
    pub fn for_visual(setup: &Setup, visual_id: Visualid) -> Option<Self> {
        Self::find(setup, |_, visual| visual.visual_id == visual_id)
    }

    /// First visual with the given depth, used when the server does not report
    /// the visual of a `GetImage` reply (pixmaps).
    pub fn for_depth(setup: &Setup, depth: u8) -> Option<Self> {
        Self::find(setup, |candidate_depth, _| candidate_depth == depth)
    }

    fn find(setup: &Setup, predicate: impl Fn(u8, &Visualtype) -> bool) -> Option<Self> {
        for root in &setup.roots {
            for depth in &root.allowed_depths {
                for visual in &depth.visuals {
                    if predicate(depth.depth, visual) {
                        let bits_per_pixel = setup
                            .pixmap_formats
                            .iter()
                            .find(|format| format.depth == depth.depth)
                            .map(|format| format.bits_per_pixel)
                            .unwrap_or(32);
                        return Some(Self {
                            depth: depth.depth,
                            bits_per_pixel,
                            red_mask: visual.red_mask,
                            green_mask: visual.green_mask,
                            blue_mask: visual.blue_mask,
                            lsb_first: matches!(setup.image_byte_order, ImageOrder::LSB_FIRST),
                        });
                    }
                }
            }
        }
        None
    }

    fn channel(&self, pixel: u32, mask: u32) -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        if max == 0 {
            return 0;
        }
        let value = (pixel & mask) >> shift;
        ((value as u64 * 255 + (max as u64) / 2) / max as u64) as u8
    }

    fn read_pixel(&self, bytes: &[u8]) -> u32 {
        match self.bytes_per_pixel() {
            1 => bytes[0] as u32,
            2 => {
                let arr = [bytes[0], bytes[1]];
                if self.lsb_first {
                    u16::from_le_bytes(arr) as u32
                } else {
                    u16::from_be_bytes(arr) as u32
                }
            }
            3 => {
                let arr = [bytes[0], bytes[1], bytes[2]];
                if self.lsb_first {
                    u32::from_le_bytes([arr[0], arr[1], arr[2], 0])
                } else {
                    u32::from_be_bytes([0, arr[0], arr[1], arr[2]])
                }
            }
            _ => {
                let arr = [bytes[0], bytes[1], bytes[2], bytes[3]];
                if self.lsb_first {
                    u32::from_le_bytes(arr)
                } else {
                    u32::from_be_bytes(arr)
                }
            }
        }
    }

    fn write_pixel(&self, rgba: &[u8], out: &mut [u8]) {
        let scale = |value: u8, mask: u32| -> u32 {
            if mask == 0 {
                return 0;
            }
            let shift = mask.trailing_zeros();
            let max = mask >> shift;
            let scaled = ((value as u64 * max as u64 + 127) / 255) as u32;
            (scaled << shift) & mask
        };
        let pixel = scale(rgba[0], self.red_mask)
            | scale(rgba[1], self.green_mask)
            | scale(rgba[2], self.blue_mask);
        match self.bytes_per_pixel() {
            1 => out[0] = pixel as u8,
            2 => {
                let arr = if self.lsb_first {
                    (pixel as u16).to_le_bytes()
                } else {
                    (pixel as u16).to_be_bytes()
                };
                out[..2].copy_from_slice(&arr);
            }
            3 => {
                let arr = if self.lsb_first {
                    pixel.to_le_bytes()
                } else {
                    pixel.to_be_bytes()
                };
                if self.lsb_first {
                    out[..3].copy_from_slice(&arr[..3]);
                } else {
                    out[..3].copy_from_slice(&arr[1..]);
                }
            }
            _ => {
                let arr = if self.lsb_first {
                    pixel.to_le_bytes()
                } else {
                    pixel.to_be_bytes()
                };
                out[..4].copy_from_slice(&arr);
            }
        }
    }

    /// Decode `rows` scanlines of server pixels into tightly packed RGBA.
    pub fn decode_rows(
        &self,
        data: &[u8],
        width: u32,
        rows: u32,
        src_stride: usize,
    ) -> Result<Vec<u8>> {
        let bpp = self.bytes_per_pixel();
        let row_bytes = width as usize * bpp;
        if src_stride < row_bytes {
            return Err(CaptureError::Backend(format!(
                "X11 returned {src_stride} bytes per row, expected at least {row_bytes}"
            )));
        }
        let needed = src_stride * (rows as usize);
        if data.len() < needed {
            return Err(CaptureError::Backend(format!(
                "X11 returned {} bytes, expected {needed}",
                data.len()
            )));
        }
        let mut out = vec![0u8; width as usize * rows as usize * 4];
        for row in 0..rows as usize {
            let src = &data[row * src_stride..row * src_stride + row_bytes];
            let dst = &mut out[row * width as usize * 4..(row + 1) * width as usize * 4];
            for (col, pixel_bytes) in src.chunks_exact(bpp).enumerate() {
                let pixel = self.read_pixel(pixel_bytes);
                let dst = &mut dst[col * 4..col * 4 + 4];
                dst[0] = self.channel(pixel, self.red_mask);
                dst[1] = self.channel(pixel, self.green_mask);
                dst[2] = self.channel(pixel, self.blue_mask);
                dst[3] = 255;
            }
        }
        Ok(out)
    }

    /// Encode RGBA into the server's pixel layout for `PutImage`.
    ///
    /// Scanlines are padded to a 4 byte boundary, as the X protocol requires.
    pub fn encode(&self, rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        let bpp = self.bytes_per_pixel();
        let row_bytes = width as usize * bpp;
        let stride = row_bytes.div_ceil(4) * 4;
        let mut out = vec![0u8; stride * height as usize];
        for row in 0..height as usize {
            for col in 0..width as usize {
                let src =
                    &rgba[(row * width as usize + col) * 4..(row * width as usize + col) * 4 + 4];
                let dst_start = row * stride + col * bpp;
                self.write_pixel(src, &mut out[dst_start..dst_start + bpp]);
            }
        }
        out
    }

    /// Multiply RGB by `factor` (used to dim the selection overlay).
    pub fn dim(rgba: &mut [u8], factor: f32) {
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[0] = (pixel[0] as f32 * factor) as u8;
            pixel[1] = (pixel[1] as f32 * factor) as u8;
            pixel[2] = (pixel[2] as f32 * factor) as u8;
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Atoms {
    pub net_client_list: Atom,
    pub net_client_list_stacking: Atom,
    pub net_active_window: Atom,
    pub net_wm_name: Atom,
    pub net_wm_pid: Atom,
    pub net_wm_state: Atom,
    pub net_wm_state_hidden: Atom,
    pub net_frame_extents: Atom,
    pub net_wm_window_type: Atom,
    pub net_wm_window_type_dock: Atom,
    pub net_wm_window_type_desktop: Atom,
    pub net_wm_cm: Atom,
    pub utf8_string: Atom,
    pub wm_name: Atom,
    pub wm_class: Atom,
}

pub struct X11Connection {
    pub conn: RustConnection,
    pub screen_num: usize,
    pub root: Window,
    pub root_depth: u8,
    pub bounds: Region,
    pub atoms: Atoms,
    pub has_composite: bool,
    pub has_randr: bool,
    /// A compositing manager currently owns `_NET_WM_CM_Sn`.
    pub composited: bool,
    pub root_format: VisualFormat,
}

impl X11Connection {
    pub fn connect() -> Result<Self> {
        match std::env::var_os("DISPLAY") {
            Some(value) if !value.is_empty() => {}
            _ => {
                return Err(CaptureError::NoDisplay(
                    "DISPLAY is not set, so no X11 server can be reached".into(),
                ))
            }
        }
        let (conn, screen_num) = x11rb::connect(None)
            .map_err(|err| CaptureError::NoDisplay(format!("could not connect to X11: {err}")))?;
        let setup = conn.setup();
        let screen = setup
            .roots
            .get(screen_num)
            .ok_or_else(|| CaptureError::NoDisplay("the X11 server has no screen".into()))?;
        let root = screen.root;
        let root_depth = screen.root_depth;
        let bounds = Region::new(
            0,
            0,
            screen.width_in_pixels as u32,
            screen.height_in_pixels as u32,
        );
        let root_format = VisualFormat::for_visual(setup, screen.root_visual).ok_or_else(|| {
            CaptureError::Backend("the root visual is missing from the X11 setup".into())
        })?;

        let has_composite = conn
            .extension_information(x11rb::protocol::composite::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_some();
        let has_randr = conn
            .extension_information(x11rb::protocol::randr::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_some();

        let net_wm_cm = Self::intern(&conn, &format!("_NET_WM_CM_S{screen_num}"))?;
        let composited = conn
            .get_selection_owner(net_wm_cm)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.owner != x11rb::NONE)
            .unwrap_or(false);

        let atoms = Atoms {
            net_client_list: Self::intern(&conn, "_NET_CLIENT_LIST")?,
            net_client_list_stacking: Self::intern(&conn, "_NET_CLIENT_LIST_STACKING")?,
            net_active_window: Self::intern(&conn, "_NET_ACTIVE_WINDOW")?,
            net_wm_name: Self::intern(&conn, "_NET_WM_NAME")?,
            net_wm_pid: Self::intern(&conn, "_NET_WM_PID")?,
            net_wm_state: Self::intern(&conn, "_NET_WM_STATE")?,
            net_wm_state_hidden: Self::intern(&conn, "_NET_WM_STATE_HIDDEN")?,
            net_frame_extents: Self::intern(&conn, "_NET_FRAME_EXTENTS")?,
            net_wm_window_type: Self::intern(&conn, "_NET_WM_WINDOW_TYPE")?,
            net_wm_window_type_dock: Self::intern(&conn, "_NET_WM_WINDOW_TYPE_DOCK")?,
            net_wm_window_type_desktop: Self::intern(&conn, "_NET_WM_WINDOW_TYPE_DESKTOP")?,
            net_wm_cm,
            utf8_string: Self::intern(&conn, "UTF8_STRING")?,
            wm_name: Self::intern(&conn, "WM_NAME")?,
            wm_class: Self::intern(&conn, "WM_CLASS")?,
        };

        Ok(Self {
            conn,
            screen_num,
            root,
            root_depth,
            bounds,
            atoms,
            has_composite,
            has_randr,
            composited,
            root_format,
        })
    }

    fn intern(conn: &RustConnection, name: &str) -> Result<Atom> {
        conn.intern_atom(false, name.as_bytes())
            .map_err(|err| CaptureError::Backend(format!("intern_atom({name}) failed: {err}")))?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|err| CaptureError::Backend(format!("intern_atom({name}) failed: {err}")))
    }

    /// `format = 32` property as a list of cardinals/atoms/window ids.
    pub fn property_u32(&self, window: Window, property: Atom) -> Vec<u32> {
        if property == x11rb::NONE {
            return Vec::new();
        }
        let reply = self
            .conn
            .get_property(false, window, property, AtomEnum::ANY, 0, 4096)
            .ok()
            .and_then(|cookie| cookie.reply().ok());
        let Some(reply) = reply else {
            return Vec::new();
        };
        if reply.format != 32 {
            return Vec::new();
        }
        reply
            .value
            .chunks_exact(4)
            .map(|chunk| {
                let arr = [chunk[0], chunk[1], chunk[2], chunk[3]];
                if self.root_format.lsb_first {
                    u32::from_le_bytes(arr)
                } else {
                    u32::from_be_bytes(arr)
                }
            })
            .collect()
    }

    pub fn property_u32_first(&self, window: Window, property: Atom) -> Option<u32> {
        self.property_u32(window, property).into_iter().next()
    }

    pub fn property_text(&self, window: Window, property: Atom) -> Option<String> {
        if property == x11rb::NONE {
            return None;
        }
        let reply = self
            .conn
            .get_property(false, window, property, AtomEnum::ANY, 0, 1024)
            .ok()
            .and_then(|cookie| cookie.reply().ok())?;
        if reply.value.is_empty() {
            return None;
        }
        // Some legacy clients store WM_NAME as Latin-1; lossy conversion keeps
        // the app usable instead of hiding the window.
        let text = String::from_utf8_lossy(&reply.value)
            .trim_end_matches('\0')
            .to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub fn window_title(&self, window: Window) -> String {
        self.property_text(window, self.atoms.net_wm_name)
            .or_else(|| self.property_text(window, self.atoms.wm_name))
            .unwrap_or_default()
    }

    /// `WM_CLASS` as `instance class`; returns the class when present.
    pub fn window_class(&self, window: Window) -> String {
        let reply = self
            .conn
            .get_property(false, window, self.atoms.wm_class, AtomEnum::STRING, 0, 512)
            .ok()
            .and_then(|cookie| cookie.reply().ok());
        let Some(reply) = reply else {
            return String::new();
        };
        let parts: Vec<String> = reply
            .value
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).to_string())
            .collect();
        parts.last().cloned().unwrap_or_default()
    }

    pub fn window_pid(&self, window: Window) -> Option<u32> {
        self.property_u32_first(window, self.atoms.net_wm_pid)
    }

    /// Window geometry translated to root coordinates.
    pub fn window_geometry(&self, window: Window) -> Option<Region> {
        let geometry = self.conn.get_geometry(window).ok()?.reply().ok()?;
        let translated = self
            .conn
            .translate_coordinates(window, self.root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        Some(Region::new(
            translated.dst_x as i32,
            translated.dst_y as i32,
            geometry.width as u32,
            geometry.height as u32,
        ))
    }

    /// `(left, right, top, bottom)` window manager decoration sizes.
    pub fn frame_extents(&self, window: Window) -> (i32, i32, i32, i32) {
        let extents = self.property_u32(window, self.atoms.net_frame_extents);
        if extents.len() == 4 {
            (
                extents[0] as i32,
                extents[1] as i32,
                extents[2] as i32,
                extents[3] as i32,
            )
        } else {
            (0, 0, 0, 0)
        }
    }

    pub fn is_viewable(&self, window: Window) -> Result<bool> {
        let attrs = self
            .conn
            .get_window_attributes(window)
            .map_err(|_| CaptureError::TargetUnavailable {
                detail: "the window no longer exists".into(),
            })?
            .reply()
            .map_err(|_| CaptureError::TargetUnavailable {
                detail: "the window no longer exists".into(),
            })?;
        Ok(matches!(attrs.map_state, MapState::VIEWABLE))
    }

    pub fn is_minimized(&self, window: Window) -> bool {
        self.property_u32(window, self.atoms.net_wm_state)
            .contains(&self.atoms.net_wm_state_hidden)
    }

    pub fn active_window(&self) -> Option<Window> {
        self.property_u32_first(self.root, self.atoms.net_active_window)
            .filter(|id| *id != x11rb::NONE)
    }

    /// Client windows in bottom-to-top stacking order (EWMH).
    ///
    /// Falls back to `_NET_CLIENT_LIST` and finally to the root's children when
    /// a window manager does not publish EWMH properties.
    pub fn client_windows(&self) -> Vec<Window> {
        let stacking = self.property_u32(self.root, self.atoms.net_client_list_stacking);
        if !stacking.is_empty() {
            return stacking;
        }
        let clients = self.property_u32(self.root, self.atoms.net_client_list);
        if !clients.is_empty() {
            return clients;
        }
        let mut fallback = Vec::new();
        if let Some(reply) = self
            .conn
            .query_tree(self.root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        {
            for child in reply.children {
                if let Some(geometry) = self.window_geometry(child) {
                    if geometry.is_usable() && self.is_viewable(child).unwrap_or(false) {
                        fallback.push(child);
                    }
                }
            }
        }
        fallback
    }

    /// Windows stacked above `window`, topmost first.
    pub fn windows_above(&self, window: Window) -> Vec<Window> {
        let mut above = Vec::new();
        for id in self.client_windows().into_iter().rev() {
            if id == window {
                break;
            }
            above.push(id);
        }
        above
    }

    pub fn randr_monitors(&self) -> Vec<Region> {
        if !self.has_randr {
            return Vec::new();
        }
        let Ok(cookie) = self.conn.randr_get_monitors(self.root, true) else {
            return Vec::new();
        };
        let Ok(reply) = cookie.reply() else {
            return Vec::new();
        };
        reply
            .monitors
            .iter()
            .map(|monitor| {
                Region::new(
                    monitor.x as i32,
                    monitor.y as i32,
                    monitor.width as u32,
                    monitor.height as u32,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(bpp: u8, lsb: bool, r: u32, g: u32, b: u32) -> VisualFormat {
        VisualFormat {
            depth: 24,
            bits_per_pixel: bpp,
            red_mask: r,
            green_mask: g,
            blue_mask: b,
            lsb_first: lsb,
        }
    }

    #[test]
    fn decodes_common_24bit_pixels() {
        let f = format(32, true, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff);
        // BGRX little endian: 0x00RRGGBB stored as BB GG RR 00
        let data = [0x30u8, 0x20, 0x10, 0x00];
        let rgba = f.decode_rows(&data, 1, 1, 4).unwrap();
        assert_eq!(rgba, vec![0x10, 0x20, 0x30, 0xff]);
    }

    #[test]
    fn decodes_16bit_565() {
        let f = format(16, true, 0xf800, 0x07e0, 0x001f);
        // 0xF800 (pure red) little endian
        let data = [0x00u8, 0xf8, 0xe0, 0x07];
        let rgba = f.decode_rows(&data, 2, 1, 4).unwrap();
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
        // 0x07E0 pure green
        assert_eq!(&rgba[4..8], &[0, 255, 0, 255]);
    }

    #[test]
    fn roundtrip_encode_decode() {
        let f = format(32, true, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff);
        let rgba = vec![
            10, 20, 30, 255, 200, 100, 50, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ];
        let encoded = f.encode(&rgba, 2, 2);
        let decoded = f.decode_rows(&encoded, 2, 2, encoded.len() / 2).unwrap();
        assert_eq!(decoded, rgba);
    }

    #[test]
    fn rejects_short_buffers() {
        let f = format(32, true, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff);
        assert!(f.decode_rows(&[0; 4], 4, 2, 16).is_err());
    }

    #[test]
    fn dim_scales_rgb_only() {
        let mut rgba = vec![100, 100, 100, 255];
        VisualFormat::dim(&mut rgba, 0.5);
        assert_eq!(rgba, vec![50, 50, 50, 255]);
    }
}
