//! In-memory image type produced by all capture backends.

use crate::geometry::Region;

/// An 8-bit RGBA image, top-down, tightly packed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl RawImage {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let expected = width as usize * height as usize * 4;
        if rgba.len() != expected {
            return None;
        }
        Some(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Copy a sub-rectangle out of this image.
    ///
    /// `region` is relative to the image origin. Returns `None` when the
    /// rectangle is empty or not fully inside the image.
    pub fn crop(&self, region: Region) -> Option<Self> {
        if region.width == 0 || region.height == 0 {
            return None;
        }
        if region.x < 0 || region.y < 0 {
            return None;
        }
        if region.right() > self.width as i32 || region.bottom() > self.height as i32 {
            return None;
        }
        let row_bytes = region.width as usize * 4;
        let mut rgba = Vec::with_capacity(row_bytes * region.height as usize);
        for row in 0..region.height as usize {
            let start = ((region.y as usize + row) * self.width as usize + region.x as usize) * 4;
            rgba.extend_from_slice(&self.rgba[start..start + row_bytes]);
        }
        Self::new(region.width, region.height, rgba)
    }
}

impl std::fmt::Display for RawImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}×{}", self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_size_mismatch() {
        assert!(RawImage::new(2, 2, vec![0; 15]).is_none());
        assert!(RawImage::new(2, 2, vec![0; 16]).is_some());
        assert!(RawImage::new(0, 0, vec![]).is_none());
    }

    #[test]
    fn crop_extracts_the_requested_pixels() {
        // 3x2 image, red channel encodes the x position.
        let mut rgba = Vec::new();
        for y in 0..2u8 {
            for x in 0..3u8 {
                rgba.extend_from_slice(&[x * 10, y, 0, 255]);
            }
        }
        let image = RawImage::new(3, 2, rgba).unwrap();
        let cropped = image.crop(Region::new(1, 1, 2, 1)).unwrap();
        assert_eq!((cropped.width, cropped.height), (2, 1));
        assert_eq!(&cropped.rgba[0..4], &[10, 1, 0, 255]);
        assert_eq!(&cropped.rgba[4..8], &[20, 1, 0, 255]);
        assert!(image.crop(Region::new(2, 0, 2, 1)).is_none());
        assert!(image.crop(Region::new(0, 0, 0, 1)).is_none());
    }
}
