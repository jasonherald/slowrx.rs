//! Pixel buffer for a decoded SSTV image.
//!
//! Pure data — no PNG / file I/O dependency. Callers serialize via
//! their preferred crate (`image`, `png`, etc.).

use crate::modespec::SstvMode;

/// Row-major RGB pixel buffer for a decoded SSTV image.
///
/// Pixels are stored as `[r, g, b]` triples in row-major order.
/// Out-of-bounds [`Self::pixel`] / [`Self::put_pixel`] calls return
/// `None` / silently no-op respectively.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SstvImage {
    /// The mode this image was decoded as.
    pub mode: SstvMode,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Row-major `[r, g, b]` pixel data of length `width * height`.
    pub pixels: Vec<[u8; 3]>,
}

impl SstvImage {
    /// Construct a black-filled image for the given dimensions.
    ///
    /// # Panics
    /// Panics if `width * height` overflows `usize` (unreachable for valid SSTV dims).
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn new(mode: SstvMode, width: u32, height: u32) -> Self {
        let n = (width as usize)
            .checked_mul(height as usize)
            .expect("width * height overflowed usize");
        Self {
            mode,
            width,
            height,
            pixels: vec![[0; 3]; n],
        }
    }

    /// Read a single pixel. Returns `None` if `(x, y)` is out of bounds.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        self.index(x, y).map(|i| self.pixels[i])
    }

    /// Write a single pixel. Silently no-ops on out-of-bounds coordinates.
    pub fn put_pixel(&mut self, x: u32, y: u32, rgb: [u8; 3]) {
        if let Some(i) = self.index(x, y) {
            self.pixels[i] = rgb;
        }
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.width && y < self.height {
            Some((y as usize) * (self.width as usize) + (x as usize))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_image_is_black() {
        let img = SstvImage::new(SstvMode::Pd120, 4, 3);
        assert_eq!(img.width, 4);
        assert_eq!(img.height, 3);
        assert_eq!(img.pixels.len(), 12);
        assert!(img.pixels.iter().all(|p| *p == [0, 0, 0]));
    }

    #[test]
    fn put_and_read_pixel() {
        let mut img = SstvImage::new(SstvMode::Pd180, 4, 3);
        img.put_pixel(2, 1, [10, 20, 30]);
        assert_eq!(img.pixel(2, 1), Some([10, 20, 30]));
        assert_eq!(img.pixel(0, 0), Some([0, 0, 0]));
    }

    #[test]
    fn oob_pixel_returns_none() {
        let img = SstvImage::new(SstvMode::Pd120, 4, 3);
        assert_eq!(img.pixel(4, 0), None);
        assert_eq!(img.pixel(0, 3), None);
    }

    #[test]
    fn oob_put_silently_noops() {
        let mut img = SstvImage::new(SstvMode::Pd120, 4, 3);
        img.put_pixel(99, 99, [255, 255, 255]);
        assert!(img.pixels.iter().all(|p| *p == [0, 0, 0]));
    }
}
