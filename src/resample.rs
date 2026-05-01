//! Internal rational resampler: caller's audio rate → 11025 Hz working rate.
//!
//! Hand-rolled 64-tap Hann-windowed-sinc polyphase FIR. We picked this over
//! `rubato` for zero extra deps and a small file. Quality target is "audible
//! loss < 0.1 dB across SSTV-relevant frequencies (1500-2300 Hz)" — easily
//! met at typical input rates (44.1k, 48k). Translated in spirit from
//! slowrx's implicit resampling inside `pcm.c`'s 44.1 kHz read loop.

use crate::error::{Error, Result};

/// Working sample rate the decoder operates at internally. Any caller
/// sample rate is resampled to this before processing.
pub const WORKING_SAMPLE_RATE_HZ: u32 = 11_025;

/// Maximum supported caller input sample rate.
pub const MAX_INPUT_SAMPLE_RATE_HZ: u32 = 192_000;

/// Number of FIR taps. Higher = sharper transition + more CPU.
/// 64 is the sweet spot at our quality target.
const FIR_TAPS: usize = 64;

/// Polyphase FIR resampler. Stateful — holds a tail buffer to avoid
/// glitches across `process` calls.
pub struct Resampler {
    input_rate: u32,
    /// `input_rate / WORKING_SAMPLE_RATE_HZ`, expressed as a stride.
    stride: f64,
    /// Position into the input buffer (fractional, accumulates across calls).
    phase: f64,
    /// Carry-over input samples from the previous call.
    tail: Vec<f32>,
    /// Cutoff frequency normalized to input rate (taps spaced at `1/input_rate`).
    cutoff_norm: f64,
}

/// Cutoff frequency (Hz) for the resampler, derived from the input rate.
/// Min of (`input_rate/2`, `working_rate/2`) × 0.45, hard-capped at 4500 Hz.
fn cutoff_hz(input_rate: u32) -> f64 {
    (f64::from(input_rate.min(WORKING_SAMPLE_RATE_HZ)) * 0.45).min(4500.0)
}

/// Compute one FIR tap value for a given tap index and fractional phase.
/// `tap_index` is in 0..`FIR_TAPS`. `frac` is in [0, 1) — the sub-sample
/// offset of the output sample's center from the integer input grid.
/// `fc` is the cutoff normalized to input rate (`cutoff_hz / input_rate`).
///
/// Sinc shifts with `frac`; Hann window stays anchored to the tap grid.
/// This is the standard windowed-sinc fractional-delay formulation —
/// see e.g. Smith, "Digital Audio Resampling Home Page" (CCRMA, 2002).
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn fir_tap(tap_index: usize, frac: f64, fc: f64) -> f32 {
    let m = FIR_TAPS as f64;
    let n = (tap_index as f64) - (m - 1.0) / 2.0 - frac;
    let sinc = if n.abs() < 1e-12 {
        2.0 * fc
    } else {
        (2.0 * std::f64::consts::PI * fc * n).sin() / (std::f64::consts::PI * n)
    };
    let w = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * (tap_index as f64) / (m - 1.0)).cos());
    // Tap values are bounded in [-1, 1]; the f32 cast is exact-enough.
    (sinc * w) as f32
}

impl Resampler {
    /// Construct a resampler converting `input_rate` → [`WORKING_SAMPLE_RATE_HZ`].
    ///
    /// # Errors
    /// Returns [`Error::InvalidSampleRate`] if `input_rate` is 0 or
    /// > [`MAX_INPUT_SAMPLE_RATE_HZ`].
    pub fn new(input_rate: u32) -> Result<Self> {
        if input_rate == 0 || input_rate > MAX_INPUT_SAMPLE_RATE_HZ {
            return Err(Error::InvalidSampleRate { got: input_rate });
        }
        Ok(Self {
            input_rate,
            stride: f64::from(input_rate) / f64::from(WORKING_SAMPLE_RATE_HZ),
            phase: 0.0,
            tail: Vec::new(),
            cutoff_norm: cutoff_hz(input_rate) / f64::from(input_rate),
        })
    }

    /// Resample a chunk of input audio into working-rate output.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        // Concatenate carry-over with the new chunk.
        let mut buf = std::mem::take(&mut self.tail);
        buf.extend_from_slice(input);

        let mut out = Vec::new();
        let half = (FIR_TAPS as f64) / 2.0;
        loop {
            let center = self.phase + half;
            let needed_end = (center + half).ceil() as usize;
            if needed_end > buf.len() {
                break;
            }
            let frac = self.phase.fract();
            let start = self.phase.floor() as isize;

            // Convolve, computing each tap on-the-fly with the fractional
            // phase shift. This makes the resampler a true fractional-delay
            // FIR rather than the quantized integer-delay version.
            let mut acc: f32 = 0.0;
            for k in 0..FIR_TAPS {
                let tap = fir_tap(k, frac, self.cutoff_norm);
                let idx = start + k as isize;
                if (0..buf.len() as isize).contains(&idx) {
                    acc += tap * buf[idx as usize];
                }
            }
            out.push(acc);
            self.phase += self.stride;
        }

        // Keep the trailing samples that the next call will need.
        let drop = self.phase.floor() as usize;
        if drop < buf.len() {
            self.tail = buf[drop..].to_vec();
            self.phase -= drop as f64;
        } else {
            self.tail.clear();
            self.phase -= buf.len() as f64;
        }
        out
    }

    /// Caller-provided input sample rate.
    #[must_use]
    pub fn input_rate(&self) -> u32 {
        self.input_rate
    }

    /// Clear FIR tail buffer + phase accumulator so a subsequent call to
    /// `process` starts with a clean state. Keeps the input rate, cutoff,
    /// and stride — the rate doesn't change across `reset_state` calls.
    pub(crate) fn reset_state(&mut self) {
        self.tail.clear();
        self.phase = 0.0;
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::expect_used
)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn synth_tone_at(rate: u32, freq_hz: f64, secs: f64) -> Vec<f32> {
        let n = (secs * f64::from(rate)).round() as usize;
        (0..n)
            .map(|i| {
                let t = (i as f64) / f64::from(rate);
                (2.0 * PI * freq_hz * t).sin() as f32
            })
            .collect()
    }

    #[test]
    fn rejects_zero_rate() {
        assert!(matches!(
            Resampler::new(0),
            Err(Error::InvalidSampleRate { got: 0 })
        ));
    }

    #[test]
    fn rejects_oversize_rate() {
        assert!(matches!(
            Resampler::new(MAX_INPUT_SAMPLE_RATE_HZ + 1),
            Err(Error::InvalidSampleRate { .. })
        ));
    }

    #[test]
    fn accepts_common_rates() {
        for rate in [8_000, 11_025, 22_050, 32_000, 44_100, 48_000, 96_000] {
            assert!(Resampler::new(rate).is_ok(), "{rate} should be accepted");
        }
    }

    #[test]
    fn passthrough_when_rate_matches_working_rate() {
        // At equal rates the resampler still applies its FIR (no special-case
        // bypass). Verify the output length is approximately equal to input
        // length and that a 1500 Hz tone survives.
        let mut r = Resampler::new(WORKING_SAMPLE_RATE_HZ).unwrap();
        let in_audio = synth_tone_at(WORKING_SAMPLE_RATE_HZ, 1500.0, 0.1);
        let out = r.process(&in_audio);
        // Allow up to FIR_TAPS samples of length variance (group delay + tail).
        let expected = in_audio.len();
        assert!(
            (out.len() as isize - expected as isize).abs() < 100,
            "len mismatch: out={} expected≈{}",
            out.len(),
            expected
        );
        let p = crate::vis::goertzel_power(&out, 1500.0);
        let p_off = crate::vis::goertzel_power(&out, 800.0);
        assert!(p > 10.0 * p_off, "tone should survive: {p} vs {p_off}");
    }

    #[test]
    fn resamples_44100_to_11025_preserves_tone_frequency() {
        // 1 second of 1900 Hz at 44100 Hz → resample → expect 1900 Hz at 11025 Hz.
        let mut r = Resampler::new(44_100).expect("44.1k resampler");
        let in_audio = synth_tone_at(44_100, 1900.0, 1.0);
        let out = r.process(&in_audio);
        // Output should be ~11025 samples (1 second at working rate)
        let expected = WORKING_SAMPLE_RATE_HZ as usize;
        assert!(
            (out.len() as isize - expected as isize).abs() < 200,
            "out.len()={} expected≈{expected}",
            out.len()
        );
        // Goertzel power at 1900 Hz should be much greater than at 1700/2100 Hz.
        let p_target = crate::vis::goertzel_power(&out, 1900.0);
        let p_off1 = crate::vis::goertzel_power(&out, 1700.0);
        let p_off2 = crate::vis::goertzel_power(&out, 2100.0);
        assert!(
            p_target > 10.0 * p_off1.max(p_off2),
            "p1900={p_target} p1700={p_off1} p2100={p_off2}"
        );
    }

    #[test]
    fn resamples_48000_to_11025() {
        let mut r = Resampler::new(48_000).expect("48k resampler");
        let in_audio = synth_tone_at(48_000, 1500.0, 0.5);
        let out = r.process(&in_audio);
        let expected = (WORKING_SAMPLE_RATE_HZ / 2) as usize;
        assert!((out.len() as isize - expected as isize).abs() < 200);
    }

    #[test]
    fn resamples_48000_to_11025_preserves_tone_quality() {
        // 0.5 s of 1900 Hz at 48 kHz, non-integer ratio (4.354...).
        // Pre-fix this test would have shown ~10× signal-to-noise margin
        // around 1900 Hz; with proper polyphase the margin should be 100×+.
        let mut r = Resampler::new(48_000).expect("48k resampler");
        let in_audio = synth_tone_at(48_000, 1900.0, 0.5);
        let out = r.process(&in_audio);
        let p_target = crate::vis::goertzel_power(&out, 1900.0);
        let p_off1 = crate::vis::goertzel_power(&out, 1700.0);
        let p_off2 = crate::vis::goertzel_power(&out, 2100.0);
        // Tighter than the integer-ratio 10× threshold — non-integer
        // ratios with broken polyphase would NOT meet this.
        assert!(
            p_target > 50.0 * p_off1.max(p_off2),
            "p1900={p_target} p1700={p_off1} p2100={p_off2} (polyphase quality)"
        );
    }

    #[test]
    fn streaming_calls_are_consistent() {
        let mut r = Resampler::new(44_100).unwrap();
        let in_audio = synth_tone_at(44_100, 1900.0, 0.5);
        let single = r.process(&in_audio);
        let mut r2 = Resampler::new(44_100).unwrap();
        let mid = in_audio.len() / 2;
        let mut split = r2.process(&in_audio[..mid]);
        split.extend_from_slice(&r2.process(&in_audio[mid..]));
        // Length should match within ±2 samples; per-sample diff should be
        // tiny (filter edge effects).
        assert!((single.len() as isize - split.len() as isize).abs() <= 2);
        let common = single.len().min(split.len());
        let max_diff = (0..common)
            .map(|i| (single[i] - split[i]).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_diff < 0.01, "max_diff={max_diff}");
    }
}
