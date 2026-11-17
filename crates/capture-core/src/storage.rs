//! Image encoding and collision-safe saving.

use crate::error::{CaptureError, Result};
use crate::image::RawImage;
use crate::naming::{create_unique_file, render_pattern};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
        }
    }

    /// Never trusts the user's format string; anything unknown becomes PNG.
    pub fn from_setting(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Self::Jpeg,
            _ => Self::Png,
        }
    }
}

/// Result of a successful save, everything the UI needs for its confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavedImage {
    pub path: PathBuf,
    pub filename: String,
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

/// Create the folder if needed and prove that we can write into it.
///
/// A folder that exists but is read-only is reported before capturing, so the
/// user does not lose a screenshot they already framed.
pub fn ensure_writable_dir(dir: &Path) -> Result<()> {
    if dir.as_os_str().is_empty() {
        return Err(CaptureError::Settings(
            "no output folder is configured".into(),
        ));
    }
    std::fs::create_dir_all(dir).map_err(|source| CaptureError::OutputDirectory {
        path: dir.to_path_buf(),
        source,
    })?;
    let meta = std::fs::metadata(dir).map_err(|source| CaptureError::OutputDirectory {
        path: dir.to_path_buf(),
        source,
    })?;
    if !meta.is_dir() {
        return Err(CaptureError::OutputDirectory {
            path: dir.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "the output path is not a folder",
            ),
        });
    }
    let probe = dir.join(format!(
        ".write-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(source) => Err(CaptureError::OutputDirectory {
            path: dir.to_path_buf(),
            source,
        }),
    }
}

/// Encode `image` and write it into `dir` using the configured pattern.
///
/// Filenames are allocated atomically, so a second capture inside the same
/// second becomes `..._1.png` instead of overwriting the first. A short retry
/// covers transient file-system hiccups (a momentarily busy folder, an
/// interrupted write) without ever reusing the first file name.
pub fn save_image(
    image: &RawImage,
    dir: &Path,
    pattern: &str,
    format: ImageFormat,
    jpeg_quality: u8,
    captured_at: &DateTime<Local>,
    mode_label: &str,
) -> Result<SavedImage> {
    ensure_writable_dir(dir)?;
    let encoded = encode(image, format, jpeg_quality)?;
    let base = render_pattern(pattern, captured_at, mode_label);
    let (path, bytes) =
        write_with_retry(dir, &base, format.extension(), &encoded).map_err(|source| {
            CaptureError::Save {
                path: dir.join(format!("{base}.{}", format.extension())),
                source,
            }
        })?;
    Ok(SavedImage {
        filename: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| base.clone()),
        path,
        format,
        width: image.width,
        height: image.height,
        bytes,
    })
}

/// How many times one save is attempted before the error is reported.
const SAVE_ATTEMPTS: u32 = 3;

fn write_with_retry(
    dir: &Path,
    base: &str,
    extension: &str,
    encoded: &[u8],
) -> std::io::Result<(PathBuf, u64)> {
    let mut last_error = None;
    for attempt in 0..SAVE_ATTEMPTS {
        match write_file(dir, base, extension, encoded) {
            Ok(saved) => return Ok(saved),
            Err(source) => {
                last_error = Some(source);
                if attempt + 1 < SAVE_ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_millis(
                        20 * u64::from(attempt + 1),
                    ));
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("could not save the screenshot")))
}

fn write_file(
    dir: &Path,
    base: &str,
    extension: &str,
    encoded: &[u8],
) -> std::io::Result<(PathBuf, u64)> {
    let (mut file, path) = create_unique_file(dir, base, extension)?;
    let written = file.write_all(encoded).and_then(|_| file.flush());
    drop(file);

    if let Err(source) = written {
        // Do not leave a half-written PNG under a valid-looking name; the
        // retry allocates a fresh collision-free name.
        let _ = std::fs::remove_file(&path);
        return Err(source);
    }

    let bytes = std::fs::metadata(&path)
        .map(|meta| meta.len())
        .unwrap_or(encoded.len() as u64);
    Ok((path, bytes))
}

/// Encode an image in memory. Split out so it can be unit tested without disk.
pub fn encode(image: &RawImage, format: ImageFormat, jpeg_quality: u8) -> Result<Vec<u8>> {
    let rgba = image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone())
        .ok_or_else(|| {
            CaptureError::Encode("image buffer size does not match dimensions".into())
        })?;
    let mut out = Vec::new();
    match format {
        ImageFormat::Png => {
            let encoder = image::codecs::png::PngEncoder::new(&mut out);
            image::ImageEncoder::write_image(
                encoder,
                rgba.as_raw(),
                image.width,
                image.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|err| CaptureError::Encode(err.to_string()))?;
        }
        ImageFormat::Jpeg => {
            let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut out,
                jpeg_quality.clamp(1, 100),
            );
            image::ImageEncoder::write_image(
                encoder,
                rgb.as_raw(),
                image.width,
                image.height,
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|err| CaptureError::Encode(err.to_string()))?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    fn image(w: u32, h: u32, color: [u8; 4]) -> RawImage {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            rgba.extend_from_slice(&color);
        }
        RawImage::new(w, h, rgba).unwrap()
    }

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 13, 14, 32, 8).unwrap()
    }

    #[test]
    fn png_is_a_real_png_with_expected_size() {
        let dir = tempfile::tempdir().unwrap();
        let saved = save_image(
            &image(8, 6, [255, 0, 0, 255]),
            dir.path(),
            "Screenshot_{date}_{time}",
            ImageFormat::Png,
            90,
            &now(),
            "region",
        )
        .unwrap();
        assert_eq!(saved.filename, "Screenshot_2026-11-13_14-32-08.png");
        assert_eq!((saved.width, saved.height), (8, 6));
        let bytes = std::fs::read(&saved.path).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.width(), 8);
        assert_eq!(decoded.height(), 6);
        assert!(saved.bytes >= bytes.len() as u64);
    }

    #[test]
    fn jpeg_uses_jpg_extension_and_is_decodable() {
        let dir = tempfile::tempdir().unwrap();
        let saved = save_image(
            &image(20, 10, [10, 120, 200, 255]),
            dir.path(),
            "shot_{mode}",
            ImageFormat::Jpeg,
            80,
            &now(),
            "window",
        )
        .unwrap();
        assert_eq!(saved.filename, "shot_window.jpg");
        let bytes = std::fs::read(&saved.path).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
        assert!(image::load_from_memory(&bytes).is_ok());
    }

    #[test]
    fn second_capture_in_same_second_gets_a_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let a = save_image(
            &image(2, 2, [0, 0, 0, 255]),
            dir.path(),
            "Screenshot_{date}_{time}",
            ImageFormat::Png,
            90,
            &now(),
            "region",
        )
        .unwrap();
        let b = save_image(
            &image(2, 2, [1, 1, 1, 255]),
            dir.path(),
            "Screenshot_{date}_{time}",
            ImageFormat::Png,
            90,
            &now(),
            "region",
        )
        .unwrap();
        assert_eq!(a.filename, "Screenshot_2026-11-13_14-32-08.png");
        assert_eq!(b.filename, "Screenshot_2026-11-13_14-32-08_1.png");
    }

    #[test]
    fn missing_output_folder_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        ensure_writable_dir(&nested).unwrap();
        assert!(nested.is_dir());
    }

    #[test]
    fn read_only_folder_reports_output_directory_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let err = ensure_writable_dir(&locked).unwrap_err();
        assert!(matches!(err, CaptureError::OutputDirectory { .. }));
        assert!(err.user_message().contains("Cannot write"));
    }

    #[test]
    fn format_parsing_defaults_to_png() {
        assert_eq!(ImageFormat::from_setting("jpeg"), ImageFormat::Jpeg);
        assert_eq!(ImageFormat::from_setting("PNG"), ImageFormat::Png);
        assert_eq!(ImageFormat::from_setting("bmp"), ImageFormat::Png);
    }
}
