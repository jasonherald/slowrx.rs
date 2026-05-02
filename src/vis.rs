//! VIS (Vertical Interval Signaling) header detection.
//!
//! Faithful translation of slowrx's `vis.c` (Oona Räisänen, ISC License) —
//! see `NOTICE.md`. A 10 ms-hop sliding window with 20 ms Hann-weighted
//! audio feeds a 512-FFT (zero-padded for spectral interpolation),
//! peak-find in 500-3300 Hz, Gaussian-log peak interpolation. The
//! resulting frequencies feed a 45-entry sliding history; the matcher
//! tries 9 alignments (i × j, 3 phases × 3 leader candidates) using
//! **relative** ±25 Hz tolerance from the observed leader. On match,
//! `HedrShift = leader_observed - 1900` is plumbed through to the
//! per-pixel demod for radio-mistuning compensation.
//!
//! Sizes scale by 1/4 from slowrx's 44.1 kHz to our 11.025 kHz
//! ([`HOP_SAMPLES`], [`WINDOW_SAMPLES`], [`FFT_LEN`]); [`HISTORY_LEN`]
//! stays 45 (450 ms). `FFT_LEN=512` gives 21.5 Hz/bin like slowrx's 2048/44100.

use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::Arc;

use crate::resample::WORKING_SAMPLE_RATE_HZ;

// Tone frequencies relative to the observed leader (slowrx vis.c).
pub(crate) const LEADER_HZ: f64 = 1900.0;
pub(crate) const BREAK_HZ_OFFSET: f64 = -700.0; // 1200 - 1900
pub(crate) const BIT_ZERO_OFFSET: f64 = -600.0; // 1300 - 1900
pub(crate) const BIT_ONE_OFFSET: f64 = -800.0; // 1100 - 1900
pub(crate) const TONE_TOLERANCE_HZ: f64 = 25.0; // ±25 Hz

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) const HOP_SAMPLES: usize = (0.010 * WORKING_SAMPLE_RATE_HZ as f64) as usize;
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) const WINDOW_SAMPLES: usize = (0.020 * WORKING_SAMPLE_RATE_HZ as f64) as usize;
pub(crate) const FFT_LEN: usize = 512;
pub(crate) const HISTORY_LEN: usize = 45; // slowrx HedrBuf size

const SEARCH_LO_HZ: f64 = 500.0;
const SEARCH_HI_HZ: f64 = 3300.0;

/// VIS detection state machine at the [`WORKING_SAMPLE_RATE_HZ`] working rate.
pub(crate) struct VisDetector {
    fft: Arc<dyn rustfft::Fft<f32>>,
    hann: Vec<f32>,
    fft_buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Audio samples since `audio_origin_sample`.
    audio_buffer: Vec<f32>,
    /// Working-rate sample index of `audio_buffer[0]`.
    audio_origin_sample: u64,
    /// 45-entry circular frequency history (Hz) — slowrx's `HedrBuf`.
    history: [f64; HISTORY_LEN],
    /// Ring-buffer write position (slowrx's `HedrPtr`).
    history_ptr: usize,
    /// Hops recorded so far, capped at `HISTORY_LEN` — gates pattern matching.
    history_filled: usize,
    /// Hops FFT'd; combined with `audio_origin_sample` it locates each window.
    hops_completed: u64,
    detected: Option<DetectedVis>,
}

/// Result of a successful VIS detection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DetectedVis {
    /// 7-bit VIS code (LSB-first).
    pub code: u8,
    /// Radio mistuning offset in Hz: `observed_leader - 1900`. The caller
    /// plumbs this through to per-pixel demod for accurate pixel-frequency
    /// mapping (slowrx vis.c line 106 → video.c line 406).
    pub hedr_shift_hz: f64,
    /// Working-rate sample index where the VIS stop-bit window ended.
    pub end_sample: u64,
}

impl VisDetector {
    /// Construct a fresh VIS detector. Allocates the FFT plan + reusable
    /// buffers; reuse across many `process` calls.
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_LEN);
        let scratch_len = fft.get_inplace_scratch_len();
        Self {
            fft,
            hann: build_hann_window(WINDOW_SAMPLES),
            fft_buf: vec![Complex { re: 0.0, im: 0.0 }; FFT_LEN],
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len.max(FFT_LEN)],
            audio_buffer: Vec::with_capacity(WINDOW_SAMPLES * 4),
            audio_origin_sample: 0,
            history: [0.0; HISTORY_LEN],
            history_ptr: 0,
            history_filled: 0,
            hops_completed: 0,
            detected: None,
        }
    }

    /// Push working-rate audio into the detector. `total_samples_consumed`
    /// is the running sample count *after* this chunk was added — used to
    /// resolve every hop's absolute sample index.
    pub fn process(&mut self, samples: &[f32], total_samples_consumed: u64) {
        // Slowrx vis.c line 137: `if (gotvis) break;` — drop further audio
        // until the caller drains the detection result.
        if self.detected.is_some() {
            return;
        }
        // Re-anchor `audio_origin_sample` lazily on first sample.
        if self.audio_buffer.is_empty() {
            #[allow(clippy::cast_possible_truncation)]
            let chunk_len = samples.len() as u64;
            self.audio_origin_sample = total_samples_consumed.saturating_sub(chunk_len);
        }
        self.audio_buffer.extend_from_slice(samples);

        // Each loop iteration: process one hop, optionally match, drain.
        loop {
            let buf_window_start = self.next_window_start_in_buffer();
            let buf_window_end = buf_window_start + WINDOW_SAMPLES;
            if buf_window_end > self.audio_buffer.len() {
                break;
            }
            self.process_hop(buf_window_start);
            self.hops_completed = self.hops_completed.saturating_add(1);

            if self.history_filled >= HISTORY_LEN {
                if let Some((code, hedr_shift_hz, i_match)) =
                    match_vis_pattern(&self.rotated_history())
                {
                    // Stop-bit hop = `tone[14*3+i]` = `(2-i)` hops back
                    // from the latest. The bit is 30 ms = 3 hops long,
                    // so its absolute end-sample simplifies to:
                    //   `(hops_completed + i) * HOP_SAMPLES`
                    // (slowrx vis.c lines 168-170 instead skips a fixed
                    // 20 ms regardless of `i`; we use the precise i-aware
                    // boundary so per-pixel image alignment stays tight).
                    let stop_end_abs =
                        (self.hops_completed.saturating_add(i_match as u64)) * HOP_SAMPLES as u64;
                    let drain_to_buf =
                        usize::try_from(stop_end_abs.saturating_sub(self.audio_origin_sample))
                            .unwrap_or(usize::MAX)
                            .min(self.audio_buffer.len());
                    self.detected = Some(DetectedVis {
                        code,
                        hedr_shift_hz,
                        end_sample: stop_end_abs,
                    });
                    self.audio_buffer.drain(..drain_to_buf);
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        self.audio_origin_sample += drain_to_buf as u64;
                    }
                    return;
                }
            }

            // Drop samples no future window will touch (next hop starts
            // at buf_window_start + HOP_SAMPLES).
            let drain_to = buf_window_start + HOP_SAMPLES;
            self.audio_buffer.drain(..drain_to);
            #[allow(clippy::cast_possible_truncation)]
            {
                self.audio_origin_sample += drain_to as u64;
            }
        }
    }

    /// Take the detected VIS (if any). Audio buffer is preserved so the
    /// caller can recover post-stop-bit residue via [`Self::take_residual_buffer`].
    pub fn take_detected(&mut self) -> Option<DetectedVis> {
        self.detected.take()
    }

    /// Take any audio still buffered (post-stop-bit residue). The detector
    /// keeps an empty buffer; the next `process` call re-anchors the origin.
    pub fn take_residual_buffer(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.audio_buffer)
    }

    /// Position in `audio_buffer` where the next 20 ms window starts.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn next_window_start_in_buffer(&self) -> usize {
        let next_hop_abs = self.hops_completed * HOP_SAMPLES as u64;
        next_hop_abs.saturating_sub(self.audio_origin_sample) as usize
    }

    /// Run one 20 ms FFT, append its peak frequency to history. Slowrx
    /// vis.c line 67 falls back to `HedrBuf[(HedrPtr-1) % 45]` when the
    /// peak is at the search boundary or has a non-positive neighbour;
    /// we encode that by returning NaN from `estimate_peak_freq`.
    fn process_hop(&mut self, buf_window_start: usize) {
        let window = &self.audio_buffer[buf_window_start..buf_window_start + WINDOW_SAMPLES];
        for (i, slot) in self.fft_buf.iter_mut().enumerate() {
            *slot = if i < WINDOW_SAMPLES {
                Complex {
                    re: window[i] * self.hann[i],
                    im: 0.0,
                }
            } else {
                Complex { re: 0.0, im: 0.0 }
            };
        }
        self.fft
            .process_with_scratch(&mut self.fft_buf, &mut self.scratch[..]);

        let peak_hz = estimate_peak_freq(&self.fft_buf);
        let prev_idx = (self.history_ptr + HISTORY_LEN - 1) % HISTORY_LEN;
        self.history[self.history_ptr] = if peak_hz.is_finite() {
            peak_hz
        } else {
            self.history[prev_idx]
        };
        self.history_ptr = (self.history_ptr + 1) % HISTORY_LEN;
        if self.history_filled < HISTORY_LEN {
            self.history_filled += 1;
        }
    }

    /// Rotate the circular history so `[0]` is oldest, `[HISTORY_LEN-1]`
    /// is newest — slowrx vis.c line 76: `tone[i] = HedrBuf[(HedrPtr + i) % 45]`.
    fn rotated_history(&self) -> [f64; HISTORY_LEN] {
        let mut out = [0.0_f64; HISTORY_LEN];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.history[(self.history_ptr + i) % HISTORY_LEN];
        }
        out
    }
}

/// Build a length-`n` symmetric Hann window. Matches slowrx vis.c line 30:
/// `Hann[i] = 0.5 * (1 - cos(2π i / (n-1)))`.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn build_hann_window(n: usize) -> Vec<f32> {
    let m = (n.saturating_sub(1).max(1)) as f64;
    (0..n)
        .map(|i| {
            let v = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * (i as f64) / m).cos());
            v as f32
        })
        .collect()
}

/// Find the dominant peak in 500..3300 Hz and refine via Gaussian-log
/// peak interpolation (slowrx vis.c lines 54-70). Returns NaN if the
/// peak is at the boundary or any of the three sample bins (peak-1,
/// peak, peak+1) are non-positive — callers then fall back to the
/// previous history entry.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn estimate_peak_freq(spectrum: &[Complex<f32>]) -> f64 {
    let fft_len = spectrum.len();
    let bin_for = |hz: f64| -> usize {
        ((hz * fft_len as f64) / f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize
    };
    let lo = bin_for(SEARCH_LO_HZ);
    let hi = bin_for(SEARCH_HI_HZ);
    if lo == 0 || hi >= fft_len.saturating_sub(1) || lo >= hi {
        return f64::NAN;
    }
    let power = |c: Complex<f32>| -> f64 {
        let r = f64::from(c.re);
        let i = f64::from(c.im);
        r * r + i * i
    };
    let mut max_bin = lo;
    let mut max_p = power(spectrum[lo]);
    for (k, &c) in spectrum.iter().enumerate().take(hi).skip(lo + 1) {
        let p = power(c);
        if p > max_p {
            max_p = p;
            max_bin = k;
        }
    }
    if max_bin <= lo || max_bin >= hi {
        return f64::NAN;
    }
    let p_prev = power(spectrum[max_bin - 1]);
    let p_curr = max_p;
    let p_next = power(spectrum[max_bin + 1]);
    if p_prev <= 0.0 || p_curr <= 0.0 || p_next <= 0.0 {
        return f64::NAN;
    }
    // bin = MaxBin + log(P[+1]/P[-1]) / (2 * log(P[0]^2 / (P[+1]*P[-1])))
    let num = (p_next / p_prev).ln();
    let denom = 2.0 * (p_curr * p_curr / (p_next * p_prev)).ln();
    let bin = if denom.abs() > 1e-12 {
        max_bin as f64 + num / denom
    } else {
        max_bin as f64
    };
    bin / fft_len as f64 * f64::from(WORKING_SAMPLE_RATE_HZ)
}

/// Match the 14-window VIS pattern in a 45-entry frequency history.
/// Tries 9 alignments (i × j, 3 phases × 3 leader candidates). Returns
/// `(vis_code, hedr_shift_hz, i)` on detection (`hedr_shift_hz =
/// observed_leader - 1900`, `i` is the matched phase). Uses relative
/// ±25 Hz tolerance — slowrx vis.c lines 82-104. (Indices like `3 + i`
/// spell out slowrx's `tone[1*3+i]` so parity with C is one-to-one.)
fn match_vis_pattern(tones: &[f64; HISTORY_LEN]) -> Option<(u8, f64, usize)> {
    let tol = TONE_TOLERANCE_HZ;
    for i in 0..3 {
        for j in 0..3 {
            let leader = tones[j];
            if !within(tones[3 + i], leader, tol)
                || !within(tones[6 + i], leader, tol)
                || !within(tones[9 + i], leader, tol)
                || !within(tones[12 + i], leader, tol)
            {
                continue;
            }
            let break_target = leader + BREAK_HZ_OFFSET;
            if !within(tones[15 + i], break_target, tol)
                || !within(tones[42 + i], break_target, tol)
            {
                continue;
            }
            let zero_target = leader + BIT_ZERO_OFFSET;
            let one_target = leader + BIT_ONE_OFFSET;
            let mut code = 0u8;
            let mut parity = 0u8;
            let mut bit_ok = true;
            for k in 0..8 {
                let t = tones[18 + i + 3 * k];
                let bit = if within(t, zero_target, tol) {
                    0u8
                } else if within(t, one_target, tol) {
                    1u8
                } else {
                    bit_ok = false;
                    break;
                };
                if k < 7 {
                    code |= bit << k;
                    parity ^= bit;
                } else if parity != bit {
                    bit_ok = false;
                }
            }
            if bit_ok {
                return Some((code, leader - LEADER_HZ, i));
            }
        }
    }
    None
}

#[inline]
fn within(value: f64, target: f64, tol: f64) -> bool {
    (value - target).abs() < tol
}

/// Goertzel power on `samples` at `target_hz` (bin power, ~amplitude²).
/// Used by `decoder::estimate_freq` and the resample-quality tests.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn goertzel_power(samples: &[f32], target_hz: f64) -> f64 {
    let n = samples.len() as f64;
    if n == 0.0 {
        return 0.0;
    }
    let k = (0.5 + n * target_hz / f64::from(WORKING_SAMPLE_RATE_HZ)).floor();
    let coeff = 2.0 * (2.0 * std::f64::consts::PI * k / n).cos();
    let mut s_prev = 0.0_f64;
    let mut s_prev2 = 0.0_f64;
    for &sample in samples {
        let s = f64::from(sample) + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }
    s_prev2.mul_add(s_prev2, s_prev.mul_add(s_prev, -coeff * s_prev * s_prev2))
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::expect_used,
    clippy::wildcard_imports,
    clippy::must_use_candidate,
    dead_code
)]
pub mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Generate `secs` of pure tone at `freq_hz` at the working sample rate.
    pub fn synth_tone(freq_hz: f64, secs: f64) -> Vec<f32> {
        let n = (secs * f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize;
        synth_tone_n(freq_hz, n)
    }

    /// Generate `n` samples of pure tone at `freq_hz` at the working sample rate.
    pub fn synth_tone_n(freq_hz: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = (i as f64) / f64::from(WORKING_SAMPLE_RATE_HZ);
                (2.0 * PI * freq_hz * t).sin() as f32
            })
            .collect()
    }

    /// Build a synthetic VIS burst encoding `code` with even parity.
    /// `freq_offset_hz` shifts every tone (mistuned-radio test fixture).
    /// Continuous-phase: avoids bit-boundary discontinuities that would
    /// pull FFT peaks off-tone.
    pub fn synth_vis_with_offset(code: u8, pre_silence_secs: f64, freq_offset_hz: f64) -> Vec<f32> {
        assert!(code < 0x80, "VIS codes are 7 bits");
        let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
        let mut out: Vec<f32> = vec![0.0; (pre_silence_secs * sr).round() as usize];
        let mut phase = 0.0_f64;
        let mut emit = |freq: f64, secs: f64, out: &mut Vec<f32>| {
            let dphi = 2.0 * PI * freq / sr;
            for _ in 0..(secs * sr).round() as usize {
                out.push(phase.sin() as f32);
                phase += dphi;
                if phase > 2.0 * PI {
                    phase -= 2.0 * PI;
                }
            }
        };
        let leader = LEADER_HZ + freq_offset_hz;
        let break_f = leader + BREAK_HZ_OFFSET;
        let bit_freq = |bit: u8| -> f64 {
            leader
                + if bit == 1 {
                    BIT_ONE_OFFSET
                } else {
                    BIT_ZERO_OFFSET
                }
        };
        emit(leader, 0.300, &mut out);
        emit(break_f, 0.030, &mut out);
        let mut parity = 0u8;
        for b in 0..7 {
            let bit = (code >> b) & 1;
            parity ^= bit;
            emit(bit_freq(bit), 0.030, &mut out);
        }
        emit(bit_freq(parity), 0.030, &mut out);
        emit(break_f, 0.030, &mut out);
        out
    }

    /// Convenience wrapper: zero-offset VIS burst.
    pub fn synth_vis(code: u8, pre_silence_secs: f64) -> Vec<f32> {
        synth_vis_with_offset(code, pre_silence_secs, 0.0)
    }

    /// Helper: feed `audio` into a fresh detector and return the result.
    fn run(audio: &[f32]) -> Option<DetectedVis> {
        let mut det = VisDetector::new();
        det.process(audio, audio.len() as u64);
        det.take_detected()
    }

    /// Helper: build a VIS burst with a trailing zero pad so the sliding
    /// window has clean post-stop-bit hops to consume.
    fn vis_padded(code: u8, pre_silence_secs: f64, freq_offset_hz: f64) -> Vec<f32> {
        let mut audio = synth_vis_with_offset(code, pre_silence_secs, freq_offset_hz);
        audio.extend(std::iter::repeat_n(0.0_f32, 256));
        audio
    }

    #[test]
    fn empty_input_returns_zero_power() {
        assert_eq!(goertzel_power(&[], 1900.0), 0.0);
    }

    #[test]
    fn goertzel_handcomputed_quarter_cycle() {
        let samples = [1.0_f32, 0.0, -1.0, 0.0];
        let target = f64::from(WORKING_SAMPLE_RATE_HZ) / 4.0;
        let p = goertzel_power(&samples, target);
        assert!((p - 4.0).abs() < 1e-9, "expected 4.0, got {p}");
    }

    #[test]
    fn hann_window_endpoints_are_zero() {
        let h = build_hann_window(WINDOW_SAMPLES);
        assert!(h[0].abs() < 1e-6);
        assert!(h[h.len() - 1].abs() < 1e-6);
        let mid = h.len() / 2;
        assert!((h[mid] - 1.0).abs() < 1e-2, "middle ≈ 1, got {}", h[mid]);
    }

    #[test]
    fn detects_clean_pd120_and_pd180() {
        for &code in &[0x5F_u8, 0x60] {
            let d = run(&vis_padded(code, 0.0, 0.0)).expect("clean detect");
            assert_eq!(d.code, code);
            assert!(d.hedr_shift_hz.abs() < 10.0);
        }
    }

    #[test]
    fn detects_pd120_with_50hz_offset() {
        let d = run(&vis_padded(0x5F, 0.050, 50.0)).expect("offset PD120");
        assert_eq!(d.code, 0x5F);
        assert!(
            (d.hedr_shift_hz - 50.0).abs() < 10.0,
            "got {}",
            d.hedr_shift_hz
        );
    }

    #[test]
    fn detects_pd180_with_minus_70hz_offset() {
        let d = run(&vis_padded(0x60, 0.080, -70.0)).expect("offset PD180");
        assert_eq!(d.code, 0x60);
        assert!(
            (d.hedr_shift_hz + 70.0).abs() < 10.0,
            "got {}",
            d.hedr_shift_hz
        );
    }

    #[test]
    fn detects_with_pre_silence_aligned_or_misaligned() {
        // Aligned: 7×HOP samples; misaligned: 37 samples (not a hop boundary).
        for pre_samples in [(7 * HOP_SAMPLES) as f64, 37.0] {
            let pre_secs = pre_samples / f64::from(WORKING_SAMPLE_RATE_HZ);
            let d = run(&vis_padded(0x5F, pre_secs, 0.0)).expect("detect after silence");
            assert_eq!(d.code, 0x5F);
        }
    }

    #[test]
    fn rejects_isolated_noise() {
        let mut x: u64 = 0xdead_beef_cafe_babe;
        let n = WORKING_SAMPLE_RATE_HZ as usize;
        let audio: Vec<f32> = (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (((x as i64) as f64) / (i64::MAX as f64)) as f32 * 0.3
            })
            .collect();
        assert!(run(&audio).is_none());
    }

    #[test]
    fn rejects_constant_off_band_tone() {
        // A pure 1750 Hz tone has no break/leader pattern; must reject.
        let n = WORKING_SAMPLE_RATE_HZ as usize;
        let audio = synth_tone_n(1750.0, n);
        assert!(run(&audio).is_none());
    }

    #[test]
    fn parity_failure_is_rejected() {
        // Zero out one of the bit windows so the bit classifier rejects.
        let mut audio = synth_vis(0x5F, 0.0);
        let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
        let bit5_start = ((0.300 + 0.030 + 5.0 * 0.030) * sr) as usize;
        let bit5_end = bit5_start + (0.030 * sr) as usize;
        for s in &mut audio[bit5_start..bit5_end] {
            *s = 0.0;
        }
        audio.extend(std::iter::repeat_n(0.0_f32, 256));
        assert!(run(&audio).is_none());
    }

    #[cfg(test)]
    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(64))]

            #[test]
            fn detector_does_not_panic_on_arbitrary_audio(
                len in 0usize..32_000,
                seed in 0u64..u64::MAX,
            ) {
                let mut x = seed.max(1);
                let mut audio = Vec::with_capacity(len);
                for _ in 0..len {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    let v = ((x as i64) as f64) / (i64::MAX as f64);
                    audio.push(v as f32);
                }
                let _ = run(&audio);
            }

            #[test]
            fn every_valid_vis_code_decodes_correctly(code in 0u8..0x80) {
                let audio = vis_padded(code, 0.0, 0.0);
                let d = run(&audio).expect("clean VIS always decodes");
                prop_assert_eq!(d.code, code);
            }
        }
    }
}
