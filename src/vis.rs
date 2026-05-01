//! VIS (Vertical Interval Signaling) header detection.
//!
//! Translated in spirit from slowrx's `vis.c` (Oona Räisänen, ISC License).
//! See `NOTICE.md` for full attribution. We replace slowrx's 2048-point FFT
//! plus Gaussian-interpolated peak finder with four Goertzel filters tuned
//! at the four tone frequencies that actually matter for VIS detection
//! (1900 / 1200 / 1100 / 1300 Hz). The result is mathematically equivalent
//! for VIS purposes and dramatically simpler to test in isolation.

// Items below are consumed by later VIS tasks (1.3 / 1.4); until those land
// they're only exercised from tests, so silence dead-code at module level.
#![allow(dead_code)]

use crate::resample::WORKING_SAMPLE_RATE_HZ;

/// VIS leader / sync tones in Hz.
pub(crate) const LEADER_HZ: f64 = 1900.0;
pub(crate) const BREAK_HZ: f64 = 1200.0;
pub(crate) const BIT_ZERO_HZ: f64 = 1300.0;
pub(crate) const BIT_ONE_HZ: f64 = 1100.0;

/// Power output of a single Goertzel filter run on an input window.
///
/// Returns the bin power (proportional to amplitude squared).
/// `target_hz` is the frequency to match; `samples` is the input
/// window at the working sample rate ([`WORKING_SAMPLE_RATE_HZ`]).
pub(crate) fn goertzel_power(samples: &[f32], target_hz: f64) -> f64 {
    // VIS windows are ~330 samples at 11025 Hz, far below f64's 2^53 mantissa.
    #[allow(clippy::cast_precision_loss)]
    let n = samples.len() as f64;
    if n == 0.0 {
        return 0.0;
    }
    let k = (0.5 + n * target_hz / f64::from(WORKING_SAMPLE_RATE_HZ)).floor();
    let omega = 2.0 * std::f64::consts::PI * k / n;
    let coeff = 2.0 * omega.cos();

    let mut s_prev = 0.0_f64;
    let mut s_prev2 = 0.0_f64;
    for &sample in samples {
        let s = f64::from(sample) + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }
    s_prev2.mul_add(s_prev2, s_prev.mul_add(s_prev, -coeff * s_prev * s_prev2))
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp
)]
pub(crate) mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Generate `secs` of pure tone at `freq_hz` at the working sample rate.
    pub(crate) fn synth_tone(freq_hz: f64, secs: f64) -> Vec<f32> {
        let n = (secs * f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize;
        (0..n)
            .map(|i| {
                let t = (i as f64) / f64::from(WORKING_SAMPLE_RATE_HZ);
                (2.0 * PI * freq_hz * t).sin() as f32
            })
            .collect()
    }

    /// Build a synthetic VIS burst encoding the given 7-bit `code`
    /// with even parity. Pads `pre_silence_secs` of zeros before the
    /// leader so the detector has to find the burst inside a longer
    /// audio buffer.
    pub(crate) fn synth_vis(code: u8, pre_silence_secs: f64) -> Vec<f32> {
        assert!(code < 0x80, "VIS codes are 7 bits");

        let mut out: Vec<f32> = Vec::new();
        // Pre-silence
        let n_pre = (pre_silence_secs * f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize;
        out.resize(n_pre, 0.0);

        // 300 ms leader (10 × 30 ms windows of 1900 Hz)
        out.extend(synth_tone(LEADER_HZ, 0.300));
        // 30 ms break at 1200 Hz
        out.extend(synth_tone(BREAK_HZ, 0.030));

        // 7 data bits (LSB-first per slowrx convention)
        let mut parity = 0u8;
        for b in 0..7 {
            let bit = (code >> b) & 1;
            parity ^= bit;
            let f = if bit == 1 { BIT_ONE_HZ } else { BIT_ZERO_HZ };
            out.extend(synth_tone(f, 0.030));
        }
        // 8th bit = even parity
        let parity_freq = if parity == 1 { BIT_ONE_HZ } else { BIT_ZERO_HZ };
        out.extend(synth_tone(parity_freq, 0.030));

        // 30 ms stop at 1200 Hz
        out.extend(synth_tone(BREAK_HZ, 0.030));
        out
    }

    #[test]
    fn synth_vis_for_pd120_has_expected_length() {
        // 0.3 s leader + 0.03 s break + 8 × 0.03 s bits + 0.03 s stop = 0.6 s nominal.
        // (with no pre-silence)
        // synth_vis calls synth_tone 11 times (1 leader + 10 data/stop windows), each
        // independently rounding to the nearest sample, so the actual total may differ
        // from round(0.6 × sr) by up to ±11 samples.  We use a tolerance of ±11.
        let samples = synth_vis(0x5F, 0.0);
        let expected_len = (0.6 * f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize;
        assert!(
            (samples.len() as isize - expected_len as isize).abs() <= 11,
            "len={} expected≈{expected_len}",
            samples.len()
        );
    }

    #[test]
    fn goertzel_finds_leader_tone() {
        let samples = synth_tone(1900.0, 0.030);
        let p_leader = goertzel_power(&samples, LEADER_HZ);
        let p_break = goertzel_power(&samples, BREAK_HZ);
        // Leader bin has dramatically more power than the break bin.
        assert!(
            p_leader > 50.0 * p_break,
            "leader={p_leader} break={p_break}"
        );
    }

    #[test]
    fn goertzel_finds_bit_zero() {
        let samples = synth_tone(1300.0, 0.030);
        let p0 = goertzel_power(&samples, BIT_ZERO_HZ);
        let p1 = goertzel_power(&samples, BIT_ONE_HZ);
        assert!(p0 > 50.0 * p1, "bit0={p0} bit1={p1}");
    }

    #[test]
    fn goertzel_finds_bit_one() {
        let samples = synth_tone(1100.0, 0.030);
        let p0 = goertzel_power(&samples, BIT_ZERO_HZ);
        let p1 = goertzel_power(&samples, BIT_ONE_HZ);
        assert!(p1 > 50.0 * p0, "bit0={p0} bit1={p1}");
    }

    #[test]
    fn empty_input_returns_zero_power() {
        assert_eq!(goertzel_power(&[], 1900.0), 0.0);
    }

    #[test]
    fn goertzel_handcomputed_quarter_cycle() {
        // x = [1, 0, -1, 0], target = sample_rate/4 → expected power = 4.0
        let samples = [1.0_f32, 0.0, -1.0, 0.0];
        let target = f64::from(WORKING_SAMPLE_RATE_HZ) / 4.0;
        let p = goertzel_power(&samples, target);
        assert!((p - 4.0).abs() < 1e-9, "expected 4.0, got {p}");
    }
}
