//! Screen geometry helpers shared by the backends and the settings model.

use serde::{Deserialize, Serialize};

/// A rectangle in root-window (virtual desktop) coordinates.
///
/// X11 uses signed coordinates because multi-monitor layouts can place a
/// monitor at a negative offset; sizes are unsigned and never negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    /// Smallest region we are willing to save. Prevents "one pixel" files from
    /// an accidental click.
    pub const MIN_SIZE: u32 = 5;

    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn empty() -> Self {
        Self::new(0, 0, 0, 0)
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn is_usable(&self) -> bool {
        self.width >= Self::MIN_SIZE && self.height >= Self::MIN_SIZE
    }

    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }

    /// Build a normalised region from two dragged corners (any direction).
    pub fn from_corners(x1: i32, y1: i32, x2: i32, y2: i32) -> Self {
        let left = x1.min(x2);
        let top = y1.min(y2);
        let width = (x1.max(x2) - left) as u32;
        let height = (y1.max(y2) - top) as u32;
        Self::new(left, top, width, height)
    }

    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn intersect(&self, other: &Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= left || bottom <= top {
            return None;
        }
        Some(Self::new(
            left,
            top,
            (right - left) as u32,
            (bottom - top) as u32,
        ))
    }

    /// Clamp this region into `bounds`. Returns `None` when nothing of the
    /// region remains on screen (for example a monitor was unplugged).
    pub fn clamp_to(&self, bounds: &Self) -> Option<Self> {
        let clipped = self.intersect(bounds)?;
        if clipped.is_empty() {
            None
        } else {
            Some(clipped)
        }
    }

    /// True when both rectangles share any area.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.intersect(other).is_some()
    }

    pub fn to_x11_rect(&self) -> (i16, i16, u16, u16) {
        (
            self.x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            self.y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            self.width.min(u16::MAX as u32) as u16,
            self.height.min(u16::MAX as u32) as u16,
        )
    }
}

impl Default for Region {
    fn default() -> Self {
        Self::empty()
    }
}

/// Human readable description used in status lines, e.g. `1920×1080 region`.
impl std::fmt::Display for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}×{} at {},{}", self.width, self.height, self.x, self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_corners_normalises_reverse_drags() {
        let a = Region::from_corners(300, 200, 100, 50);
        let b = Region::from_corners(100, 50, 300, 200);
        assert_eq!(a, b);
        assert_eq!(a, Region::new(100, 50, 200, 150));
    }

    #[test]
    fn contains_point_is_half_open() {
        let r = Region::new(10, 10, 5, 5);
        assert!(r.contains_point(10, 10));
        assert!(r.contains_point(14, 14));
        assert!(!r.contains_point(15, 15));
        assert!(!r.contains_point(9, 10));
    }

    #[test]
    fn clamp_keeps_overlap_and_drops_off_screen() {
        let bounds = Region::new(0, 0, 1920, 1080);
        let r = Region::new(-100, 900, 400, 400);
        assert_eq!(r.clamp_to(&bounds), Some(Region::new(0, 900, 300, 180)));
        let gone = Region::new(2500, 10, 100, 100);
        assert_eq!(gone.clamp_to(&bounds), None);
    }

    #[test]
    fn min_size_guard() {
        assert!(!Region::new(0, 0, 4, 100).is_usable());
        assert!(Region::new(0, 0, 5, 5).is_usable());
    }

    #[test]
    fn overlap() {
        let a = Region::new(0, 0, 10, 10);
        assert!(a.overlaps(&Region::new(9, 9, 10, 10)));
        assert!(!a.overlaps(&Region::new(10, 10, 1, 1)));
    }
}
