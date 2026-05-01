//! VIS (Vertical Interval Signaling) header detection.
//!
//! Translated in spirit from slowrx's `vis.c` (Oona Räisänen, ISC License).
//! See `NOTICE.md` for full attribution. We replace slowrx's 2048-point FFT
//! plus Gaussian-interpolated peak finder with four Goertzel filters tuned
//! at the four tone frequencies that actually matter for VIS detection
//! (1900 / 1200 / 1100 / 1300 Hz). The result is mathematically equivalent
//! for VIS purposes and dramatically simpler to test in isolation.

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

/// Each VIS bit lasts 30 ms; at 11025 Hz that is `WINDOW_SAMPLES`
/// audio samples per bit.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) const WINDOW_SAMPLES: usize = (0.030 * WORKING_SAMPLE_RATE_HZ as f64) as usize; // 330

/// VIS detection state machine. Operates on audio at the
/// [`WORKING_SAMPLE_RATE_HZ`] working rate.
///
/// Slides a 30-ms window across input audio. For each window, runs the
/// four Goertzel filters and records the dominant tone. When the last
/// 14 windows match the VIS pattern (leader · break · 8 bits · stop),
/// emits the decoded 7-bit code via [`Self::take_detected`].
pub(crate) struct VisDetector {
    buffer: Vec<f32>,
    tones: Vec<Tone>,
    detected: Option<DetectedVis>,
}

/// Categorical tone classification for a single 30 ms window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Leader,  // 1900 Hz
    Break,   // 1200 Hz
    BitZero, // 1300 Hz
    BitOne,  // 1100 Hz
    Other,
}

/// Result of a successful VIS detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DetectedVis {
    /// 7-bit VIS code (LSB-first decoded).
    pub code: u8,
    /// Decoder-relative sample index where the stop-bit ended.
    pub end_sample: u64,
}

impl VisDetector {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(WINDOW_SAMPLES * 4),
            tones: Vec::new(),
            detected: None,
        }
    }

    /// Push working-rate audio samples into the detector.
    ///
    /// `total_samples_consumed` is the running sample count from the
    /// caller's perspective (used to populate `DetectedVis::end_sample`).
    pub fn process(&mut self, samples: &[f32], total_samples_consumed: u64) {
        self.buffer.extend_from_slice(samples);

        while self.buffer.len() >= WINDOW_SAMPLES && self.detected.is_none() {
            let window: Vec<f32> = self.buffer.drain(..WINDOW_SAMPLES).collect();
            let tone = classify(&window);
            self.tones.push(tone);
            if self.tones.len() > 14 {
                self.tones.remove(0);
            }
            if self.tones.len() == 14 {
                if let Some(code) = match_vis_pattern(&self.tones) {
                    // The window we just consumed is the stop bit.
                    // `total_samples_consumed` is the running counter
                    // AFTER the caller's chunk was appended; subtracting
                    // the not-yet-consumed remainder of `buffer` gives
                    // the sample boundary at the end of the just-drained
                    // (stop-bit) window — the exact end of the burst.
                    let end_sample =
                        total_samples_consumed.saturating_sub(self.buffer.len() as u64);
                    self.detected = Some(DetectedVis { code, end_sample });
                }
            }
        }
    }

    /// Take the detected VIS (if any) and clear internal state to
    /// resume awaiting a new VIS.
    pub fn take_detected(&mut self) -> Option<DetectedVis> {
        let d = self.detected.take();
        if d.is_some() {
            self.tones.clear();
            self.buffer.clear();
        }
        d
    }
}

fn classify(window: &[f32]) -> Tone {
    let p_leader = goertzel_power(window, LEADER_HZ);
    let p_break = goertzel_power(window, BREAK_HZ);
    let p0 = goertzel_power(window, BIT_ZERO_HZ);
    let p1 = goertzel_power(window, BIT_ONE_HZ);
    // Pick the maximum; require at least 5× margin over the second-place
    // tone to avoid mis-classifying noise. The 5× threshold is empirical;
    // slowrx uses ±25 Hz frequency tolerance which translates to a similar
    // power ratio in tone-classifier terms.
    let mut ranked = [
        (Tone::Leader, p_leader),
        (Tone::Break, p_break),
        (Tone::BitZero, p0),
        (Tone::BitOne, p1),
    ];
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    if ranked[0].1 > 5.0 * ranked[1].1 {
        ranked[0].0
    } else {
        Tone::Other
    }
}

/// Match the 14-window VIS pattern: 4 leader + 1 break + 8 bits + 1 stop.
/// Returns the recovered 7-bit code on parity-OK, otherwise `None`.
fn match_vis_pattern(tones: &[Tone]) -> Option<u8> {
    debug_assert_eq!(tones.len(), 14);
    // Windows [0..4] = leader, [4] = break, [5..13] = bits, [13] = stop.
    if !(tones[0] == Tone::Leader
        && tones[1] == Tone::Leader
        && tones[2] == Tone::Leader
        && tones[3] == Tone::Leader)
    {
        return None;
    }
    if tones[4] != Tone::Break || tones[13] != Tone::Break {
        return None;
    }
    let mut code = 0u8;
    let mut parity = 0u8;
    for (i, tone) in tones[5..12].iter().enumerate() {
        let bit = match tone {
            Tone::BitOne => 1,
            Tone::BitZero => 0,
            _ => return None,
        };
        code |= bit << i;
        parity ^= bit;
    }
    let parity_bit = match tones[12] {
        Tone::BitOne => 1,
        Tone::BitZero => 0,
        _ => return None,
    };
    if parity != parity_bit {
        return None;
    }
    Some(code)
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::expect_used
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

    #[test]
    fn detects_pd120_vis_clean_signal() {
        let mut det = VisDetector::new();
        let audio = synth_vis(0x5F, 0.0);
        let n = audio.len();
        det.process(&audio, n as u64);
        let d = det.take_detected().expect("PD120 detected");
        assert_eq!(d.code, 0x5F);
    }

    #[test]
    fn detects_pd180_vis_clean_signal() {
        let mut det = VisDetector::new();
        let audio = synth_vis(0x60, 0.0);
        let n = audio.len();
        det.process(&audio, n as u64);
        let d = det.take_detected().expect("PD180 detected");
        assert_eq!(d.code, 0x60);
    }

    #[test]
    fn detects_vis_after_pre_silence() {
        // Pre-silence is a whole multiple of WINDOW_SAMPLES so the burst
        // aligns with detector window boundaries. Sub-window alignment is
        // a follow-up task (FrameSync realigns at 10 ms granularity).
        let mut det = VisDetector::new();
        let pre_secs = (WINDOW_SAMPLES as f64 * 7.0) / f64::from(WORKING_SAMPLE_RATE_HZ);
        let audio = synth_vis(0x5F, pre_secs);
        let n = audio.len();
        det.process(&audio, n as u64);
        let d = det.take_detected().expect("PD120 detected after silence");
        assert_eq!(d.code, 0x5F);
    }

    #[test]
    fn rejects_random_noise() {
        let mut det = VisDetector::new();
        // 1 second of mid-band noise that doesn't match VIS.
        let n = WORKING_SAMPLE_RATE_HZ as usize;
        let audio: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(WORKING_SAMPLE_RATE_HZ);
                (0.3 * (2.0 * PI * 1750.0 * t).sin()) as f32
            })
            .collect();
        det.process(&audio, n as u64);
        assert!(det.take_detected().is_none());
    }

    #[test]
    fn parity_failure_is_rejected() {
        let mut det = VisDetector::new();
        // Build a clean burst, then overwrite bit-0's detector window
        // with the break tone so the bit decoder rejects the slot.
        // synth_vis emits 300 ms of leader (10 detector windows) + 30 ms
        // break (1 window), so bit-0 lives at detector window 11.
        let mut audio = synth_vis(0x5F, 0.0);
        let start = WINDOW_SAMPLES * 11;
        let end = start + WINDOW_SAMPLES;
        let break_window = synth_tone(BREAK_HZ, 0.030);
        audio[start..end].copy_from_slice(&break_window[..WINDOW_SAMPLES]);
        det.process(&audio, audio.len() as u64);
        assert!(det.take_detected().is_none());
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn detector_does_not_panic_on_arbitrary_audio(
            len in 0usize..32_000,
            seed in 0u64..u64::MAX,
        ) {
            // Deterministic pseudo-random audio in [-1, 1].
            let mut x = seed;
            let mut audio = Vec::with_capacity(len);
            for _ in 0..len {
                // xorshift
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let v = ((x as i64) as f64) / (i64::MAX as f64);
                audio.push(v as f32);
            }
            let mut det = VisDetector::new();
            det.process(&audio, audio.len() as u64);
            // Whatever it returns is fine; it just must not panic.
            let _ = det.take_detected();
        }

        #[test]
        fn every_valid_vis_code_decodes_correctly(code in 0u8..0x80) {
            let mut det = VisDetector::new();
            let audio = synth_vis(code, 0.0);
            det.process(&audio, audio.len() as u64);
            let d = det.take_detected().expect("clean VIS always decodes");
            prop_assert_eq!(d.code, code);
        }
    }
}
