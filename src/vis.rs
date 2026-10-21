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
//! (`HOP_SAMPLES`, `WINDOW_SAMPLES`, `FFT_LEN`); `HISTORY_LEN`
//! stays 45 (450 ms). `FFT_LEN=512` gives 21.5 Hz/bin like slowrx's 2048/44100.
//!
//! Inline `// slowrx <file>.c:NNN` line refs are against the gitignored
//! local reference clone in `original/slowrx/` (see `clone-slowrx.sh`);
//! verified at audit #94 (2026-05-15).

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

/// VIS code of Robot 12 B/W, the one mode whose header inverts the parity
/// bit (slowrx `vis.c:116`: `if (VISmap[VIS] == R12BW) Parity = !Parity;`).
/// Taken from its `ModeSpec`, so the parity rule and the mode table can't
/// disagree.
pub(crate) const R12BW_VIS_CODE: u8 = crate::modespec::ROBOT12_BW.vis_code;

const SEARCH_LO_HZ: f64 = 500.0;
const SEARCH_HI_HZ: f64 = 3300.0;

// ---- Lost-VIS fallback (opt-in; no slowrx counterpart) -------------------
//
// A standard VIS header is `leader 300 ms · break 10 ms · leader 300 ms ·
// start 30 ms · 8 bits × 30 ms · stop 30 ms`. The pattern matcher only looks
// at the trailing 450 ms (end of leader 2 + bits), so a fade over the bits
// loses the image even when both leaders came through. The fallback tracks
// the *leader run* instead and, if the bits never decode, assumes the
// caller's fallback mode and starts the image where the header's timing says
// it begins. See `docs/intentional-deviations.md`.

/// A hop can start a leader run if its peak is within this of 1900 Hz (wide
/// enough for any plausible receiver mistuning; the run then tracks its own
/// mean with [`TONE_TOLERANCE_HZ`]).
const LEADER_ACQUIRE_TOLERANCE_HZ: f64 = 150.0;
/// Off-leader hops a run tolerates in a row (noise hops, or the 10 ms break)
/// before it ends.
const LEADER_RUN_MAX_GAP_HOPS: u32 = 2;
/// A run counts as a VIS preamble only within this span. Both leaders plus
/// the break are ~610 ms; a run cut short by a fade during leader 2 is
/// shorter. Image content never holds one tone this long (Robot 36 has a
/// sync pulse every 150 ms, PD/Scottie every line), and the upper bound
/// rejects a steady carrier-like tone.
const LEADER_RUN_MIN_MS: u64 = 350;
const LEADER_RUN_MAX_MS: u64 = 750;
/// Minimum share of a run's hops that sit on the leader tone.
const LEADER_RUN_MIN_HIT_RATIO: f64 = 0.7;
/// The break only counts once this much leader has preceded it.
const BREAK_MIN_LEADER_MS: u64 = 200;
/// Image start relative to the break hop's window start: break centre
/// (+10 ms) → break end (+5) → leader 2 (+300) → start + 8 bits + stop (+300).
const IMAGE_START_AFTER_BREAK_HOP_MS: u64 = 615;
/// Image start relative to the run's first hop when no break was seen:
/// leader onset (+10 ms) → leader 300 + break 10 + leader 300 + bits 300.
const IMAGE_START_AFTER_RUN_START_MS: u64 = 920;
/// How long past the expected image start to wait for the real VIS before
/// assuming the fallback mode (the matcher fires within ~25 ms of the stop
/// bit's end).
const FALLBACK_FIRE_DELAY_MS: u64 = 60;

/// Milliseconds → working-rate samples.
const fn ms_to_samples(ms: u64) -> u64 {
    ms * WORKING_SAMPLE_RATE_HZ as u64 / 1000
}

/// An in-progress run of leader-tone hops.
#[derive(Clone, Copy, Debug)]
struct LeaderRun {
    start_hop: u64,
    last_hit_hop: u64,
    hits: u32,
    sum_hz: f64,
    break_hop: Option<u64>,
    gap: u32,
}

impl LeaderRun {
    fn mean_hz(&self) -> f64 {
        self.sum_hz / f64::from(self.hits)
    }
}

/// A preamble whose bits have not (yet) decoded.
#[derive(Clone, Copy, Debug)]
struct FallbackArm {
    image_start_abs: u64,
    fire_at_abs: u64,
    hedr_shift_hz: f64,
}

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
    /// Absolute working-rate sample index of hop 0's window start, fixed
    /// when the detector anchors on its first audio. Every hop position —
    /// window starts and the reported `end_sample` — is
    /// `hop_origin + hop × HOP_SAMPLES`, so a detector built mid-stream
    /// (re-armed after an image or an unknown VIS) still reports absolute
    /// positions (#99). Zero for a detector fed from the start of a stream.
    hop_origin: u64,
    /// 45-entry circular frequency history (Hz) — slowrx's `HedrBuf`.
    history: [f64; HISTORY_LEN],
    /// Ring-buffer write position (slowrx's `HedrPtr`).
    history_ptr: usize,
    /// Hops recorded so far, capped at `HISTORY_LEN` — gates pattern matching.
    history_filled: usize,
    /// Hops FFT'd; combined with `audio_origin_sample` it locates each window.
    hops_completed: u64,
    detected: Option<DetectedVis>,
    /// `|c| crate::modespec::lookup(c).is_some()` — passed to
    /// [`match_vis_pattern`] so the matcher keeps searching alignments for a
    /// *known* VIS code (issue #89 A3). An `fn` pointer: the closure captures
    /// nothing, so it coerces.
    is_known_vis: fn(u8) -> bool,
    /// VIS code to assume when a preamble's bits are lost; `None` = off.
    fallback_code: Option<u8>,
    leader_run: Option<LeaderRun>,
    fallback_arm: Option<FallbackArm>,
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
    /// Working-rate sample index where the VIS stop-bit window ended — for
    /// an assumed header, where the image audio is expected to begin.
    pub end_sample: u64,
    /// `true` when the bits were lost and `code` is the caller's fallback
    /// mode, inferred from the leader timing (see `set_fallback_code`).
    pub assumed: bool,
}

impl VisDetector {
    /// Construct a fresh VIS detector. Allocates the FFT plan + reusable
    /// buffers; reuse across many `process` calls.
    #[must_use]
    pub fn new(is_known_vis: fn(u8) -> bool) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_LEN);
        let scratch_len = fft.get_inplace_scratch_len();
        Self {
            fft,
            hann: crate::dsp::build_hann(WINDOW_SAMPLES),
            fft_buf: vec![Complex { re: 0.0, im: 0.0 }; FFT_LEN],
            // rustfft returns scratch_len = 0 for power-of-two sizes
            // (FFT_LEN=1024 is radix-2). The prior .max(FFT_LEN) was
            // dead-allocating ~8 KiB. (Audit #92 C8.)
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len],
            audio_buffer: Vec::with_capacity(WINDOW_SAMPLES * 4),
            audio_origin_sample: 0,
            hop_origin: 0,
            history: [0.0; HISTORY_LEN],
            history_ptr: 0,
            history_filled: 0,
            hops_completed: 0,
            detected: None,
            is_known_vis,
            fallback_code: None,
            leader_run: None,
            fallback_arm: None,
        }
    }

    /// VIS code to assume when a header's leaders arrive but its bits are
    /// lost (`None` disables the fallback, the default and slowrx's
    /// behaviour). Changing it drops any leader run or armed preamble seen
    /// under the previous setting, so a header can only fire in the mode
    /// that was configured while it arrived; re-setting the same code
    /// keeps them.
    pub fn set_fallback_code(&mut self, code: Option<u8>) {
        if code != self.fallback_code {
            self.leader_run = None;
            self.fallback_arm = None;
        }
        self.fallback_code = code;
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
        // Re-anchor `audio_origin_sample` lazily on first sample. The very
        // first anchor also fixes the hop grid's origin (#99).
        if self.audio_buffer.is_empty() {
            #[allow(clippy::cast_possible_truncation)]
            let chunk_len = samples.len() as u64;
            self.audio_origin_sample = total_samples_consumed.saturating_sub(chunk_len);
            if self.hops_completed == 0 {
                self.hop_origin = self.audio_origin_sample;
            }
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
            let hop = self.hops_completed;
            self.hops_completed = self.hops_completed.saturating_add(1);
            self.track_leader(hop);

            if self.history_filled >= HISTORY_LEN {
                if let Some((code, hedr_shift_hz, i_match)) =
                    match_vis_pattern(&self.rotated_history(), self.is_known_vis)
                {
                    // Stop-bit hop = `tone[14*3+i]` = `(2-i)` hops back
                    // from the latest. The bit is 30 ms = 3 hops long,
                    // so its absolute end-sample simplifies to:
                    //   `(hops_completed + i) * HOP_SAMPLES`
                    // giving a uniform ~5 ms past stop-bit end across all `i`.
                    //
                    // **Divergence from slowrx (round-2 audit Finding 11):**
                    // slowrx (`vis.c:165-170`) uses a fixed `+20 ms` skip, but
                    // its `WindowPtr` at detection is already `(2-i)×10 ms` past
                    // the stop-bit hop center — so slowrx's actual offset from
                    // true stop-bit end is `5+20=25 ms` (i=0), `15 ms` (i=1),
                    // `5 ms` (i=2), varying 5–25 ms across the three phases.
                    // Rust uses a uniform 5 ms (one HOP_SAMPLES past the stop-bit
                    // hop center), which is tighter than slowrx's 5–25 ms range.
                    // The divergence is benign: `find_sync` absorbs the 0–20 ms
                    // audio-position difference via the Skip computation.
                    let stop_end_abs =
                        self.hop_abs(self.hops_completed.saturating_add(i_match as u64));
                    let drain_to_buf =
                        usize::try_from(stop_end_abs.saturating_sub(self.audio_origin_sample))
                            .unwrap_or(usize::MAX)
                            .min(self.audio_buffer.len());
                    self.detected = Some(DetectedVis {
                        code,
                        hedr_shift_hz,
                        end_sample: stop_end_abs,
                        assumed: false,
                    });
                    self.leader_run = None;
                    self.fallback_arm = None;
                    self.audio_buffer.drain(..drain_to_buf);
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        self.audio_origin_sample += drain_to_buf as u64;
                    }
                    return;
                }
            }

            if self.fire_fallback_if_due(hop) {
                return;
            }

            // Drop samples no future window will touch (next hop starts
            // at buf_window_start + HOP_SAMPLES) — but while a fallback is
            // armed, keep everything from the expected image start on.
            let mut drain_to = buf_window_start + HOP_SAMPLES;
            if let Some(arm) = self.fallback_arm {
                let keep_from =
                    usize::try_from(arm.image_start_abs.saturating_sub(self.audio_origin_sample))
                        .unwrap_or(usize::MAX);
                drain_to = drain_to.min(keep_from);
            }
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
    ///
    /// **Window-semantics note (#28):** slowrx (`vis.c:169`) explicitly calls
    /// `readPcm(20e-3 * 44100)` after detection to "skip the rest of the stop
    /// bit" because its `pcm.WindowPtr` is at the *centre* of the stop-bit
    /// analysis window (10 ms past + 10 ms future). Rust uses a purely past
    /// window (each 20 ms window drains from the buffer head), so by the time
    /// detection fires the `audio_buffer` head is already past the stop bit.
    /// No extra skip is needed here; the residual is already aligned to the
    /// start of post-VIS image audio. The behaviors converge to the same
    /// post-stop-bit buffer state without the explicit skip.
    ///
    /// **Re-anchor contract (#40):** After calling `take_residual_buffer`,
    /// the calling code MUST construct a fresh `VisDetector::new()` rather
    /// than re-using this instance for subsequent VIS detection. The persisted
    /// `hops_completed` and `history` / `history_ptr` / `history_filled` state
    /// are not reset here. Re-using a spent detector would corrupt `end_sample`
    /// calculations and the pattern-match window on the second detection.
    /// (In V1 `SstvDecoder::process` always creates a new `VisDetector` after
    /// each image; this contract must be preserved if mid-image VIS detection
    /// is reactivated in V2.)
    pub fn take_residual_buffer(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.audio_buffer)
    }

    /// Absolute working-rate sample index where hop `hop`'s window starts.
    fn hop_abs(&self, hop: u64) -> u64 {
        self.hop_origin + hop * HOP_SAMPLES as u64
    }

    /// Position in `audio_buffer` where the next 20 ms window starts.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn next_window_start_in_buffer(&self) -> usize {
        self.hop_abs(self.hops_completed)
            .saturating_sub(self.audio_origin_sample) as usize
    }

    /// Run one 20 ms FFT, append its peak frequency to history. Slowrx
    /// vis.c line 67 falls back to `HedrBuf[(HedrPtr-1) % 45]` when the
    /// peak is at the search boundary or has a non-positive neighbour;
    /// we encode that by returning NaN from `estimate_peak_freq`.
    ///
    /// **`HedrBuf`\[-1\] UB fix:** slowrx C's fallback `HedrBuf[(HedrPtr - 1) % 45]`
    /// wraps to `HedrBuf[44]` when only the first sample exists (`HedrPtr` == 1),
    /// reading garbage from a slot last touched by a prior detection (or
    /// zero-initialised on first use — still technically UB in C). Rust's
    /// `history` is zero-initialised and the `prev_idx` computation uses
    /// wrapping arithmetic, so the first-hop fallback reads `history[44] = 0.0`
    /// (a clean zero, not garbage). **Deliberate fidelity improvement**; see
    /// `docs/intentional-deviations.md::"Fidelity improvements over slowrx"`
    /// → `"HedrBuf[-1] wraparound read in VIS detector"`.
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

    /// Update the leader-run tracker with the hop just processed, arming the
    /// lost-VIS fallback when a run ends looking like a VIS preamble.
    fn track_leader(&mut self, hop: u64) {
        if self.fallback_code.is_none() {
            return;
        }
        let f = self.history[(self.history_ptr + HISTORY_LEN - 1) % HISTORY_LEN];
        let Some(mut run) = self.leader_run else {
            if (f - LEADER_HZ).abs() < LEADER_ACQUIRE_TOLERANCE_HZ {
                self.leader_run = Some(LeaderRun {
                    start_hop: hop,
                    last_hit_hop: hop,
                    hits: 1,
                    sum_hz: f,
                    break_hop: None,
                    gap: 0,
                });
            }
            return;
        };
        let leader = run.mean_hz();
        let led_ms = (hop - run.start_hop) * 10;
        if within(f, leader, TONE_TOLERANCE_HZ) {
            run.hits += 1;
            run.sum_hz += f;
            run.last_hit_hop = hop;
            run.gap = 0;
        } else if run.break_hop.is_none()
            && led_ms >= BREAK_MIN_LEADER_MS
            && within(f, leader + BREAK_HZ_OFFSET, TONE_TOLERANCE_HZ)
        {
            run.break_hop = Some(hop);
            run.gap += 1;
        } else {
            run.gap += 1;
        }
        if run.gap > LEADER_RUN_MAX_GAP_HOPS {
            self.leader_run = None;
            self.arm_if_preamble(&run);
        } else {
            self.leader_run = Some(run);
        }
    }

    /// Arm the fallback if a just-ended leader run has a VIS preamble's
    /// length and consistency.
    fn arm_if_preamble(&mut self, run: &LeaderRun) {
        if self.fallback_arm.is_some() {
            return;
        }
        let span_hops = run.last_hit_hop - run.start_hop + 1;
        let span_ms = span_hops * 10;
        #[allow(clippy::cast_precision_loss)]
        let hit_ratio = f64::from(run.hits) / span_hops as f64;
        if !(LEADER_RUN_MIN_MS..=LEADER_RUN_MAX_MS).contains(&span_ms)
            || hit_ratio < LEADER_RUN_MIN_HIT_RATIO
        {
            return;
        }
        let image_start_abs = match run.break_hop {
            Some(b) => self.hop_abs(b) + ms_to_samples(IMAGE_START_AFTER_BREAK_HOP_MS),
            None => self.hop_abs(run.start_hop) + ms_to_samples(IMAGE_START_AFTER_RUN_START_MS),
        };
        self.fallback_arm = Some(FallbackArm {
            image_start_abs,
            fire_at_abs: image_start_abs + ms_to_samples(FALLBACK_FIRE_DELAY_MS),
            hedr_shift_hz: run.mean_hz() - LEADER_HZ,
        });
    }

    /// If an armed fallback's deadline has passed with no real VIS, emit the
    /// assumed detection, keeping audio from the expected image start on.
    fn fire_fallback_if_due(&mut self, hop: u64) -> bool {
        let (Some(arm), Some(code)) = (self.fallback_arm, self.fallback_code) else {
            return false;
        };
        if self.hop_abs(hop) + (WINDOW_SAMPLES as u64) < arm.fire_at_abs {
            return false;
        }
        let drain_to_buf =
            usize::try_from(arm.image_start_abs.saturating_sub(self.audio_origin_sample))
                .unwrap_or(usize::MAX)
                .min(self.audio_buffer.len());
        self.audio_buffer.drain(..drain_to_buf);
        #[allow(clippy::cast_possible_truncation)]
        {
            self.audio_origin_sample += drain_to_buf as u64;
        }
        self.detected = Some(DetectedVis {
            code,
            hedr_shift_hz: arm.hedr_shift_hz,
            end_sample: arm.image_start_abs,
            assumed: true,
        });
        self.fallback_arm = None;
        self.leader_run = None;
        true
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
    // Use slowrx-equivalent truncation (not `.round()`) via `crate::dsp::get_bin`.
    // See `crate::dsp::get_bin` for rationale.
    #[allow(clippy::cast_possible_truncation)]
    let bin_for = |hz: f64| -> usize { crate::dsp::get_bin(hz, fft_len, WORKING_SAMPLE_RATE_HZ) };
    let lo = bin_for(SEARCH_LO_HZ);
    let hi = bin_for(SEARCH_HI_HZ);
    if lo == 0 || hi >= fft_len.saturating_sub(1) || lo >= hi {
        return f64::NAN;
    }
    let mut max_bin = lo;
    let mut max_p = crate::dsp::power(spectrum[lo]);
    for (k, &c) in spectrum.iter().enumerate().take(hi).skip(lo + 1) {
        let p = crate::dsp::power(c);
        if p > max_p {
            max_p = p;
            max_bin = k;
        }
    }
    if max_bin <= lo || max_bin >= hi {
        return f64::NAN;
    }
    let p_prev = crate::dsp::power(spectrum[max_bin - 1]);
    let p_curr = max_p;
    let p_next = crate::dsp::power(spectrum[max_bin + 1]);
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
/// Tries 9 alignments (i × j, 3 phases × 3 leader candidates). Returns the
/// first **known** parity-passing alignment as `(vis_code, hedr_shift_hz, i)`
/// (or the first parity-passing **unknown** code as a fallback) (`hedr_shift_hz =
/// observed_leader - 1900`, `i` is the matched phase). Uses relative
/// ±25 Hz tolerance — slowrx vis.c lines 82-104. (Indices like `3 + i`
/// spell out slowrx's `tone[1*3+i]` so parity with C is one-to-one.)
///
/// **Deliberate divergence from slowrx (round-2 audit Finding 5):** slowrx's
/// outer `for (i=0; i<3; i++)` loop has an `if (HedrShift != 0) break;` guard
/// at the top (`vis.c:82-83`). A first-match parity failure sets `HedrShift =
/// tone[j] - 1900` but then clears `gotvis`. If `HedrShift ≠ 0`, the outer
/// loop fires the guard on the *next* `i` iteration, skipping all remaining
/// `(i, j)` candidates — even if one of them would have passed parity. Rust
/// instead tries all 9 `(i, j)` combinations exhaustively and returns on the
/// first parity-passing match. This is the correct behavior; slowrx's early
/// exit is a quirk of its `HedrShift`-set-before-parity-check pattern. The
/// difference only manifests on mistuned radios (`HedrShift` ≠ 0) where the
/// first pattern match has parity failure — a rare edge case in practice.
///
/// **Keep-searching for a known code (issue #89, A3):** like slowrx (`vis.c`),
/// this tries all 9 alignments and returns the *first known* code (`is_known`).
/// A clean burst whose only parity-passing alignment maps to an *unknown* code
/// (a mode this crate doesn't implement) is still returned — as
/// the `first_unknown` fallback — so the caller can surface it via
/// `SstvEvent::UnknownVis`. See `docs/intentional-deviations.md`
/// ("VIS: keep-searching for a known code; surface clean unknown bursts").
fn match_vis_pattern(
    tones: &[f64; HISTORY_LEN],
    is_known: impl Fn(u8) -> bool,
) -> Option<(u8, f64, usize)> {
    let tol = TONE_TOLERANCE_HZ;
    let mut first_unknown: Option<(u8, f64, usize)> = None;
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
                } else {
                    // `bit` is the received parity bit. R12BW flips the
                    // *accumulated* parity per slowrx `vis.c:116`:
                    // `if (VISmap[VIS] == R12BW) Parity = !Parity;`.
                    if code == R12BW_VIS_CODE {
                        parity ^= 1;
                    }
                    if parity != bit {
                        bit_ok = false;
                    }
                    break; // k == 7 is the last iteration; explicit break mirrors the classify-fail arm
                }
            }
            if bit_ok {
                let m = (code, leader - LEADER_HZ, i);
                if is_known(code) {
                    return Some(m); // slowrx breaks on the first known code
                }
                first_unknown.get_or_insert(m);
            }
        }
    }
    first_unknown
}

#[inline]
fn within(value: f64, target: f64, tol: f64) -> bool {
    (value - target).abs() < tol
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

    /// Build a 45-entry tone history (`HedrBuf` order: `[0]` oldest) that the
    /// matcher reads as `phase0_code` at phase `i = 0` and, if `phase1_code`
    /// is `Some(c)`, as `c` at phase `i = 1`. Phase `i = 2`'s bit slots are
    /// left as leader tones, so it fails bit classification (never matches).
    /// All leader / break slots are positioned so phases 0 and 1 pass those
    /// checks. Tones are at the un-shifted (`hedr_shift = 0`) frequencies.
    fn synth_tone_history(phase0_code: u8, phase1_code: Option<u8>) -> [f64; HISTORY_LEN] {
        let leader = LEADER_HZ;
        let break_f = leader + BREAK_HZ_OFFSET;
        let zero_f = leader + BIT_ZERO_OFFSET;
        let one_f = leader + BIT_ONE_OFFSET;
        let bit_freq = |b: u8| if b == 1 { one_f } else { zero_f };
        let mut t = [leader; HISTORY_LEN];
        // Break tones — checked at indices 15+i and 42+i for i in 0..3.
        t[15] = break_f;
        t[16] = break_f;
        t[42] = break_f;
        t[43] = break_f;
        // Phase i=0: data bits at tones[18 + 3k] (k=0..6), parity at tones[39].
        let mut p0 = 0u8;
        for k in 0..7 {
            let b = (phase0_code >> k) & 1;
            p0 ^= b;
            t[18 + 3 * k] = bit_freq(b);
        }
        let p0 = if phase0_code == R12BW_VIS_CODE {
            p0 ^ 1
        } else {
            p0
        };
        t[39] = bit_freq(p0);
        // Phase i=1: data bits at tones[19 + 3k], parity at tones[40].
        if let Some(c) = phase1_code {
            let mut p1 = 0u8;
            for k in 0..7 {
                let b = (c >> k) & 1;
                p1 ^= b;
                t[19 + 3 * k] = bit_freq(b);
            }
            let p1 = if c == R12BW_VIS_CODE { p1 ^ 1 } else { p1 };
            t[40] = bit_freq(p1);
        }
        t
    }

    /// `is_known` predicate as used in production: a 7-bit VIS code is "known"
    /// iff it maps to a `ModeSpec`.
    fn vis_known(code: u8) -> bool {
        crate::modespec::lookup(code).is_some()
    }

    /// Build a synthetic VIS burst encoding `code` with even parity — odd
    /// for Robot 12 B/W, whose parity bit is inverted (slowrx `vis.c:116`).
    /// `freq_offset_hz` shifts every tone (mistuned-radio test fixture).
    /// Continuous-phase: avoids bit-boundary discontinuities that would
    /// pull FFT peaks off-tone.
    pub fn synth_vis_with_offset(code: u8, pre_silence_secs: f64, freq_offset_hz: f64) -> Vec<f32> {
        assert!(code < 0x80, "VIS codes are 7 bits");
        let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
        let mut tone = crate::test_tone::ToneWriter::with_pre_silence_samples(
            (pre_silence_secs * sr).round() as usize,
        );
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
        tone.fill_secs(leader, 0.300);
        tone.fill_secs(break_f, 0.030);
        let mut parity = 0u8;
        for b in 0..7 {
            let bit = (code >> b) & 1;
            parity ^= bit;
            tone.fill_secs(bit_freq(bit), 0.030);
        }
        // R12BW (`R12BW_VIS_CODE`) inverts the parity bit per slowrx `vis.c:116`.
        // The detector's `match_vis_pattern` does the same inversion when
        // checking, so synthetic bursts must follow the same convention or
        // they'd fail parity at the receiver.
        let parity_bit = if code == R12BW_VIS_CODE {
            parity ^ 1
        } else {
            parity
        };
        tone.fill_secs(bit_freq(parity_bit), 0.030);
        tone.fill_secs(break_f, 0.030);
        tone.into_vec()
    }

    /// Convenience wrapper: zero-offset VIS burst.
    pub fn synth_vis(code: u8, pre_silence_secs: f64) -> Vec<f32> {
        synth_vis_with_offset(code, pre_silence_secs, 0.0)
    }

    /// Helper: feed `audio` into a fresh detector and return the result.
    fn run(audio: &[f32]) -> Option<DetectedVis> {
        let mut det = VisDetector::new(vis_known);
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

    /// R12BW (code 0x06) uses inverted parity per slowrx `vis.c:116`.
    /// `match_vis_pattern` must invert before checking, otherwise every
    /// valid R12BW burst would be silently rejected.
    #[test]
    fn r12bw_uses_inverted_parity() {
        // The parity rule keys off the Robot 12 B/W `ModeSpec`'s code.
        assert_eq!(R12BW_VIS_CODE, 0x06);
        let spec = crate::modespec::lookup(R12BW_VIS_CODE).expect("R12BW is in the mode table");
        assert_eq!(spec.mode, crate::modespec::SstvMode::Robot12Bw);
        // Standard synth_vis emits inverted parity for code 0x06, so the
        // detector must accept it under the R12BW convention.
        let audio = vis_padded(0x06, 0.0, 0.0);
        let detected = run(&audio).expect("R12BW (0x06) with inverted parity decodes");
        assert_eq!(detected.code, 0x06);
    }

    /// Negative side of #26: standard (non-inverted) parity for code 0x06
    /// must NOT decode. This is what would have happened before the
    /// inversion landed — silent rejection of every R12BW burst.
    #[test]
    fn r12bw_rejects_standard_parity() {
        // Hand-emit a 0x06 burst with the standard (uninverted) parity
        // bit. The detector should now reject it (post-fix), proving the
        // inversion is doing real work and not a no-op.
        let mut tone = crate::test_tone::ToneWriter::new();
        let break_f = LEADER_HZ + BREAK_HZ_OFFSET;
        let bit_freq = |bit: u8| {
            LEADER_HZ
                + if bit == 1 {
                    BIT_ONE_OFFSET
                } else {
                    BIT_ZERO_OFFSET
                }
        };
        tone.fill_secs(LEADER_HZ, 0.300);
        tone.fill_secs(break_f, 0.030);
        let mut parity = 0u8;
        for b in 0..7 {
            let bit = (R12BW_VIS_CODE >> b) & 1;
            parity ^= bit;
            tone.fill_secs(bit_freq(bit), 0.030);
        }
        // Standard (non-R12BW-inverted) parity. Detector should reject.
        tone.fill_secs(bit_freq(parity), 0.030);
        tone.fill_secs(break_f, 0.030);
        let mut out = tone.into_vec();
        out.extend(std::iter::repeat_n(0.0_f32, 256));
        assert!(
            run(&out).is_none(),
            "0x06 with standard (non-inverted) parity must NOT decode — \
             slowrx vis.c:116 inverts parity for R12BW."
        );
    }

    /// Header with both leaders but the bits replaced by noise: `leader_ms`
    /// of 1900 Hz, optionally split by a 10 ms 1200 Hz break after 300 ms,
    /// then 300 ms of noise and `tail_ms` of silence. Returns the audio and
    /// the true image-start sample (where a stop bit would have ended).
    pub fn lost_bits_header(leader_ms: f64, with_break: bool, tail_ms: f64) -> (Vec<f32>, usize) {
        let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
        let ms = |m: f64| (m * sr / 1000.0).round() as usize;
        let mut tone = crate::test_tone::ToneWriter::new();
        if with_break {
            tone.fill_secs(LEADER_HZ, 0.300);
            tone.fill_secs(LEADER_HZ + BREAK_HZ_OFFSET, 0.010);
            tone.fill_secs(LEADER_HZ, (leader_ms - 310.0) / 1000.0);
        } else {
            tone.fill_secs(LEADER_HZ, leader_ms / 1000.0);
        }
        let mut audio = tone.into_vec();
        let image_start = audio.len() + ms(300.0);
        let mut seed = 0x1234_5678_u32;
        for _ in 0..ms(300.0) {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            audio.push((f64::from(seed) / f64::from(u32::MAX) * 2.0 - 1.0) as f32);
        }
        audio.extend(std::iter::repeat_n(0.0_f32, ms(tail_ms)));
        (audio, image_start)
    }

    fn run_with_fallback(audio: &[f32], fallback: Option<u8>) -> Option<DetectedVis> {
        let mut det = VisDetector::new(vis_known);
        det.set_fallback_code(fallback);
        det.process(audio, audio.len() as u64);
        det.take_detected()
    }

    #[test]
    fn fallback_assumes_the_mode_and_finds_the_image_start() {
        for with_break in [true, false] {
            let (audio, image_start) = lost_bits_header(610.0, with_break, 200.0);
            let d = run_with_fallback(&audio, Some(0x08)).expect("fallback fires");
            assert!(d.assumed, "marked as assumed");
            assert_eq!(d.code, 0x08);
            assert!(d.hedr_shift_hz.abs() < 10.0, "hedr {}", d.hedr_shift_hz);
            let err_ms = (d.end_sample as f64 - image_start as f64) * 1000.0
                / f64::from(WORKING_SAMPLE_RATE_HZ);
            assert!(
                err_ms.abs() <= 15.0,
                "break={with_break}: image start off by {err_ms:.1} ms"
            );
        }
    }

    #[test]
    fn changing_the_fallback_mode_drops_a_pending_preamble() {
        // A header armed under one fallback must not fire under another:
        // feed the preamble with Robot 36 set, switch to PD120 before the
        // fire deadline, and nothing may be emitted for that header.
        let (audio, image_start) = lost_bits_header(610.0, true, 200.0);
        let mut det = VisDetector::new(vis_known);
        det.set_fallback_code(Some(0x08));
        let split = image_start - (0.1 * f64::from(WORKING_SAMPLE_RATE_HZ)) as usize;
        det.process(&audio[..split], split as u64);
        assert!(det.take_detected().is_none(), "not due yet");
        det.set_fallback_code(Some(0x5F));
        det.process(&audio[split..], audio.len() as u64);
        assert!(
            det.take_detected().is_none(),
            "stale preamble must not fire"
        );
        // Re-setting the *same* mode keeps a pending preamble.
        let mut det = VisDetector::new(vis_known);
        det.set_fallback_code(Some(0x08));
        det.process(&audio[..split], split as u64);
        det.set_fallback_code(Some(0x08));
        det.process(&audio[split..], audio.len() as u64);
        assert!(det
            .take_detected()
            .is_some_and(|d| d.assumed && d.code == 0x08));
    }

    #[test]
    fn fallback_is_off_by_default() {
        let (audio, _) = lost_bits_header(610.0, true, 200.0);
        assert!(run_with_fallback(&audio, None).is_none());
    }

    #[test]
    fn fallback_ignores_runs_outside_the_preamble_length() {
        // A single 300 ms leader (too short to be sure it was a header) and
        // an 800 ms one (too long — a steady tone, not two leaders).
        for leader_ms in [300.0, 800.0] {
            let (audio, _) = lost_bits_header(leader_ms, false, 300.0);
            assert!(
                run_with_fallback(&audio, Some(0x08)).is_none(),
                "{leader_ms} ms leader run must not arm the fallback"
            );
        }
    }

    #[test]
    fn a_decodable_vis_wins_over_the_fallback() {
        let mut audio = synth_vis(0x5F, 0.0);
        audio.extend(std::iter::repeat_n(0.0_f32, 2048));
        let d = run_with_fallback(&audio, Some(0x08)).expect("real VIS");
        assert_eq!(d.code, 0x5F);
        assert!(!d.assumed);
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

    #[test]
    fn match_vis_pattern_clean_known_code() {
        // 0x5F == PD120 is in the lookup table.
        let tones = synth_tone_history(0x5F, None);
        let m = match_vis_pattern(&tones, vis_known).expect("known code matches");
        assert_eq!(m.0, 0x5F);
        assert!(m.1.abs() < 1e-9, "hedr_shift should be 0, got {}", m.1);
        assert_eq!(m.2, 0, "matched at phase i = 0");
    }

    #[test]
    fn match_vis_pattern_clean_unknown_code_falls_back() {
        // 0x01 is a valid 7-bit code with parity 1, but maps to no mode.
        assert!(crate::modespec::lookup(0x01).is_none());
        let tones = synth_tone_history(0x01, None);
        let m = match_vis_pattern(&tones, vis_known)
            .expect("an unknown-but-parity-passing code is still returned (fallback)");
        assert_eq!(m.0, 0x01);
        assert_eq!(m.2, 0);
    }

    #[test]
    fn match_vis_pattern_prefers_known_over_earlier_unknown() {
        // Phase i=0 spells unknown 0x01; phase i=1 spells known 0x5F.
        // The (i,j) loop hits i=0 first — the old code returned 0x01 there;
        // the fix must skip it and return 0x5F at i=1.
        let tones = synth_tone_history(0x01, Some(0x5F));
        let m = match_vis_pattern(&tones, vis_known).expect("known code at a later alignment");
        assert_eq!(m.0, 0x5F, "should skip the earlier unknown 0x01 alignment");
        assert_eq!(m.2, 1, "matched at phase i = 1");
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
