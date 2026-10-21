//! Start an image from a mode's line-sync rhythm when no VIS was heard
//! (#115). No slowrx counterpart — see `docs/intentional-deviations.md`.
//!
//! While the decoder waits for a VIS, this detector probes the 1200 Hz sync
//! band (the same [`SyncTracker`] probe `find_sync` uses) and periodically
//! folds the most recent lines at the expected mode's line period into a
//! column accumulator. When the pulses line up — scored with the same
//! [`sync_quality`] measure as a finished image — on consecutive
//! evaluations, it locates the first line in the window whose pulse is
//! present and reports where that line starts. The decoder then decodes
//! from there; `find_sync` refines the alignment exactly as after a VIS.

use crate::modespec::{ModeSpec, SstvMode, SyncPosition};
use crate::resample::WORKING_SAMPLE_RATE_HZ;
use crate::sync::{
    best_pulse_window, sync_pulse_columns, sync_quality, SyncTracker, SYNC_PROBE_STRIDE, X_ACC_BINS,
};

/// Fold at least this many lines per evaluation …
const MIN_WINDOW_LINES: usize = 8;

/// … and at least this much audio, so a short mode (Robot 36: 150 ms
/// lines) still needs seconds of consistent pulses before an image starts.
/// A real VIS (stop bit) or the lost-VIS fallback (60 ms into the image)
/// always wins the race for an image that has a header.
const MIN_WINDOW_SECONDS: f64 = 2.0;

/// Re-evaluate every this many sync probes (~23 ms at 4 samples/probe).
const EVAL_EVERY_PROBES: usize = 64;

/// A window must score at least this on [`sync_quality`] …
pub(crate) const START_QUALITY: f32 = 0.6;

/// … on this many evaluations in a row before an image starts.
const START_CONSECUTIVE: u32 = 3;

/// Pulses at least this many of the 700 columns wide can't be faked by a
/// noise line (see `SyncStartDetector::first_pulsed_line`).
const WIDE_PULSE_COLUMNS: usize = 20;

/// A narrow-pulse image's first line must begin a run of this many
/// consecutive lines that carry the pulse (see
/// `SyncStartDetector::line_is_pulsed`).
const PULSED_RUN: usize = 2;

/// A pulsed line's longest unbroken run of sync-band hits must cover at
/// least this fraction of the pulse length.
const PULSE_RUN_FRACTION: f64 = 0.5;

/// ... capped at this many probes (3.6 ms): noise essentially never trips
/// the probe this many times in a row, and a long pulse (PD's 20 ms) that
/// noise has fragmented on a weak signal still passes.
const PULSE_RUN_CAP_PROBES: f64 = 10.0;

/// Probes searched either side of a line's expected pulse.
const PULSE_SEARCH_SLACK_PROBES: f64 = 4.0;

/// Start this many sync-pulse widths ahead of the located line start, so a
/// slightly late pulse estimate never clips the first line.
const START_LEAD_SYNC_PULSES: f64 = 2.0;

/// Robot 24/36 separator tone before a line's `Cr` (even lines) …
pub(crate) const ROBOT_EVEN_SEPARATOR_HZ: f64 = 1500.0;
/// … and before its `Cb` (odd lines).
pub(crate) const ROBOT_ODD_SEPARATOR_HZ: f64 = 2300.0;
/// Length of that even/odd tone; the rest of `septr_seconds` is a 1900 Hz
/// porch.
pub(crate) const ROBOT_PARITY_SEPARATOR_SECONDS: f64 = 0.0045;

/// Where an image detected from its sync rhythm begins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SyncStart {
    /// Absolute working-rate sample index where the decoder should start
    /// collecting image audio.
    pub start_sample: u64,
    /// Score of the window that triggered the start.
    pub quality: f32,
}

/// Rolling sync-rhythm detector for one expected mode.
pub(crate) struct SyncStartDetector {
    spec: ModeSpec,
    tracker: SyncTracker,
    window_lines: usize,
    /// Working-rate audio, `audio[0]` at absolute sample `audio_origin`.
    audio: Vec<f32>,
    audio_origin: u64,
    /// One `has_sync` result per probe; `probes[k]` covers absolute samples
    /// `probe_origin + k * SYNC_PROBE_STRIDE ..` (centred half a stride in).
    probes: Vec<bool>,
    probe_origin: u64,
    since_eval: usize,
    streak: u32,
    anchored: bool,
    last_quality: f32,
    pending: Option<SyncStart>,
}

impl SyncStartDetector {
    pub(crate) fn new(spec: ModeSpec) -> Self {
        Self {
            spec,
            tracker: SyncTracker::new(0.0),
            window_lines: window_lines(spec),
            audio: Vec::new(),
            audio_origin: 0,
            probes: Vec::new(),
            probe_origin: 0,
            since_eval: 0,
            streak: 0,
            anchored: false,
            last_quality: 0.0,
            pending: None,
        }
    }

    /// The mode this detector looks for.
    pub(crate) fn spec(&self) -> ModeSpec {
        self.spec
    }

    /// Last evaluation's score (0 before the first evaluation).
    #[cfg(test)]
    pub(crate) fn last_quality(&self) -> f32 {
        self.last_quality
    }

    /// Feed working-rate audio; `total_samples_consumed` is the absolute
    /// sample index just past `samples` (the decoder's running count). Once
    /// a start is found it is held for [`Self::take_start`] and further
    /// audio is only buffered.
    pub(crate) fn process(&mut self, samples: &[f32], total_samples_consumed: u64) {
        if !self.anchored {
            self.audio_origin = total_samples_consumed.saturating_sub(samples.len() as u64);
            self.probe_origin = self.audio_origin;
            self.anchored = true;
        }
        self.audio.extend_from_slice(samples);
        if self.pending.is_some() {
            return;
        }
        let window = self.window_probes();
        loop {
            let rel = self.probes.len() * SYNC_PROBE_STRIDE + self.probe_lag();
            // The probe window needs half an FFT window past its centre; one
            // stride beyond is ample (as in the decoder's own probe loop).
            if rel + 2 * SYNC_PROBE_STRIDE > self.audio.len() {
                break;
            }
            let has = self
                .tracker
                .has_sync_at(&self.audio, rel + SYNC_PROBE_STRIDE / 2);
            self.probes.push(has);
            self.since_eval += 1;
            if self.since_eval >= EVAL_EVERY_PROBES && self.probes.len() >= window {
                self.since_eval = 0;
                self.pending = self.evaluate(self.probes.len() - window);
                if self.pending.is_some() {
                    return;
                }
            }
        }
        self.trim(window);
    }

    /// The detected start, once (later calls return `None`).
    pub(crate) fn take_start(&mut self) -> Option<SyncStart> {
        self.pending.take()
    }

    /// Hand over the buffered audio from absolute sample `start` on.
    pub(crate) fn take_audio_from(&mut self, start: u64) -> Vec<f32> {
        let skip = usize::try_from(start.saturating_sub(self.audio_origin)).unwrap_or(usize::MAX);
        let mut audio = std::mem::take(&mut self.audio);
        audio.drain(..skip.min(audio.len()));
        audio
    }

    /// Samples between `audio_origin` and `probe_origin` (audio kept ahead
    /// of the oldest probe so a start can back off before it).
    fn probe_lag(&self) -> usize {
        usize::try_from(self.probe_origin - self.audio_origin).unwrap_or(usize::MAX)
    }

    /// Window length in probes.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn window_probes(&self) -> usize {
        let samples = self.window_lines as f64 * self.line_samples();
        (samples / SYNC_PROBE_STRIDE as f64).ceil() as usize + 1
    }

    fn line_samples(&self) -> f64 {
        self.spec.line_seconds * f64::from(WORKING_SAMPLE_RATE_HZ)
    }

    /// Whether the probe `lines` line periods after probe `first` hit.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn probe_at(&self, first: usize, lines: f64) -> bool {
        let idx = first + (lines * self.line_samples() / SYNC_PROBE_STRIDE as f64) as usize;
        self.probes.get(idx).copied().unwrap_or(false)
    }

    /// Score the window starting at probe `first`; on the
    /// [`START_CONSECUTIVE`]th passing evaluation, locate the image start.
    #[allow(clippy::cast_precision_loss)]
    fn evaluate(&mut self, first: usize) -> Option<SyncStart> {
        let mut x_acc = vec![0_u32; X_ACC_BINS];
        for y in 0..self.window_lines {
            for (x, slot) in x_acc.iter_mut().enumerate() {
                if self.probe_at(first, y as f64 + x as f64 / X_ACC_BINS as f64) {
                    *slot += 1;
                }
            }
        }
        let quality = sync_quality(&x_acc, self.spec, self.window_lines);
        self.last_quality = quality;
        if quality < START_QUALITY {
            self.streak = 0;
            return None;
        }
        self.streak += 1;
        if self.streak < START_CONSECUTIVE {
            return None;
        }
        let width = sync_pulse_columns(self.spec, X_ACC_BINS);
        let (column, _) = best_pulse_window(&x_acc, width);
        // No start yet if a narrow pulse has no clean first line; the
        // streak holds, so the next evaluation (more image) tries again.
        let start_sample = self.locate_start(first, column, width)?;
        Some(SyncStart {
            start_sample,
            quality,
        })
    }

    /// Absolute start sample for the image's first window line
    /// ([`Self::first_pulsed_line`]), the pulse starting at `column` and
    /// `width` columns wide.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn locate_start(&self, first: usize, column: usize, width: usize) -> Option<u64> {
        let first_line = self.first_pulsed_line(first, column, width)?;
        let pulse =
            (first_line as f64 + column as f64 / X_ACC_BINS as f64) * self.spec.line_seconds;
        let window_start = (self.probe_origin + (first * SYNC_PROBE_STRIDE) as u64) as f64;
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let mut line_start = window_start + (pulse - pulse_offset_seconds(self.spec)) * rate;
        if self.first_line_is_odd(line_start) {
            line_start -= self.line_samples();
        }
        let start = line_start - START_LEAD_SYNC_PULSES * self.spec.sync_seconds * rate;
        Some(start.max(self.audio_origin as f64).round() as u64)
    }

    /// The first window line of the image. A wide pulse (Robot, PD: 20+ of
    /// 700 columns) can't be faked by noise, so the first line whose pulse
    /// columns are at least half hit is it. A narrow pulse (SC-2's 5.5 ms is
    /// ~8 columns, Martin's ~8, Scottie DX's ~6) can be: a noise line hits
    /// half of 8 columns by chance. Those modes need [`PULSED_RUN`]
    /// consecutive lines carrying an unbroken run of hits
    /// ([`Self::line_is_pulsed`]); if no pair passes yet, there's no first
    /// line (`None`) rather than a guess. The run test isn't used for wide pulses:
    /// on weak real signals noise fragments the pulse, and it found real
    /// Robot 36 images 4–10 lines late.
    #[allow(clippy::cast_precision_loss)]
    fn first_pulsed_line(&self, first: usize, column: usize, width: usize) -> Option<usize> {
        if width >= WIDE_PULSE_COLUMNS {
            let n = X_ACC_BINS as f64;
            return (0..self.window_lines)
                .find(|&y| {
                    let hits = (column..column + width)
                        .filter(|&c| self.probe_at(first, y as f64 + c as f64 / n))
                        .count();
                    2 * hits >= width
                })
                .or(Some(0));
        }
        (0..self.window_lines.saturating_sub(PULSED_RUN - 1))
            .find(|&y| (y..y + PULSED_RUN).all(|l| self.line_is_pulsed(first, l, column)))
    }

    /// Whether window line `y` carries the sync pulse whose start sits at
    /// `column`: the longest unbroken run of sync-band hits around it covers
    /// at least [`PULSE_RUN_FRACTION`] of the pulse (at most
    /// [`PULSE_RUN_CAP_PROBES`] probes). A real pulse is one
    /// contiguous run of hits as long as the pulse; noise trips the probe in
    /// short, scattered bursts — and with a narrow pulse (SC-2's 5.5 ms is
    /// ~8 of 700 columns, Scottie DX's ~6) a column count alone is easy for
    /// a noise line to fake.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn line_is_pulsed(&self, first: usize, y: usize, column: usize) -> bool {
        let per_probe = SYNC_PROBE_STRIDE as f64;
        let line_probes = self.line_samples() / per_probe;
        let pulse_probes = self.spec.sync_seconds * f64::from(WORKING_SAMPLE_RATE_HZ) / per_probe;
        let start = first as f64 + (y as f64 + column as f64 / X_ACC_BINS as f64) * line_probes;
        // Search a few probes either side: the column grid is coarser than
        // the probe grid on long lines.
        let lo = (start - PULSE_SEARCH_SLACK_PROBES).max(0.0) as usize;
        let hi = (start + pulse_probes + PULSE_SEARCH_SLACK_PROBES) as usize;
        let (mut run, mut longest) = (0_usize, 0_usize);
        for idx in lo..hi {
            if self.probes.get(idx).copied().unwrap_or(false) {
                run += 1;
                longest = longest.max(run);
            } else {
                run = 0;
            }
        }
        longest as f64 >= (PULSE_RUN_FRACTION * pulse_probes).min(PULSE_RUN_CAP_PROBES)
    }

    /// Robot 24/36 decode `Cr` or `Cb` by line-index parity, so an image
    /// must start on an even line. Votes the even/odd separator tone over
    /// the window's lines from absolute sample `line_start`; `true` when
    /// that line is odd. Always `false` for other modes.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn first_line_is_odd(&self, line_start: f64) -> bool {
        if !matches!(self.spec.mode, SstvMode::Robot24 | SstvMode::Robot36) {
            return false;
        }
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let separator = self.spec.sync_seconds
            + self.spec.porch_seconds
            + 2.0 * f64::from(self.spec.line_pixels) * self.spec.pixel_seconds;
        let tone_len = (ROBOT_PARITY_SEPARATOR_SECONDS * rate) as usize;
        let mut vote = 0.0;
        for k in 0..self.window_lines {
            let at = line_start + (k as f64 * self.spec.line_seconds + separator) * rate;
            let from = (at - self.audio_origin as f64).round();
            let Some(seg) = (from >= 0.0)
                .then(|| self.audio.get(from as usize..from as usize + tone_len))
                .flatten()
            else {
                continue;
            };
            let even_power = crate::dsp::goertzel_power(seg, ROBOT_EVEN_SEPARATOR_HZ);
            let odd_power = crate::dsp::goertzel_power(seg, ROBOT_ODD_SEPARATOR_HZ);
            let lean = (odd_power - even_power) / (odd_power + even_power + f64::EPSILON);
            vote += if k % 2 == 0 { lean } else { -lean };
        }
        vote > 0.0
    }

    /// Drop probes no future window needs and audio more than two lines
    /// before the oldest kept probe (a start backs off at most one line
    /// plus a few pulse widths before it).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn trim(&mut self, window: usize) {
        let excess = self.probes.len().saturating_sub(window + EVAL_EVERY_PROBES);
        self.probes.drain(..excess);
        self.probe_origin += (excess * SYNC_PROBE_STRIDE) as u64;
        let keep_from = self
            .probe_origin
            .saturating_sub((2.0 * self.line_samples()) as u64);
        let drop = usize::try_from(keep_from.saturating_sub(self.audio_origin)).unwrap_or(0);
        let drop = drop.min(self.audio.len());
        self.audio.drain(..drop);
        self.audio_origin += drop as u64;
    }
}

/// Lines folded per evaluation for `spec` (see [`MIN_WINDOW_LINES`],
/// [`MIN_WINDOW_SECONDS`]).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn window_lines(spec: ModeSpec) -> usize {
    let for_seconds = (MIN_WINDOW_SECONDS / spec.line_seconds).ceil() as usize;
    MIN_WINDOW_LINES.max(for_seconds)
}

/// Offset from a line's start, as the decoder lays lines out, to the start
/// of its sync pulse — the inverse of `find_sync`'s skip mapping. Pulses at
/// the line start give 0. Scottie's pulse sits in the right half of the
/// line, where `find_sync` folds the edge back half a line and then applies
/// `skip_correction_seconds`, so the line starts `line / 2 − correction`
/// before the pulse.
fn pulse_offset_seconds(spec: ModeSpec) -> f64 {
    match spec.sync_position {
        SyncPosition::Scottie => spec.line_seconds / 2.0 - spec.skip_correction_seconds(),
        _ => 0.0,
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::expect_used
)]
mod tests {
    use super::*;
    use crate::modespec::for_mode;
    use crate::test_tone::white_noise as noise;

    const RATE: usize = WORKING_SAMPLE_RATE_HZ as usize;

    /// Feed `audio` in 1024-sample chunks; the first start found, if any.
    fn feed(det: &mut SyncStartDetector, audio: &[f32]) -> Option<SyncStart> {
        let mut total = 0_u64;
        for chunk in audio.chunks(1024) {
            total += chunk.len() as u64;
            det.process(chunk, total);
            if let Some(s) = det.take_start() {
                return Some(s);
            }
        }
        None
    }

    fn robot36_image() -> Vec<f32> {
        let ycrcb = crate::robot_test_encoder::synthetic_ycrcb(SstvMode::Robot36);
        crate::robot_test_encoder::encode_robot(SstvMode::Robot36, &ycrcb)
    }

    /// The start the detector should report for a line beginning at
    /// absolute sample `line_start`.
    fn expected_start(spec: ModeSpec, line_start: usize) -> f64 {
        line_start as f64 - START_LEAD_SYNC_PULSES * spec.sync_seconds * RATE as f64
    }

    #[test]
    fn noise_never_starts_an_image() {
        let mut det = SyncStartDetector::new(for_mode(SstvMode::Robot36));
        assert_eq!(feed(&mut det, &noise(60 * RATE, 0xBEEF)), None);
        assert!(det.last_quality() < START_QUALITY);
    }

    #[test]
    fn a_pulse_burst_shorter_than_the_window_starts_nothing() {
        let spec = for_mode(SstvMode::Robot36);
        let line = (spec.line_seconds * RATE as f64) as usize;
        let mut audio = noise(RATE, 3);
        audio.extend(&robot36_image()[..5 * line]);
        audio.extend(noise(5 * RATE, 5));
        let mut det = SyncStartDetector::new(spec);
        assert_eq!(feed(&mut det, &audio), None);
    }

    #[test]
    fn a_headerless_robot36_image_starts_at_its_first_line() {
        let spec = for_mode(SstvMode::Robot36);
        let lead = RATE; // 1 s of noise
        let mut audio = noise(lead, 7);
        audio.extend(robot36_image());
        let mut det = SyncStartDetector::new(spec);
        let start = feed(&mut det, &audio).expect("rhythm detected");
        assert!(start.quality >= START_QUALITY);
        let err = start.start_sample as f64 - expected_start(spec, lead);
        assert!(err.abs() < 30.0, "start off by {err:.0} samples");
        let handed = det.take_audio_from(start.start_sample);
        let at = start.start_sample as usize;
        assert_eq!(
            handed[..64],
            audio[at..at + 64],
            "audio handed from the start"
        );
        assert_eq!(det.take_start(), None, "a start is reported once");
    }

    #[test]
    fn a_mid_image_join_starts_on_a_line_boundary() {
        let spec = for_mode(SstvMode::Robot36);
        let line = (spec.line_seconds * RATE as f64).round() as usize;
        let lead = RATE / 2;
        let mut audio = noise(lead, 9);
        audio.extend(&robot36_image()[100 * line..]);
        let mut det = SyncStartDetector::new(spec);
        let start = feed(&mut det, &audio).expect("rhythm detected");
        let err = start.start_sample as f64 - expected_start(spec, lead);
        assert!(err.abs() < 30.0, "start off by {err:.0} samples");
    }

    #[test]
    fn a_robot36_join_on_an_odd_line_backs_off_to_the_even_line_before_it() {
        // R36 decodes Cr on even rows and Cb on odd ones by row index, so
        // an image must start on an even line or its chroma swaps.
        let spec = for_mode(SstvMode::Robot36);
        let line = (spec.line_seconds * RATE as f64).round() as usize;
        let lead = RATE / 2;
        let mut audio = noise(lead, 13);
        audio.extend(&robot36_image()[101 * line..]);
        let mut det = SyncStartDetector::new(spec);
        let start = feed(&mut det, &audio).expect("rhythm detected");
        let err = start.start_sample as f64 - expected_start(spec, lead - line);
        assert!(err.abs() < 30.0, "start off by {err:.0} samples");
    }

    /// Narrow-pulse modes (every mode under `WIDE_PULSE_COLUMNS`) — SC-2's
    /// 5.5 ms pulse is ~8 of 700 columns on its 475 ms line, Martin 1's ~8,
    /// Scottie DX's ~6 — let a noise line fake
    /// "half the pulse columns hit", so the first line must stand out from
    /// its own background on consecutive lines (#128 review). A noise line
    /// fakes it only by chance, so many seeds are tried per mode.
    #[test]
    fn narrow_pulse_modes_start_at_their_first_line_not_on_noise() {
        let lead = RATE;
        let narrow: Vec<ModeSpec> = crate::modespec::ALL_SPECS
            .into_iter()
            .filter(|s| sync_pulse_columns(*s, X_ACC_BINS) < WIDE_PULSE_COLUMNS)
            .collect();
        // PD180/240, Scottie 1/DX, Martin 1/2 and Wraase SC-2 120/180 today.
        assert_eq!(narrow.len(), 8, "narrow-pulse modes changed: {narrow:?}");
        // Fewer seeds in debug, where the fast CI job runs this.
        let seeds: u32 = if cfg!(debug_assertions) { 4 } else { 12 };
        for spec in narrow {
            let mode = spec.mode;
            let pixels = vec![[128_u8, 64, 200]; (spec.line_pixels * spec.image_lines) as usize];
            let mut image = match spec.channel_layout {
                crate::modespec::ChannelLayout::PdYcbcr => {
                    crate::pd_test_encoder::encode_pd(mode, &pixels)
                }
                _ => crate::scottie_test_encoder::encode_scottie(mode, &pixels),
            };
            // The detector needs ~8 lines; keep 12.
            image.truncate((12.0 * spec.line_seconds * RATE as f64) as usize);
            let misplaced: Vec<(u32, f64)> = (0..seeds)
                .filter_map(|seed| {
                    let mut audio = noise(lead, 1000 + seed);
                    audio.extend(&image);
                    let mut det = SyncStartDetector::new(spec);
                    let start = feed(&mut det, &audio).expect("rhythm detected");
                    let err = start.start_sample as f64 - expected_start(spec, lead);
                    (err.abs() >= 40.0).then_some((seed, err))
                })
                .collect();
            assert!(
                misplaced.is_empty(),
                "{mode:?} (seed, samples off): {misplaced:?}"
            );
        }
    }

    /// A probe track for `spec` whose pulse (at `column`) is present from
    /// window line `from` on: solid runs when `solid`, else hits on every
    /// other probe (fragmented — column counts look fine, runs don't).
    fn pulse_track(spec: ModeSpec, column: usize, from: usize, solid: bool) -> Vec<bool> {
        let det = SyncStartDetector::new(spec);
        let line = det.line_samples() / SYNC_PROBE_STRIDE as f64;
        let pulse = (spec.sync_seconds * RATE as f64 / SYNC_PROBE_STRIDE as f64) as usize;
        let mut probes = vec![false; det.window_probes()];
        for y in from..det.window_lines {
            let start = ((y as f64 + column as f64 / X_ACC_BINS as f64) * line) as usize;
            for (k, p) in probes.iter_mut().skip(start).take(pulse).enumerate() {
                *p = solid || k % 2 == 0;
            }
        }
        probes
    }

    #[test]
    fn a_narrow_pulse_image_starts_at_its_first_solidly_pulsed_line() {
        let spec = for_mode(SstvMode::Wraase120);
        let mut det = SyncStartDetector::new(spec);
        det.probes = pulse_track(spec, 100, 2, true);
        assert_eq!(det.first_pulsed_line(0, 100, 8), Some(2));
    }

    /// The quality gate counts column hits, so a track of fragmented pulses
    /// can pass it with no line pair carrying an unbroken run. Rather than
    /// guess window line 0 (possibly noise), report no start yet (#131
    /// review); a later evaluation sees more of the image.
    #[test]
    fn fragmented_narrow_pulses_report_no_first_line() {
        let spec = for_mode(SstvMode::Wraase120);
        let mut det = SyncStartDetector::new(spec);
        det.probes = pulse_track(spec, 100, 0, false);
        assert_eq!(det.first_pulsed_line(0, 100, 8), None);
    }

    #[test]
    fn a_headerless_scottie1_image_starts_at_its_line_start_not_its_mid_line_pulse() {
        let spec = for_mode(SstvMode::Scottie1);
        let lead = RATE;
        let rgb = vec![[128_u8, 64, 200]; (spec.line_pixels * spec.image_lines) as usize];
        let mut audio = noise(lead, 11);
        audio.extend(crate::scottie_test_encoder::encode_scottie(
            SstvMode::Scottie1,
            &rgb,
        ));
        let mut det = SyncStartDetector::new(spec);
        let start = feed(&mut det, &audio).expect("rhythm detected");
        // The decoder's Scottie line convention sits ~0.75 ms from the
        // encoder's (see `pulse_offset_seconds`).
        let err = start.start_sample as f64 - expected_start(spec, lead);
        assert!(err.abs() < 30.0, "start off by {err:.0} samples");
    }

    #[test]
    fn short_modes_fold_at_least_two_seconds() {
        assert_eq!(window_lines(for_mode(SstvMode::Robot36)), 14);
        assert_eq!(window_lines(for_mode(SstvMode::Pd120)), MIN_WINDOW_LINES);
    }
}
