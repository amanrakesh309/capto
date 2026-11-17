//! Filename generation: timestamped names, safe characters, collision proof
//! allocation.
//!
//! The rule from the product spec is simple: never overwrite a file, always
//! produce a recognisable name such as `Screenshot_2026-11-13_14-32-08.png`.

use chrono::{DateTime, Local};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// Default pattern; the date/time tokens are expanded at capture time.
pub const DEFAULT_PATTERN: &str = "Screenshot_{date}_{time}";

/// Tokens accepted in a filename pattern.
pub const PATTERN_TOKENS: [&str; 4] = ["{date}", "{time}", "{datetime}", "{mode}"];

/// How many `_1`, `_2`, ... suffixes we try before giving up.
const MAX_COLLISION_SUFFIX: u32 = 999;

/// Expand `{date}`, `{time}`, `{datetime}` and `{mode}` tokens and make the
/// result safe as a single path component.
pub fn render_pattern(pattern: &str, now: &DateTime<Local>, mode: &str) -> String {
    let expanded = pattern
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H-%M-%S").to_string())
        .replace("{datetime}", &now.format("%Y-%m-%d_%H-%M-%S").to_string())
        .replace("{mode}", mode);
    sanitize_component(&expanded)
}

/// Strip path separators and characters that hurt on Linux filesystems while
/// keeping the name readable.
pub fn sanitize_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_space = false;
    for ch in raw.chars() {
        let mapped = match ch {
            '/' | '\\' | '\0' => '-',
            ':' => '-',
            c if c.is_control() => ' ',
            c => c,
        };
        if mapped == ' ' {
            if last_was_space {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }
        out.push(mapped);
    }
    // Leading dots hide files, and a leading dash looks like a CLI flag.
    let trimmed = out
        .trim()
        .trim_matches(|c| c == '.' || c == '-' || c == ' ');
    // Keep room for the extension and collision suffix on common filesystems.
    let mut limited: String = trimmed.chars().take(120).collect();
    if limited.is_empty() {
        limited = "Screenshot".to_string();
    }
    if limited == "." || limited == ".." {
        return "Screenshot".to_string();
    }
    limited
}

/// Create `dir/<base>.<ext>` without ever clobbering an existing file.
///
/// Returns the open file and its final path. The file is created with
/// `create_new`, so two processes racing for the same second cannot truncate
/// each other's screenshot.
pub fn create_unique_file(dir: &Path, base: &str, extension: &str) -> io::Result<(File, PathBuf)> {
    let base = sanitize_component(base);
    for attempt in 0..=MAX_COLLISION_SUFFIX {
        let name = if attempt == 0 {
            format!("{base}.{extension}")
        } else {
            format!("{base}_{attempt}.{extension}")
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("no free filename for {base} in {}", dir.display()),
    ))
}

/// True when the pattern contains at least one token, so two captures a second
/// apart are still distinguishable by default.
pub fn pattern_has_timestamp_token(pattern: &str) -> bool {
    ["{date}", "{time}", "{datetime}"]
        .iter()
        .any(|token| pattern.contains(token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_now() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 11, 13, 14, 32, 8)
            .single()
            .unwrap()
    }

    #[test]
    fn default_pattern_matches_spec_example() {
        let name = render_pattern(DEFAULT_PATTERN, &fixed_now(), "region");
        assert_eq!(name, "Screenshot_2026-11-13_14-32-08");
    }

    #[test]
    fn tokens_expand() {
        let name = render_pattern("shot-{date}-{time}-{mode}", &fixed_now(), "window");
        assert_eq!(name, "shot-2026-11-13-14-32-08-window");
        let dt = render_pattern("x_{datetime}", &fixed_now(), "region");
        assert_eq!(dt, "x_2026-11-13_14-32-08");
    }

    #[test]
    fn dangerous_characters_are_removed() {
        assert_eq!(sanitize_component("../../etc/passwd"), "etc-passwd");
        assert_eq!(sanitize_component("a/b:c"), "a-b-c");
        assert_eq!(sanitize_component("   "), "Screenshot");
        assert_eq!(sanitize_component(".."), "Screenshot");
        assert_eq!(sanitize_component("tab\there"), "tab here");
        assert_eq!(sanitize_component("many   spaces"), "many spaces");
    }

    #[test]
    fn unique_file_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let (f1, p1) = create_unique_file(dir.path(), "Screenshot_x", "png").unwrap();
        drop(f1);
        let (f2, p2) = create_unique_file(dir.path(), "Screenshot_x", "png").unwrap();
        drop(f2);
        assert_eq!(p1.file_name().unwrap(), "Screenshot_x.png");
        assert_eq!(p2.file_name().unwrap(), "Screenshot_x_1.png");
        assert_ne!(p1, p2);
    }

    #[test]
    fn timestamp_token_detection() {
        assert!(pattern_has_timestamp_token("{date}"));
        assert!(!pattern_has_timestamp_token("fixed-name"));
    }
}
