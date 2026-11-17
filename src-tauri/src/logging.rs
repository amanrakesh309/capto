//! Minimal file + stderr logger.
//!
//! The application is normally started from the tray at login, so its stdout is
//! gone. A small log next to the settings file makes "it did nothing" problems
//! diagnosable without pulling in a logging framework.

use log::{LevelFilter, Log, Metadata, Record};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const MAX_LOG_BYTES: u64 = 512 * 1024;

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

static LOGGER: Logger = Logger;

struct Logger;

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} [{:<5}] {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            record.level(),
            record.args()
        );
        eprint!("{line}");
        if let Some(Some(path)) = LOG_PATH.get() {
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(line.as_bytes());
            }
        }
    }

    fn flush(&self) {}
}

/// Start logging into `dir/capto.log`. Safe to call once.
pub fn init(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("capto.log");
    rotate_if_large(&path);
    let _ = LOG_PATH.set(Some(path));
    // `set_logger` fails when a logger is already installed; that is fine
    // because a test harness or an embedding process may have set one.
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(LevelFilter::Info);
}

fn rotate_if_large(path: &Path) {
    let too_large = std::fs::metadata(path)
        .map(|meta| meta.len() > MAX_LOG_BYTES)
        .unwrap_or(false);
    if too_large {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_keeps_a_previous_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        std::fs::write(&path, b"hello").unwrap();
        rotate_if_large(&path);
        assert!(path.exists(), "a small log is not rotated");

        let big = vec![b'x'; (MAX_LOG_BYTES + 10) as usize];
        std::fs::write(&path, big).unwrap();
        rotate_if_large(&path);
        assert!(!path.exists());
        assert!(dir.path().join("app.log.old").exists());
    }
}
