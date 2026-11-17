//! Live X11 integration tests.
//!
//! These need a running X server, so they are `#[ignore]`d by default. Run
//! them on a machine with a desktop session:
//!
//! ```sh
//! cargo test -p capture-core --test x11_live -- --ignored --test-threads=1
//! ```
//!
//! The tests create small, short-lived windows of their own; nothing else on the
//! desktop is captured or modified.

use capture_core::backend::x11::conn::X11Connection;
use capture_core::backend::x11::{windows, X11Backend};
use capture_core::backend::CaptureBackend;
use capture_core::geometry::Region;
use capture_core::settings::{CaptureMode, Settings, WindowTarget};
use capture_core::storage::ImageFormat;
use capture_core::CaptureService;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::xtest::ConnectionExt as XtestExt;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperExt;

const TEST_COLOR: (u8, u8, u8) = (17, 102, 187);

struct TestWindow {
    conn: RustConnection,
    window: Window,
}

impl TestWindow {
    fn new(name: &str, x: i16, y: i16, width: u16, height: u16, color: (u8, u8, u8)) -> Self {
        let (conn, screen_num) = x11rb::connect(None).expect("connect to X11");
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let format = X11Connection::connect().expect("format").root_format;
        let pixel = pack_color(&format, color.0, color.1, color.2);
        let window = conn.generate_id().unwrap();
        conn.create_window(
            screen.root_depth,
            window,
            root,
            x,
            y,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().background_pixel(pixel),
        )
        .expect("create window")
        .check()
        .expect("create window");

        let net_wm_name = conn
            .intern_atom(false, b"_NET_WM_NAME")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let utf8 = conn
            .intern_atom(false, b"UTF8_STRING")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let pid = conn
            .intern_atom(false, b"_NET_WM_PID")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            net_wm_name,
            utf8,
            name.as_bytes(),
        )
        .unwrap();
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            name.as_bytes(),
        )
        .unwrap();
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"capture-core-test\0CaptureCoreTest\0",
        )
        .unwrap();
        conn.change_property32(
            PropMode::REPLACE,
            window,
            pid,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )
        .unwrap();
        conn.map_window(window).unwrap();
        // Raise it so hit-testing is not fooled by other windows on the desktop.
        conn.configure_window(
            window,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        )
        .unwrap();
        conn.flush().unwrap();

        let test = Self { conn, window };
        test.wait_viewable();
        test
    }

    fn wait_viewable(&self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let attrs = self
                .conn
                .get_window_attributes(self.window)
                .unwrap()
                .reply();
            if let Ok(attrs) = attrs {
                if matches!(attrs.map_state, MapState::VIEWABLE) {
                    // Give the window manager a moment to finish decorating.
                    std::thread::sleep(Duration::from_millis(250));
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("test window never became viewable");
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
    }
}

fn pack_color(format: &capture_core::backend::x11::conn::VisualFormat, r: u8, g: u8, b: u8) -> u32 {
    // Reuse the encoder so the test uses exactly this desktop's pixel layout.
    let encoded = format.encode(&[r, g, b, 255], 1, 1);
    let bpp = format.bytes_per_pixel();
    let bytes = [encoded[0], encoded[1], encoded[2], encoded[3]];
    match (bpp, format.lsb_first) {
        (1, _) => bytes[0] as u32,
        (2, true) => u16::from_le_bytes([bytes[0], bytes[1]]) as u32,
        (2, false) => u16::from_be_bytes([bytes[0], bytes[1]]) as u32,
        (3, true) => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]),
        (3, false) => u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]),
        (_, true) => u32::from_le_bytes(bytes),
        (_, false) => u32::from_be_bytes(bytes),
    }
}

fn sample(image: &capture_core::RawImage, x: u32, y: u32) -> (u8, u8, u8) {
    let index = ((y * image.width + x) * 4) as usize;
    (
        image.rgba[index],
        image.rgba[index + 1],
        image.rgba[index + 2],
    )
}

fn approx(actual: (u8, u8, u8), expected: (u8, u8, u8), tolerance: i32) -> bool {
    (actual.0 as i32 - expected.0 as i32).abs() <= tolerance
        && (actual.1 as i32 - expected.1 as i32).abs() <= tolerance
        && (actual.2 as i32 - expected.2 as i32).abs() <= tolerance
}

/// Send one XTEST event and wait for the server to process it.
fn fake_input(conn: &RustConnection, root: Window, type_: u8, detail: u8, x: i16, y: i16) {
    conn.xtest_fake_input(type_, detail, 0, root, x, y, 0)
        .unwrap()
        .check()
        .unwrap();
    conn.flush().unwrap();
}

fn pointer_position(conn: &RustConnection, root: Window) -> (i16, i16) {
    let reply = conn.query_pointer(root).unwrap().reply().unwrap();
    (reply.root_x, reply.root_y)
}

/// Move the pointer and confirm the server actually got there.
///
/// A compositor may coalesce or briefly delay synthetic motion, in which case a
/// button press would land at the pointer's previous position and make the
/// selection start somewhere else. Retrying the motion keeps the test
/// deterministic without touching the overlay code itself.
fn move_pointer_to(conn: &RustConnection, root: Window, x: i16, y: i16) {
    for _ in 0..50 {
        fake_input(conn, root, MOTION_NOTIFY_EVENT, 0, x, y);
        std::thread::sleep(Duration::from_millis(30));
        let (actual_x, actual_y) = pointer_position(conn, root);
        if (actual_x - x).abs() <= 2 && (actual_y - y).abs() <= 2 {
            return;
        }
    }
    panic!("the pointer never reached {x},{y}");
}

#[test]
#[ignore = "needs a live X11 session"]
fn region_capture_returns_requested_size() {
    let mut backend = X11Backend::connect().expect("backend");
    let frame = backend
        .capture_region(Region::new(4, 4, 64, 48))
        .expect("capture");
    assert_eq!(frame.image.width, 64);
    assert_eq!(frame.image.height, 48);
    assert_eq!(frame.image.rgba.len(), 64 * 48 * 4);
}

#[test]
#[ignore = "needs a live X11 session"]
fn region_outside_the_desktop_is_rejected() {
    let mut backend = X11Backend::connect().expect("backend");
    let bounds = backend.screen_bounds().unwrap();
    let err = backend
        .capture_region(Region::new(bounds.width as i32 + 500, 10, 100, 100))
        .unwrap_err();
    assert_eq!(err.code(), "invalid_region");
}

#[test]
#[ignore = "needs a live X11 session"]
fn pinned_window_capture_uses_the_window_itself() {
    let window = TestWindow::new("CaptureCoreTarget", 120, 120, 240, 180, TEST_COLOR);
    let conn = X11Connection::connect().expect("conn");
    let target = windows::target_for(&conn, window.window).expect("target");
    assert_eq!(target.class, "CaptureCoreTest");

    let mut backend = X11Backend::connect().expect("backend");
    let frame = backend.capture_window(&target).expect("capture window");
    assert_eq!(frame.image.width, 240);
    assert_eq!(frame.image.height, 180);
    assert!(
        approx(sample(&frame.image, 120, 90), TEST_COLOR, 3),
        "expected the window's own colour, got {:?}",
        sample(&frame.image, 120, 90)
    );
}

#[test]
#[ignore = "needs a live X11 session"]
fn obscured_pinned_window_is_captured_background_or_refused() {
    // A window manager with a compositor can capture the covered window from
    // its offscreen buffer. Without one, the backend must refuse rather than
    // save the covering window.
    let target = TestWindow::new("CaptureCoreCovered", 300, 200, 260, 200, TEST_COLOR);
    let _cover = TestWindow::new("CaptureCoreCover", 320, 220, 260, 200, (220, 30, 30));

    let conn = X11Connection::connect().expect("conn");
    let target_info = windows::target_for(&conn, target.window).expect("target");
    let mut backend = X11Backend::connect().expect("backend");
    match backend.capture_window(&target_info) {
        Ok(frame) => {
            assert!(
                conn.composited,
                "covered capture only works with a compositor"
            );
            let pixel = sample(&frame.image, 130, 100);
            assert!(
                approx(pixel, TEST_COLOR, 8),
                "covered window captured something else: {pixel:?}"
            );
        }
        Err(err) => {
            assert!(
                !conn.composited,
                "covered capture should work on a composited desktop"
            );
            assert_eq!(err.code(), "target_unavailable");
            assert!(err.user_message().contains("covered"));
        }
    }
}

#[test]
#[ignore = "needs a live X11 session"]
fn window_list_contains_the_test_window() {
    let window = TestWindow::new("CaptureCoreListed", 80, 400, 200, 150, (40, 160, 90));
    let mut backend = X11Backend::connect().expect("backend");
    let windows = backend.list_windows().expect("list");
    let found = windows
        .iter()
        .find(|info| info.target.window_id == window.window);
    let found = found.expect("test window must be listed");
    assert_eq!(found.target.title, "CaptureCoreListed");
    assert!(!found.minimized);
}

#[test]
#[ignore = "needs a live X11 session"]
fn click_hit_test_finds_the_client_window() {
    let target = TestWindow::new("CaptureCoreHit", 600, 300, 220, 160, (90, 40, 160));
    let conn = X11Connection::connect().expect("conn");
    let geometry = conn.window_geometry(target.window).expect("geometry");
    let hit = windows::window_at_point(
        &conn,
        geometry.x + geometry.width as i32 / 2,
        geometry.y + geometry.height as i32 / 2,
    );
    if hit != Some(target.window) {
        let hit_title = hit.map(|w| conn.window_title(w)).unwrap_or_default();
        panic!(
            "hit test returned {hit:?} ({hit_title:?}) instead of {:?} at {},{}",
            target.window, geometry.x, geometry.y
        );
    }
}

#[test]
#[ignore = "needs a live X11 session"]
fn recycled_window_id_is_detected() {
    let window = TestWindow::new("CaptureCoreRecycled", 700, 500, 160, 120, (10, 10, 10));
    let conn = X11Connection::connect().expect("conn");
    let mut target = windows::target_for(&conn, window.window).expect("target");
    target.class = "SomethingElse".to_string();
    let mut backend = X11Backend::connect().expect("backend");
    let problem = backend.check_target(&target).expect("mismatch expected");
    assert!(problem.contains("belongs to"));
    target.class = "CaptureCoreTest".into();
    target.pid = Some(u32::MAX);
    let problem = backend
        .check_target(&target)
        .expect("pid mismatch expected");
    assert!(problem.contains("process"));
}

#[test]
#[ignore = "needs a live X11 session"]
fn closed_window_is_reported_as_unavailable() {
    let window = TestWindow::new("CaptureCoreClosed", 500, 600, 180, 120, (30, 30, 200));
    let conn = X11Connection::connect().expect("conn");
    let target: WindowTarget = windows::target_for(&conn, window.window).expect("target");
    let mut backend = X11Backend::connect().expect("backend");
    drop(window);
    std::thread::sleep(Duration::from_millis(300));
    let err = backend.capture_window(&target).unwrap_err();
    assert_eq!(err.code(), "target_unavailable");
}

#[test]
#[ignore = "needs a live X11 session"]
fn service_saves_two_distinct_pngs() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        mode: CaptureMode::Region,
        region: Region::new(20, 20, 96, 64),
        output_dir: dir.path().to_path_buf(),
        format: ImageFormat::Png,
        ..Settings::default()
    };
    settings.paused = false;
    let mut service = CaptureService::new(settings).expect("service");
    let first = service.capture_now().expect("first capture");
    let second = service.capture_now().expect("second capture");
    assert_eq!((first.saved.width, first.saved.height), (96, 64));
    assert!(first.saved.path.exists());
    assert!(second.saved.path.exists());
    assert_ne!(first.saved.filename, second.saved.filename);
    let bytes = std::fs::read(&first.saved.path).unwrap();
    assert_eq!(&bytes[..4], b"\x89PNG");
    let decoded = image::load_from_memory(&bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (96, 64));
    assert!(service.status().unwrap().target_ready);
}

#[test]
#[ignore = "needs a live X11 session"]
fn repeated_captures_always_create_a_new_file() {
    // Rapid presses land in the same second, so this exercises the collision
    // suffix path as well as the write-probe and folder checks on every save.
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        mode: CaptureMode::Region,
        region: Region::new(20, 20, 128, 96),
        output_dir: dir.path().to_path_buf(),
        ..Settings::default()
    };
    settings.paused = false;
    let mut service = CaptureService::new(settings).expect("service");

    const CAPTURES: usize = 25;
    for index in 0..CAPTURES {
        let outcome = service
            .capture_now()
            .unwrap_or_else(|err| panic!("capture {index} failed: {err}"));
        assert!(
            outcome.saved.path.exists(),
            "capture {index} reported {} but the file is missing",
            outcome.saved.path.display()
        );
    }

    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .collect();
    assert_eq!(
        files.len(),
        CAPTURES,
        "every capture must leave its own file behind"
    );
}

#[test]
#[ignore = "needs a live X11 session"]
fn repeated_pinned_window_captures_always_create_a_new_file() {
    let window = TestWindow::new("CaptureCoreRepeat", 160, 160, 200, 140, TEST_COLOR);
    let conn = X11Connection::connect().expect("conn");
    let target = windows::target_for(&conn, window.window).expect("target");
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        mode: CaptureMode::PinnedWindow,
        window: Some(target),
        output_dir: dir.path().to_path_buf(),
        ..Settings::default()
    };
    settings.paused = false;
    let mut service = CaptureService::new(settings).expect("service");

    const CAPTURES: usize = 15;
    for index in 0..CAPTURES {
        let outcome = service
            .capture_now()
            .unwrap_or_else(|err| panic!("capture {index} failed: {err}"));
        assert!(
            outcome.saved.path.exists(),
            "capture {index} reported {} but the file is missing",
            outcome.saved.path.display()
        );
    }

    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .collect();
    assert_eq!(files.len(), CAPTURES);
}

#[test]
#[ignore = "needs a live X11 session"]
fn captures_from_many_threads_never_lose_a_file() {
    // The app funnels every shortcut/tray press through a fresh worker thread
    // while sharing one capture service. Reproduce that pattern, including a
    // burst of simultaneous triggers.
    use std::sync::{Arc, Mutex};

    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        mode: CaptureMode::Region,
        region: Region::new(20, 20, 64, 64),
        output_dir: dir.path().to_path_buf(),
        ..Settings::default()
    };
    settings.paused = false;
    let service = Arc::new(Mutex::new(CaptureService::new(settings).expect("service")));

    const CAPTURES: usize = 24;
    let mut handles = Vec::new();
    for index in 0..CAPTURES {
        let service = Arc::clone(&service);
        handles.push(std::thread::spawn(move || {
            let mut service = service.lock().unwrap();
            service
                .capture_now()
                .unwrap_or_else(|err| panic!("capture {index} failed: {err}"))
                .saved
                .path
        }));
    }
    for handle in handles {
        let path = handle.join().expect("capture thread");
        assert!(path.exists(), "{} is missing", path.display());
    }

    let files = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .count();
    assert_eq!(files, CAPTURES);
}

#[test]
#[ignore = "needs a live X11 session"]
fn paused_service_refuses_to_capture() {
    let dir = tempfile::tempdir().unwrap();
    let mut service = CaptureService::new(Settings {
        region: Region::new(20, 20, 64, 64),
        output_dir: dir.path().to_path_buf(),
        ..Settings::default()
    })
    .expect("service");
    service.set_paused(true);
    let err = service.capture_now().unwrap_err();
    assert_eq!(err.code(), "paused");
    assert!(dir.path().read_dir().unwrap().next().is_none());
}

#[test]
#[ignore = "needs a live X11 session"]
fn region_overlay_records_a_drag() {
    let mut backend = X11Backend::connect().expect("backend");
    let bounds = backend.screen_bounds().unwrap();
    assert!(bounds.width > 400 && bounds.height > 300);

    let handle = std::thread::spawn(move || backend.select_region());
    std::thread::sleep(Duration::from_millis(1200));

    let (conn, screen_num) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[screen_num].root;
    move_pointer_to(&conn, root, 100, 100);
    fake_input(&conn, root, BUTTON_PRESS_EVENT, 1, 100, 100);
    for step in 1..=8 {
        let x = 100 + step * 20;
        let y = 100 + step * 10;
        move_pointer_to(&conn, root, x, y);
    }
    move_pointer_to(&conn, root, 260, 180);
    fake_input(&conn, root, BUTTON_RELEASE_EVENT, 1, 260, 180);

    let selected = handle.join().expect("thread").expect("selection");
    let selected = selected.expect("a region should have been selected");
    assert!(
        (selected.width as i32 - 160).abs() <= 4,
        "unexpected width: {selected}"
    );
    assert!(
        (selected.height as i32 - 80).abs() <= 4,
        "unexpected height: {selected}"
    );
    assert!((selected.x - 100).abs() <= 4, "unexpected x: {selected}");
    assert!((selected.y - 100).abs() <= 4, "unexpected y: {selected}");
}

#[test]
#[ignore = "needs a live X11 session"]
fn window_picker_click_returns_the_window_under_the_pointer() {
    let target = TestWindow::new("CaptureCorePicked", 400, 250, 240, 180, (70, 70, 70));
    let conn_info = X11Connection::connect().expect("conn");
    let geometry = conn_info.window_geometry(target.window).expect("geometry");
    let center = (
        (geometry.x + geometry.width as i32 / 2) as i16,
        (geometry.y + geometry.height as i32 / 2) as i16,
    );

    let mut backend = X11Backend::connect().expect("backend");
    let handle = std::thread::spawn(move || backend.select_window());
    std::thread::sleep(Duration::from_millis(1200));

    let (conn, screen_num) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[screen_num].root;
    move_pointer_to(&conn, root, center.0, center.1);
    fake_input(&conn, root, BUTTON_PRESS_EVENT, 1, center.0, center.1);
    std::thread::sleep(Duration::from_millis(80));
    fake_input(&conn, root, BUTTON_RELEASE_EVENT, 1, center.0, center.1);

    let picked = handle.join().expect("thread").expect("pick");
    let picked = picked.expect("a window should have been picked");
    assert_eq!(picked.window_id, target.window);
    assert_eq!(picked.title, "CaptureCorePicked");
}
