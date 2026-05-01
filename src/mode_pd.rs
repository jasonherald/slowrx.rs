//! PD-family mode decoder.
//!
//! PD modes encode each radio "frame" as four channels — Y(odd line),
//! Cr (shared), Cb (shared), Y(even line) — producing two image rows
//! per radio frame with chroma subsampling between them.
//!
//! Translated from slowrx's `video.c` (Oona Räisänen, ISC License).
//! Channel layout: video.c lines 81-93. YUV→RGB matrix: video.c lines
//! 446-451. See `NOTICE.md` for full attribution.

/// Map a demodulated FM frequency (Hz) to an 8-bit luminance value.
///
/// SSTV video lives in 1500–2300 Hz: 1500 Hz = black (0), 2300 Hz = white (255).
/// Linear scaling: `lum = (freq - 1500) / (2300 - 1500) * 255`.
/// Out-of-band frequencies are clamped.
///
/// Translated from slowrx's `video.c:406`:
/// `StoredLum[SampleNum] = clip((Freq - 1500) / 3.1372549);`
/// where `3.1372549 = (2300 - 1500) / 255`.
#[must_use]
#[allow(dead_code)] // Consumed by the upcoming PD line decoder (Task 2.3+).
pub(crate) fn freq_to_luminance(freq_hz: f64) -> u8 {
    let v = (freq_hz - 1500.0) / 3.137_254_9;
    // Truncation-via-`as` matches slowrx's clip() semantics
    // (slowrx uses `(unsigned char)a` which truncates the fractional part).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lum = v.clamp(0.0, 255.0) as u8;
    lum
}

/// Convert a single PD-family `[Y, Cr, Cb]` triple to `[R, G, B]`.
///
/// Translated from slowrx's `video.c:447-450`:
/// ```text
/// R = clip((100*Y + 140*Cr - 17850) / 100)
/// G = clip((100*Y -  71*Cr -  33*Cb + 13260) / 100)
/// B = clip((100*Y + 178*Cb - 22695) / 100)
/// ```
#[must_use]
#[allow(dead_code)] // Consumed by the upcoming PD line decoder (Task 2.3+).
pub(crate) fn ycbcr_to_rgb(y: u8, cr: u8, cb: u8) -> [u8; 3] {
    let yi = i32::from(y);
    let cri = i32::from(cr);
    let cbi = i32::from(cb);
    // i32 multiplications: max magnitude is 255 * 178 = 45_390, well within i32.
    // Integer division truncates toward zero (matches slowrx's `(int)` cast).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let r = ((100 * yi + 140 * cri - 17_850) / 100).clamp(0, 255) as u8;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let g = ((100 * yi - 71 * cri - 33 * cbi + 13_260) / 100).clamp(0, 255) as u8;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let b = ((100 * yi + 178 * cbi - 22_695) / 100).clamp(0, 255) as u8;
    [r, g, b]
}

#[cfg(test)]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
mod tests {
    use super::*;

    #[test]
    fn freq_1500_is_black() {
        assert_eq!(freq_to_luminance(1500.0), 0);
    }

    #[test]
    fn freq_2300_is_white() {
        assert_eq!(freq_to_luminance(2300.0), 255);
    }

    #[test]
    fn freq_below_band_clamps_to_zero() {
        assert_eq!(freq_to_luminance(1000.0), 0);
        assert_eq!(freq_to_luminance(0.0), 0);
    }

    #[test]
    fn freq_above_band_clamps_to_max() {
        assert_eq!(freq_to_luminance(3000.0), 255);
    }

    #[test]
    fn freq_midband_is_midgrey() {
        // 1900 Hz is exactly halfway → ~127.5 → 127 after truncation
        let v = freq_to_luminance(1900.0);
        assert!(
            (i32::from(v) - 127).abs() <= 1,
            "midband should be ~127, got {v}"
        );
    }

    #[test]
    fn ycbcr_neutral_grey_is_grey() {
        // Y=128, Cr=128, Cb=128 → grey (no chroma offset)
        // R = (100*128 + 140*128 - 17850)/100 = (12800 + 17920 - 17850)/100 = 128.7 → 128
        // G = (100*128 -  71*128 -  33*128 + 13260)/100 = (12800-9088-4224+13260)/100 = 127.48 → 127
        // B = (100*128 + 178*128 - 22695)/100 = (12800+22784-22695)/100 = 128.89 → 128
        let rgb = ycbcr_to_rgb(128, 128, 128);
        for ch in &rgb {
            assert!((i32::from(*ch) - 128).abs() <= 2, "got {rgb:?}");
        }
    }

    #[test]
    fn ycbcr_pure_red() {
        // Roughly: max Cr, mid Y, mid Cb → strong red.
        let rgb = ycbcr_to_rgb(76, 255, 85);
        assert!(rgb[0] > 200, "red channel should dominate, got {rgb:?}");
        assert!(rgb[2] < 100);
    }
}
