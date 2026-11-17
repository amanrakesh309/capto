# Compatibility and tested environments

This document records what Continuous Screenshot was actually tested against,
which display servers and desktop features it needs, and where it is known to
fall short. It describes release **0.1.0** (November 2026).

## Tested environment

The only environment used for live testing so far is a single X11 desktop:

| Item | Value |
| --- | --- |
| Session type | X11 (`XDG_SESSION_TYPE=x11`, `DISPLAY=:0`) |
| Desktop | Cinnamon (`XDG_CURRENT_DESKTOP=X-Cinnamon`) |
| Window manager | Mutter (Muffin) |
| Compositing manager | Running (`_NET_WM_CM_S0` is owned; Composite extension 0.4) |
| Screen | 1920×1080, depth 24, single monitor (`eDP-1`, 1920×1080+0+0) |
| RandR | Monitor query works; one monitor reported |
| Rust toolchain | 1.99.0 (test build) |
| Test date | November 2026 |

On this machine the following were verified:

* 36 unit tests (`cargo test -p capture-core --lib`) covering naming,
  collision-safe saving, PNG/JPEG output, settings validation/persistence,
  geometry and capture-mode labels.
* 12 live X11 integration tests (`crates/capture-core/tests/x11_live.rs`):
  region capture size, out-of-desktop rejection, pinned-window capture, capture
  of an obscured window (compositor buffer) or explicit refusal, window listing,
  click hit-testing, recycled window ids, closed windows, two distinct PNGs in
  the same second, paused capture, the region-selection overlay, and the window
  picker.
* The Tauri application itself is **not** included in that verification: the
  build environment has no C toolchain/WebKitGTK, so the shell was checked by
  source review only. The Tauri CLI 2.12.3 did locate and accept the project,
  and `tauri.conf.json` parses with the `tauri-utils` 2.10.2 `Config` parser, but
  compilation stopped at the missing C toolchain. Build and exercise the shell
  on a desktop before packaging.

## Display-server support

### X11 — supported

The backend talks to the X server directly through `x11rb` (no `scrot`, `maim`
or portal dependency).

| Operation | Mechanism |
| --- | --- |
| Region capture | `GetImage` on the root window, clipped to the current desktop. |
| Pinned / foreground window capture | `CompositeNameWindowPixmap` on the window manager frame, then crop to the application's client rectangle. |
| Window without a compositor | `GetImage` on the client window, only if no other window overlaps it; otherwise an explicit “target unavailable” error. |
| Region selection | Full-screen override-redirect overlay with a dimmed backdrop and rubber band. Escape or right-click cancels. |
| Window picking | Pointer grab and crosshair; the click is hit-tested against EWMH client windows expanded by `_NET_FRAME_EXTENTS`. |
| Active window | `_NET_ACTIVE_WINDOW` / input focus, used for foreground mode and status text. |

Window captures contain the **client area only** — no title bar, borders, drop
shadows or window-manager buttons.

### XWayland — partially visible, not supported

If `DISPLAY` points at XWayland, the application starts and can capture X11
windows. It cannot see native Wayland windows, so a “foreground” or pinned
capture may be incomplete or report the target as unavailable. An X11 session is
the supported configuration.

### Wayland (no XWayland) — unsupported

A pure Wayland session is detected at startup (`XDG_SESSION_TYPE=wayland` with
no `DISPLAY`). The app still opens, shows a banner explaining that capturing is
unavailable, and keeps its settings editable. It does **not** fall back to a
different capture mechanism. Supporting Wayland would require the
`xdg-desktop-portal` ScreenCast API and a new backend behind the existing trait.

### No compositing manager — covered windows are refused

Without a compositor, X11 does not keep offscreen contents for hidden windows.
If the pinned window is covered, the backend returns a clear “target
unavailable” message naming the blocking window. It never saves the covering
window's pixels and never substitutes a full-screen capture. A partial overlap
is detected before readback; an uncovered window is captured directly.

### Screen layout and coordinates

* All geometry is in **X server pixels** in one virtual-desktop coordinate space,
  so multi-monitor layouts with negative coordinates are represented correctly.
* The selection overlay spans the whole root window, not just one monitor.
* The backend queries RandR for the monitor count and shows it in the
  compatibility notes. A full-screen capture would include all monitors.
* A remembered region that no longer fits is **clipped** to the current desktop
  with a warning when it still overlaps; a region entirely off-screen is
  rejected with “the saved region cannot be used”.
* A pinned window that is partly off-screen is captured (compositor path) with
  a warning; a window entirely off-screen or minimized is reported unavailable.
* Exact multi-monitor behaviour and mixed-DPI setups have **not** been exercised
  in this release (the test machine has one monitor). The coordinate handling is
  designed for it, but treat it as unverified.

### Visual formats

The decoder handles the common X11 true-color layouts (16/24/32 bits per pixel,
LSB/MSB first) and falls back to the root visual. Testing used depth 24; 8-bit
paletted and 30-bit deep-color screens are untested (paletted visuals are not
converted through their colour map). The image is written as 8-bit RGBA PNG or
JPEG regardless of the source depth.

## Desktop environment and tray support

The tray icon uses Tauri's tray implementation, which relies on
StatusNotifierItem/AppIndicator on Linux.

| Desktop | Expected tray support | Status |
| --- | --- | --- |
| Cinnamon | Native | **Tested** |
| KDE Plasma (X11) | Native | Not tested |
| Xfce (X11) | Needs the StatusNotifier plugin | Not tested |
| MATE (X11) | Usually available | Not tested |
| GNOME (X11) | Needs the AppIndicator extension | Not tested |
| sway/wlroots (Wayland) | — | Unsupported (Wayland capture) |

Closing the settings window hides it; the app keeps running in the tray. On a
desktop without StatusNotifier/AppIndicator support the icon will not appear:
the global shortcut still captures, and the settings window is shown on first
run or while no target is selected. Full tray control requires a supported
desktop/panel.

### Global shortcuts

Shortcuts are registered through X11 key grabs (`global_hotkey`). Common
conflicts:

* `PrintScreen` is often already grabbed by the desktop's own screenshot tool.
* Desktop-reserved combinations such as `Super+…` may be refused by the window
  manager.
* Another running application may hold the same accelerator.

When registration fails, the app keeps the previously working shortcut (or none,
if this is the first run), shows a plain-language message, and leaves the tray
menu working. Changes are only persisted after successful registration. An empty
shortcut disables the global shortcut.

## Known gaps and untested areas

* The Tauri shell, `.deb`/AppImage bundles and the tray UI have not been built or
  run in the verification environment (no C toolchain/WebKitGTK).
* Multi-monitor and mixed-DPI behaviour is implemented but unverified.
* Only Cinnamon/Muffin with a compositor has been exercised. Other window
  managers may decorate differently; the code uses `_NET_FRAME_EXTENTS` and a
  tree-descent fallback, but this is not a guarantee.
* Non-composited desktops only support capturing unobscured windows.
* 8-bit and 30-bit visuals, and non-Latin window titles in exotic encodings, are
  untested.
* The app does not use `xdg-desktop-portal`; on X11 it relies on direct X server
  access. Sandboxes (Flatpak/Snap) that hide the X socket or window contents will
  prevent capture. This is expected and reported as a permission/no-display
  error rather than silently producing a blank image.
* No image editing, annotation, OCR, scrolling capture, timers or video.

## Reproducing the live tests

From an X11 session with the screen unlocked:

```sh
cargo test -p capture-core --test x11_live -- --ignored --test-threads=1
```

The tests create and destroy their own small windows. The two interactive tests
(`region_overlay_records_a_drag`, `window_picker_click_returns_the_window_under_the_pointer`)
drive the overlay with XTEST: the pointer moves and clicks, so do not run them
while doing something else. For an unattended run, skip them:

```sh
cargo test -p capture-core --test x11_live -- --ignored --test-threads=1 \
  --skip region_overlay_records_a_drag --skip window_picker_click_returns_the_window_under_the_pointer
```

To report a compatibility problem, include:

* `echo "$XDG_SESSION_TYPE $XDG_CURRENT_DESKTOP $DISPLAY"`
* your window manager/compositor and whether a compositor is running
* monitor layout (`xrandr --query` or your desktop's display settings)
* the contents of `~/.config/io.github.continuousscreenshot/continuous-screenshot.log`
* whether the failure happens for region captures, window captures, or both
