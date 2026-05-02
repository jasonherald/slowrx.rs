//! PD-family mode decoder.
//!
//! PD modes encode each radio "frame" as four channels — Y(odd line),
//! Cr (shared), Cb (shared), Y(even line) — producing two image rows
//! per radio frame with chroma subsampling between them.
//!
//! Translated from slowrx's `video.c` (Oona Räisänen, ISC License).
//! Channel layout: video.c lines 81-93. YUV→RGB matrix: video.c lines
//! 446-451. See `NOTICE.md` for full attribution.

use rustfft::{num_complex::Complex, FftPlanner};

/// Map a demodulated FM frequency (Hz) to an 8-bit luminance value.
///
/// SSTV video lives in 1500–2300 Hz: 1500 Hz = black (0), 2300 Hz = white (255).
/// Linear scaling: `lum = (freq - 1500) / (2300 - 1500) * 255`.
/// Out-of-band frequencies are clamped. `hedr_shift_hz` shifts the band
/// to compensate for radio mistuning detected at VIS time:
/// `lum = (freq - 1500 - hedr_shift_hz) / 3.1372549`.
///
/// Translated from slowrx's `video.c:406`:
/// `StoredLum[SampleNum] = clip((Freq - 1500 - HedrShift) / 3.1372549);`
/// where `3.1372549 = (2300 - 1500) / 255`.
#[must_use]
pub(crate) fn freq_to_luminance(freq_hz: f64, hedr_shift_hz: f64) -> u8 {
    let v = (freq_hz - 1500.0 - hedr_shift_hz) / 3.137_254_9;
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
#[doc(hidden)]
pub fn ycbcr_to_rgb(y: u8, cr: u8, cb: u8) -> [u8; 3] {
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

/// FFT length used for per-pixel demod. Matches slowrx's bin spacing:
/// 256/11025 Hz = 43.07 Hz/bin, equal to slowrx's 1024/44100 Hz.
pub(crate) const FFT_LEN: usize = 256;

/// Hann window of length [`FFT_LEN`], precomputed once per decoder invocation.
#[allow(clippy::cast_precision_loss)]
fn hann_window() -> Vec<f32> {
    (0..FFT_LEN)
        .map(|i| {
            let m = (FFT_LEN - 1) as f32;
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * (i as f32) / m).cos())
        })
        .collect()
}

/// Per-pixel demod context: holds an FFT plan + reusable buffers.
/// Construct once per decoder; reuse for many `pixel_freq` calls.
pub(crate) struct PdDemod {
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    hann: Vec<f32>,
    fft_buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl PdDemod {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_LEN);
        let scratch_len = fft.get_inplace_scratch_len();
        Self {
            fft,
            hann: hann_window(),
            fft_buf: vec![Complex { re: 0.0, im: 0.0 }; FFT_LEN],
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len.max(FFT_LEN)],
        }
    }

    /// Estimate the dominant tone frequency in a 256-sample window centered
    /// near `center_sample` of the working-rate audio buffer. The actual
    /// window is `audio[center_sample - FFT_LEN/2 .. center_sample + FFT_LEN/2]`,
    /// clamped at audio boundaries.
    ///
    /// `hedr_shift_hz` shifts the peak-search range from 1500..2300 Hz to
    /// `(1500 + hedr_shift) .. (2300 + hedr_shift)` so a mistuned radio's
    /// pixel band lines up with the search window. Translated from slowrx
    /// `video.c:382` where the search starts at `GetBin(1500 + HedrShift)`.
    ///
    /// Returns the estimated frequency in Hz. Uses Gaussian-log peak
    /// interpolation matching slowrx `video.c:391-394`.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    pub fn pixel_freq(&mut self, audio: &[f32], center_sample: i64, hedr_shift_hz: f64) -> f64 {
        // Fill the reusable buffer in-place — avoids a per-call Vec allocation.
        let half = (FFT_LEN as i64) / 2;
        for i in 0..FFT_LEN {
            let idx = center_sample - half + i as i64;
            let s = if idx >= 0 && (idx as usize) < audio.len() {
                audio[idx as usize]
            } else {
                0.0
            };
            self.fft_buf[i] = Complex {
                re: s * self.hann[i],
                im: 0.0,
            };
        }

        self.fft
            .process_with_scratch(&mut self.fft_buf, &mut self.scratch[..]);

        // Search peak in bins corresponding to (1500+HedrShift)..(2300+HedrShift) Hz.
        let bin_for = |hz: f64| -> usize {
            (hz * (FFT_LEN as f64) / f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ)).round()
                as usize
        };
        let lo = bin_for(1500.0 + hedr_shift_hz).saturating_sub(1).max(1);
        let hi = bin_for(2300.0 + hedr_shift_hz)
            .saturating_add(1)
            .min(FFT_LEN / 2 - 1);

        let power = |c: Complex<f32>| -> f64 {
            let r = f64::from(c.re);
            let i = f64::from(c.im);
            r * r + i * i
        };

        let mut max_bin = lo;
        let mut max_p = power(self.fft_buf[lo]);
        for (k, &c) in self.fft_buf.iter().enumerate().take(hi + 1).skip(lo + 1) {
            let p = power(c);
            if p > max_p {
                max_p = p;
                max_bin = k;
            }
        }

        // Gaussian-log peak interpolation (slowrx video.c:391-394).
        // Freq_bin = MaxBin + log(P[k+1]/P[k-1]) / (2 * log(P[k]^2 / (P[k+1] * P[k-1])))
        let p_prev = power(self.fft_buf[max_bin - 1]);
        let p_curr = max_p;
        let p_next = power(self.fft_buf[max_bin + 1]);

        // If any neighbor power is non-positive, skip interpolation
        // (log of zero blows up). slowrx falls back to a clipped centre.
        let interp_ok = p_prev > 0.0 && p_curr > 0.0 && p_next > 0.0;
        let freq_bin = if interp_ok {
            let num = (p_next / p_prev).ln();
            let denom = 2.0 * (p_curr * p_curr / (p_next * p_prev)).ln();
            if denom.abs() > 1e-12 {
                (max_bin as f64) + num / denom
            } else {
                max_bin as f64
            }
        } else {
            max_bin as f64
        };

        freq_bin * f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ) / (FFT_LEN as f64)
    }
}

/// Decode one PD radio frame (one `Y(odd)/Cr/Cb/Y(even)` sequence) into two
/// image rows of `image`. Translated from slowrx `video.c` lines 81-93
/// (channel layout) and `video.c` lines 411-450 (per-pixel demod + chroma
/// combine). `pair_start_sample` is where this pair's sync pulse begins
/// inside `audio`; `rate_hz` is the slant-corrected rate from
/// [`crate::sync::find_sync`]. We extract a per-channel slice so the
/// per-pixel FFT window does not bleed across channel edges.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_arguments
)]
pub(crate) fn decode_pd_line_pair(
    spec: crate::modespec::ModeSpec,
    pair_index: u32,
    audio: &[f32],
    pair_start_sample: i64,
    rate_hz: f64,
    image: &mut crate::image::SstvImage,
    demod: &mut PdDemod,
    hedr_shift_hz: f64,
) {
    let sync_secs = spec.sync_seconds;
    let porch_secs = spec.porch_seconds;
    let pixel_secs = spec.pixel_seconds;
    let width = spec.line_pixels;

    // PD channel time offsets (seconds from start of line pair):
    // Y(odd) → Cr → Cb → Y(even). slowrx video.c:81-93.
    let chan_starts_sec = [
        sync_secs + porch_secs,                                       // Y(odd)
        sync_secs + porch_secs + f64::from(width) * pixel_secs,       // Cr
        sync_secs + porch_secs + 2.0 * f64::from(width) * pixel_secs, // Cb
        sync_secs + porch_secs + 3.0 * f64::from(width) * pixel_secs, // Y(even)
    ];

    let row0 = pair_index * 2;
    let row1 = row0 + 1;

    let mut y_odd = vec![0_u8; width as usize];
    let mut cr = vec![0_u8; width as usize];
    let mut cb = vec![0_u8; width as usize];
    let mut y_even = vec![0_u8; width as usize];

    for (chan_idx, channel_buf) in [&mut y_odd, &mut cr, &mut cb, &mut y_even]
        .iter_mut()
        .enumerate()
    {
        let start_sec = chan_starts_sec[chan_idx];
        let end_sec = start_sec + f64::from(width) * pixel_secs;
        // Channel start in audio (absolute sample index).
        let chan_start_abs = pair_start_sample + (start_sec * rate_hz).round() as i64;
        let chan_end_abs = pair_start_sample + (end_sec * rate_hz).round() as i64;

        // Extract just this channel's samples (zero-pad if the input window
        // doesn't fully cover the channel — happens at line-pair end-of-input).
        let chan_len = (chan_end_abs - chan_start_abs).max(0) as usize;
        let mut chan_samples = vec![0.0_f32; chan_len];
        for (i, dst) in chan_samples.iter_mut().enumerate() {
            let src_idx = chan_start_abs + i as i64;
            if src_idx >= 0 && (src_idx as usize) < audio.len() {
                *dst = audio[src_idx as usize];
            }
        }

        for x in 0..width as usize {
            // Center sample relative to the channel slice.
            let center_sec_rel = (x as f64 + 0.5) * pixel_secs;
            let center_sample_rel = (center_sec_rel * rate_hz).round() as i64;
            let freq = demod.pixel_freq(&chan_samples, center_sample_rel, hedr_shift_hz);
            channel_buf[x] = freq_to_luminance(freq, hedr_shift_hz);
        }
    }

    for x in 0..width as usize {
        let rgb_odd = ycbcr_to_rgb(y_odd[x], cr[x], cb[x]);
        let rgb_even = ycbcr_to_rgb(y_even[x], cr[x], cb[x]);
        image.put_pixel(x as u32, row0, rgb_odd);
        image.put_pixel(x as u32, row1, rgb_even);
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
pub mod test_encoder {
    //! Synthetic PD encoder for round-trip testing. Produces continuous-phase
    //! FM audio matching the encoder side of the SSTV protocol.

    use crate::modespec::SstvMode;
    use crate::resample::WORKING_SAMPLE_RATE_HZ;
    use std::f64::consts::PI;

    const SYNC_HZ: f64 = 1200.0;
    const PORCH_HZ: f64 = 1500.0;
    const BLACK_HZ: f64 = 1500.0;
    const WHITE_HZ: f64 = 2300.0;

    fn lum_to_freq(lum: u8) -> f64 {
        BLACK_HZ + (WHITE_HZ - BLACK_HZ) * f64::from(lum) / 255.0
    }

    /// Encoder helper: emit samples up to sample index `target_n` (exclusive)
    /// at frequency `freq_hz`, advancing both the running sample count `n`
    /// and the running phase. Using cumulative sample targets prevents the
    /// per-tone rounding error that would otherwise compound over the line
    /// (640 pixels × ~0.094 sample/pixel rounding error per pixel adds up to
    /// 60+ samples per line at PD120, breaking decoder line alignment).
    fn fill_to(out: &mut Vec<f32>, freq_hz: f64, target_n: usize, phase: &mut f64) {
        let dphi = 2.0 * PI * freq_hz / f64::from(WORKING_SAMPLE_RATE_HZ);
        while out.len() < target_n {
            out.push(phase.sin() as f32);
            *phase += dphi;
            if *phase > 2.0 * PI {
                *phase -= 2.0 * PI;
            }
        }
    }

    /// Encode an image as PD120 / PD180 audio. `ycrcb` is row-major
    /// `[Y, Cr, Cb]` triples of length `width * height`. Pairs of rows
    /// share averaged chroma, matching how the decoder will recover them.
    #[must_use]
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn encode_pd(mode: SstvMode, ycrcb: &[[u8; 3]]) -> Vec<f32> {
        assert!(matches!(mode, SstvMode::Pd120 | SstvMode::Pd180));
        let spec = crate::modespec::for_mode(mode);
        let w = spec.line_pixels;
        let h = spec.image_lines;
        assert_eq!(ycrcb.len() as u32, w * h);
        assert_eq!(h % 2, 0);

        let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
        let mut out = Vec::new();
        let mut phase = 0.0_f64;

        // Cumulative time tracker (seconds). Targets are computed as
        // `(running_t * sr).round()` so per-event rounding doesn't drift.
        let mut t = 0.0_f64;
        let advance = |t: &mut f64, secs: f64| -> usize {
            *t += secs;
            (*t * sr).round() as usize
        };

        for y_pair in 0..h / 2 {
            fill_to(
                &mut out,
                SYNC_HZ,
                advance(&mut t, spec.sync_seconds),
                &mut phase,
            );
            fill_to(
                &mut out,
                PORCH_HZ,
                advance(&mut t, spec.porch_seconds),
                &mut phase,
            );

            // Y(odd row).
            for x in 0..w {
                let lum = ycrcb[((y_pair * 2) * w + x) as usize][0];
                fill_to(
                    &mut out,
                    lum_to_freq(lum),
                    advance(&mut t, spec.pixel_seconds),
                    &mut phase,
                );
            }
            // Cr (averaged across pair).
            for x in 0..w {
                let cr_a = ycrcb[((y_pair * 2) * w + x) as usize][1];
                let cr_b = ycrcb[((y_pair * 2 + 1) * w + x) as usize][1];
                let cr = u8::midpoint(cr_a, cr_b);
                fill_to(
                    &mut out,
                    lum_to_freq(cr),
                    advance(&mut t, spec.pixel_seconds),
                    &mut phase,
                );
            }
            // Cb (averaged).
            for x in 0..w {
                let cb_a = ycrcb[((y_pair * 2) * w + x) as usize][2];
                let cb_b = ycrcb[((y_pair * 2 + 1) * w + x) as usize][2];
                let cb = u8::midpoint(cb_a, cb_b);
                fill_to(
                    &mut out,
                    lum_to_freq(cb),
                    advance(&mut t, spec.pixel_seconds),
                    &mut phase,
                );
            }
            // Y(even row).
            for x in 0..w {
                let lum = ycrcb[((y_pair * 2 + 1) * w + x) as usize][0];
                fill_to(
                    &mut out,
                    lum_to_freq(lum),
                    advance(&mut t, spec.pixel_seconds),
                    &mut phase,
                );
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
mod tests {
    use super::*;

    #[test]
    fn freq_1500_is_black() {
        assert_eq!(freq_to_luminance(1500.0, 0.0), 0);
    }

    #[test]
    fn freq_2300_is_white() {
        assert_eq!(freq_to_luminance(2300.0, 0.0), 255);
    }

    #[test]
    fn freq_below_band_clamps_to_zero() {
        assert_eq!(freq_to_luminance(1000.0, 0.0), 0);
        assert_eq!(freq_to_luminance(0.0, 0.0), 0);
    }

    #[test]
    fn freq_above_band_clamps_to_max() {
        assert_eq!(freq_to_luminance(3000.0, 0.0), 255);
    }

    #[test]
    fn freq_midband_is_midgrey() {
        // 1900 Hz is exactly halfway → ~127.5 → 127 after truncation
        let v = freq_to_luminance(1900.0, 0.0);
        assert!(
            (i32::from(v) - 127).abs() <= 1,
            "midband should be ~127, got {v}"
        );
    }

    #[test]
    fn freq_to_luminance_with_hedr_shift_scales_band() {
        // With +50 Hz HedrShift, the band shifts to 1550..2350.
        // freq=1550 should be black, freq=2350 should be white.
        assert_eq!(freq_to_luminance(1550.0, 50.0), 0);
        assert_eq!(freq_to_luminance(2350.0, 50.0), 255);
        // 1500 (would be black at zero shift) is now sub-band, still clamps to 0.
        assert_eq!(freq_to_luminance(1500.0, 50.0), 0);
        // 1950 with +50 shift is the new midband ≈ same as 1900 with 0 shift.
        let a = freq_to_luminance(1950.0, 50.0);
        let b = freq_to_luminance(1900.0, 0.0);
        assert!(
            i32::from(a).abs_diff(i32::from(b)) <= 1,
            "shifted midband {a} vs unshifted {b}"
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

    use crate::resample::WORKING_SAMPLE_RATE_HZ;
    use std::f64::consts::PI;

    fn synth_tone(freq_hz: f64, secs: f64) -> Vec<f32> {
        let n = (secs * f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize;
        (0..n)
            .map(|i| {
                let t = (i as f64) / f64::from(WORKING_SAMPLE_RATE_HZ);
                (2.0 * PI * freq_hz * t).sin() as f32
            })
            .collect()
    }

    #[test]
    fn pdfft_recovers_known_tone_within_5hz() {
        let mut d = PdDemod::new();
        // 100 ms of pure tone at 1900 Hz; ample for FFT_LEN=256.
        let audio = synth_tone(1900.0, 0.100);
        let center = (audio.len() / 2) as i64;
        let est = d.pixel_freq(&audio, center, 0.0);
        assert!((est - 1900.0).abs() < 5.0, "expected ≈1900, got {est}");
    }

    #[test]
    fn pdfft_recovers_band_edges() {
        let mut d = PdDemod::new();
        for &f in &[1500.0_f64, 1700.0, 2100.0, 2300.0] {
            let audio = synth_tone(f, 0.100);
            let center = (audio.len() / 2) as i64;
            let est = d.pixel_freq(&audio, center, 0.0);
            assert!(
                (est - f).abs() < 8.0,
                "f={f} estimate={est} (band edge precision)"
            );
        }
    }

    #[test]
    fn pdfft_returns_finite_for_silence() {
        let mut d = PdDemod::new();
        let audio = vec![0.0_f32; 1024];
        let est = d.pixel_freq(&audio, 512, 0.0);
        assert!(est.is_finite(), "got {est}");
        // Silence has no peak; the search expands by ±1 bin around the
        // 1500-2300 band, and bin width is ~43 Hz, so the fallback may
        // land within ~50 Hz of either edge.
        assert!((1450.0..=2350.0).contains(&est), "out of band: {est}");
    }

    #[test]
    fn pixel_freq_with_hedr_shift() {
        // A 1950 Hz tone with hedr_shift=+50 should yield the same luminance
        // as a 1900 Hz tone with hedr_shift=0, since the search band shifts.
        let mut d = PdDemod::new();
        let audio = synth_tone(1950.0, 0.100);
        let center = (audio.len() / 2) as i64;
        let est_shifted = d.pixel_freq(&audio, center, 50.0);
        let est_unshifted_baseline = {
            let baseline = synth_tone(1900.0, 0.100);
            d.pixel_freq(&baseline, (baseline.len() / 2) as i64, 0.0)
        };
        // Tone-frequency itself is recovered correctly; what matters is that
        // the shifted estimator finds 1950, the unshifted finds 1900, and
        // mapping each to luminance gives ≈ same value.
        assert!((est_shifted - 1950.0).abs() < 5.0, "got {est_shifted}");
        let lum_shifted = freq_to_luminance(est_shifted, 50.0);
        let lum_unshifted = freq_to_luminance(est_unshifted_baseline, 0.0);
        assert!(
            i32::from(lum_shifted).abs_diff(i32::from(lum_unshifted)) <= 2,
            "lum_shifted={lum_shifted} lum_unshifted={lum_unshifted}"
        );
    }
}
