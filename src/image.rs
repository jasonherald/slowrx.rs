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
#[derive(Clone, Debug, Eq, PartialEq)]
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
        self.index(x, y).and_then(|i| self.pixels.get(i).copied())
    }

    /// Write a single pixel. Silently no-ops on out-of-bounds coordinates.
    pub fn put_pixel(&mut self, x: u32, y: u32, rgb: [u8; 3]) {
        if let Some(px) = self.index(x, y).and_then(|i| self.pixels.get_mut(i)) {
            *px = rgb;
        }
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.width && y < self.height {
            let row = (y as usize).checked_mul(self.width as usize)?;
            row.checked_add(x as usize)
        } else {
            None
        }
    }
}

/// How clearly an image's line-sync pulses showed up while it was decoded:
/// `0.0` means no line structure at all, about `1.0` means a sync pulse on
/// every line. Carried on [`crate::SstvEvent::ImageComplete`].
///
/// On weak signals a VIS can be followed by noise (the transmission fades
/// or ends, or a VIS matched by chance in noise), and the decoder still
/// produces a full-size "image" of that noise. [`Self::looks_like_noise`]
/// flags those, so callers can drop them instead of saving junk. Measured
/// on real ISS passes: legible images 0.43–0.95 (clean ~0.95), noise
/// images 0.03–0.10.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct SyncQuality(f32);

impl SyncQuality {
    /// Scores below this look like noise rather than an image. Sits in the
    /// gap between the classes measured on real passes (noise ≤ 0.10,
    /// legible ≥ 0.43).
    pub const NOISE_THRESHOLD: f32 = 0.2;

    /// Wrap a raw score, clamped to `[0, 1]` (`NaN` becomes `0`).
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self(if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, 1.0)
        })
    }

    /// The score in `[0, 1]`.
    #[must_use]
    pub fn value(self) -> f32 {
        self.0
    }

    /// Whether the image is more likely noise than a picture
    /// (score below [`Self::NOISE_THRESHOLD`]).
    #[must_use]
    pub fn looks_like_noise(self) -> bool {
        self.0 < Self::NOISE_THRESHOLD
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

    #[test]
    fn metadata_buffer_desync_does_not_panic() {
        // Caller mutated `pixels` to a length shorter than width*height
        // (a misuse, but should degrade gracefully).
        let mut img = SstvImage::new(SstvMode::Pd120, 4, 3);
        img.pixels.truncate(2); // width*height was 12, now 2; pixel() returns None past the buffer.
        assert_eq!(img.pixel(3, 2), None);
        assert_eq!(img.pixel(0, 0), Some([0, 0, 0]));
        // put_pixel() must silently no-op on indices past the buffer.
        img.put_pixel(3, 2, [1, 2, 3]);
        assert_eq!(img.pixels.len(), 2); // no growth
    }

    #[test]
    fn sync_quality_clamps_and_classifies() {
        assert!((SyncQuality::new(1.7).value() - 1.0).abs() < f32::EPSILON);
        assert!(SyncQuality::new(-0.3).value().abs() < f32::EPSILON);
        assert!(SyncQuality::new(f32::NAN).value().abs() < f32::EPSILON);
        assert!(SyncQuality::new(0.05).looks_like_noise());
        assert!(!SyncQuality::new(SyncQuality::NOISE_THRESHOLD).looks_like_noise());
        assert!(!SyncQuality::new(0.95).looks_like_noise());
    }
}
