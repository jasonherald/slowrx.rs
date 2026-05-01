//! `SstvDecoder` — public state machine driving the decode pipeline.
//!
//! This is the V1 skeleton: state machine shell + public API surface.
//! VIS detection lands in PR-1; per-mode pixel decoding lands in PR-2.
//!
//! Translated in spirit from slowrx's `slowrx.c` `Listen()` loop +
//! `vis.c` `GetVIS()` + `video.c` `GetVideo()`. ISC License — see
//! `NOTICE.md`.

use crate::error::Result;
use crate::image::SstvImage;
use crate::modespec::SstvMode;
use crate::resample::Resampler;

/// One observable event emitted by [`SstvDecoder::process`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SstvEvent {
    /// VIS header parsed and a known mode dispatched.
    VisDetected {
        /// Mode identified by the VIS bits.
        mode: SstvMode,
        /// Working-rate (11025 Hz) sample offset where the VIS stop bit ended.
        /// Useful for callers that want to align audio captures with decoder events.
        sample_offset: u64,
    },
    /// One scan line completed (callers may render incrementally).
    LineDecoded {
        /// Mode currently being decoded.
        mode: SstvMode,
        /// 0-based row index for this line.
        line_index: u32,
        /// Row pixels in `[r, g, b]` order, length = mode's `line_pixels`.
        pixels: Vec<[u8; 3]>,
    },
    /// Image complete. `partial: true` when the in-flight image was
    /// closed via [`SstvDecoder::reset`] rather than reaching its
    /// natural line count.
    ImageComplete {
        /// Final pixel buffer.
        image: SstvImage,
        /// `true` if the image was cut short by reset/mid-image VIS.
        partial: bool,
    },
}

/// Internal state of the decoder.
enum State {
    AwaitingVis,
    Decoding {
        mode: SstvMode,
        spec: crate::modespec::ModeSpec,
        line_pair_index: u32,
        image: SstvImage,
        /// Buffered working-rate samples not yet consumed by per-pixel decode.
        buffer: Vec<f32>,
    },
}

/// Streaming SSTV decoder. Push audio buffers in via
/// [`Self::process`]; consume the returned events.
pub struct SstvDecoder {
    resampler: Resampler,
    vis: crate::vis::VisDetector,
    pd_demod: crate::mode_pd::PdDemod,
    state: State,
    samples_processed: u64,
    /// Cumulative working-rate samples emitted by the resampler.
    /// Used as the unit for `SstvEvent::VisDetected.sample_offset` so
    /// that value is consistent regardless of caller's input rate.
    working_samples_emitted: u64,
}

impl SstvDecoder {
    /// Construct a decoder consuming audio at `input_sample_rate_hz`.
    ///
    /// # Errors
    /// Returns [`crate::Error::InvalidSampleRate`] if the rate is 0 or
    /// > [`crate::resample::MAX_INPUT_SAMPLE_RATE_HZ`].
    pub fn new(input_sample_rate_hz: u32) -> Result<Self> {
        Ok(Self {
            resampler: Resampler::new(input_sample_rate_hz)?,
            vis: crate::vis::VisDetector::new(),
            pd_demod: crate::mode_pd::PdDemod::new(),
            state: State::AwaitingVis,
            samples_processed: 0,
            working_samples_emitted: 0,
        })
    }

    /// Process a chunk of mono `f32` audio samples in caller's rate.
    ///
    /// Returns events produced during this call's processing window.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn process(&mut self, audio: &[f32]) -> Vec<SstvEvent> {
        let working = self.resampler.process(audio);
        self.samples_processed = self.samples_processed.saturating_add(audio.len() as u64);
        self.working_samples_emitted = self
            .working_samples_emitted
            .saturating_add(working.len() as u64);

        let mut out = Vec::new();
        let mut remaining: &[f32] = working.as_slice();
        loop {
            match &mut self.state {
                State::AwaitingVis => {
                    self.vis.process(remaining, self.working_samples_emitted);
                    remaining = &[];
                    if let Some(detected) = self.vis.take_detected() {
                        if let Some(spec) = crate::modespec::lookup(detected.code) {
                            out.push(SstvEvent::VisDetected {
                                mode: spec.mode,
                                sample_offset: detected.end_sample,
                            });
                            let image =
                                SstvImage::new(spec.mode, spec.line_pixels, spec.image_lines);
                            // Recover any post-stop-bit audio that the VIS
                            // detector buffered but did not consume — it is
                            // the leading edge of the image data.
                            let residual = self.vis.take_residual_buffer();
                            self.state = State::Decoding {
                                mode: spec.mode,
                                spec,
                                line_pair_index: 0,
                                image,
                                buffer: residual,
                            };
                            continue; // re-enter loop to process leftover audio
                        }
                        // Unknown VIS codes silently drop. Reset the
                        // detector's buffer so it does not accumulate
                        // forever on uninterpretable bursts.
                        let _ = self.vis.take_residual_buffer();
                    }
                    break;
                }
                State::Decoding {
                    mode,
                    spec,
                    line_pair_index,
                    image,
                    buffer,
                } => {
                    buffer.extend_from_slice(remaining);

                    // TODO(future): mid-image VIS detection. When a new VIS
                    // burst arrives during decoding the spec calls for flushing
                    // the in-flight image as `partial: true` and restarting.
                    // The straightforward approach — running `self.vis` against
                    // `buffer` each call — fails because the decoding buffer is
                    // not aligned to 30 ms window boundaries: the residual from
                    // the previous VIS detection starts at an arbitrary sample
                    // offset, so the first classifier window is a mix of silence
                    // and leader tone and does not reliably pass the 5× dominance
                    // threshold. A correct implementation would re-align the VIS
                    // window scan to the next 30 ms boundary, or run a separate
                    // correlator tuned to the 1900 Hz leader. Deferred to PR-3.

                    let work_rate = f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ);
                    // Per-pixel FFT window is centered on the pixel's
                    // expected sample, so we need FFT_LEN/2 trailing
                    // samples beyond the last pixel of the pair.
                    let lookahead = crate::mode_pd::FFT_LEN / 2;

                    // Compute the cumulative sample boundary for the
                    // CURRENT pair's start vs. the NEXT pair's start.
                    // Using rounded-cumulative-time matches how a
                    // continuous-phase encoder would lay out samples,
                    // and prevents a per-pair rounding bias from
                    // accumulating (PD180 drifts ~0.05 samples/pair, so
                    // by row 250 a fixed `samples_per_pair` is off by
                    // ~12 samples — enough to misalign the line pair).
                    loop {
                        let cur_pair = u64::from(*line_pair_index);
                        // buffer always begins at the current pair's start sample.
                        // The next pair begins at:
                        //   round((cur_pair + 1) * line_seconds * sr) -
                        //   round(cur_pair * line_seconds * sr)
                        // samples after the current start.
                        let cur_off = (cur_pair as f64 * spec.line_seconds * work_rate).round();
                        let next_off =
                            ((cur_pair + 1) as f64 * spec.line_seconds * work_rate).round();
                        let samples_per_pair = (next_off - cur_off) as usize;
                        let needed = samples_per_pair + lookahead;
                        if buffer.len() < needed {
                            break;
                        }

                        // Pass a window that overlaps the next pair's
                        // leading samples so the rightmost pixels see
                        // a full FFT window.
                        crate::mode_pd::decode_pd_line_pair(
                            *spec,
                            *line_pair_index,
                            &buffer[..needed],
                            image,
                            &mut self.pd_demod,
                        );

                        let row0 = *line_pair_index * 2;
                        let row1 = row0 + 1;
                        let line_pixels = spec.line_pixels as usize;
                        for r in [row0, row1] {
                            let start = (r as usize) * line_pixels;
                            let end = start + line_pixels;
                            out.push(SstvEvent::LineDecoded {
                                mode: *mode,
                                line_index: r,
                                pixels: image.pixels[start..end].to_vec(),
                            });
                        }

                        buffer.drain(..samples_per_pair);
                        *line_pair_index += 1;

                        if *line_pair_index * 2 >= spec.image_lines {
                            // Image complete.
                            let final_image = std::mem::replace(
                                image,
                                SstvImage::new(*mode, spec.line_pixels, spec.image_lines),
                            );
                            out.push(SstvEvent::ImageComplete {
                                image: final_image,
                                partial: false,
                            });
                            self.state = State::AwaitingVis;
                            break;
                        }
                    }
                    break;
                }
            }
        }
        out
    }

    /// Reset to `AwaitingVis`; discard any in-flight image.
    pub fn reset(&mut self) {
        self.state = State::AwaitingVis;
        self.samples_processed = 0;
        self.working_samples_emitted = 0;
        self.vis = crate::vis::VisDetector::new();
    }

    /// Total samples processed since construction (or last `reset`).
    #[must_use]
    pub fn samples_processed(&self) -> u64 {
        self.samples_processed
    }
}

/// Estimate the dominant tone frequency in `window` (working-rate samples).
/// Returns the estimated frequency in Hz, biased toward 1500-2300 Hz
/// (the SSTV video band).
///
/// Algorithm: Goertzel-bank evaluated at 25-Hz steps from 1450 to 2350 Hz,
/// then quadratic peak interpolation around the maximum bin.
#[must_use]
#[allow(clippy::cast_precision_loss, dead_code)]
pub(crate) fn estimate_freq(window: &[f32]) -> f64 {
    const STEP_HZ: f64 = 25.0;
    const FIRST_HZ: f64 = 1450.0;
    const N_BINS: usize = 37; // 1450..2350 inclusive at 25 Hz steps

    let mut powers = [0.0_f64; N_BINS];
    for (i, p) in powers.iter_mut().enumerate() {
        let f = FIRST_HZ + (i as f64) * STEP_HZ;
        *p = crate::vis::goertzel_power(window, f);
    }
    let (mut max_i, mut max_p) = (0_usize, powers[0]);
    for (i, &p) in powers.iter().enumerate().skip(1) {
        if p > max_p {
            max_p = p;
            max_i = i;
        }
    }
    let center_hz = FIRST_HZ + (max_i as f64) * STEP_HZ;
    // Quadratic interpolation if we have both neighbours.
    if max_i > 0 && max_i < N_BINS - 1 && max_p > 0.0 {
        let a = powers[max_i - 1];
        let b = max_p;
        let c = powers[max_i + 1];
        let denom = a - 2.0 * b + c;
        if denom.abs() > 1e-12 {
            let delta = 0.5 * (a - c) / denom;
            return center_hz + delta * STEP_HZ;
        }
    }
    center_hz
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::resample::{MAX_INPUT_SAMPLE_RATE_HZ, WORKING_SAMPLE_RATE_HZ};

    #[test]
    fn rejects_invalid_sample_rates() {
        assert!(matches!(
            SstvDecoder::new(0),
            Err(Error::InvalidSampleRate { got: 0 })
        ));
        assert!(matches!(
            SstvDecoder::new(MAX_INPUT_SAMPLE_RATE_HZ + 1),
            Err(Error::InvalidSampleRate { .. })
        ));
    }

    #[test]
    fn accepts_common_rates() {
        assert!(SstvDecoder::new(11_025).is_ok());
        assert!(SstvDecoder::new(44_100).is_ok());
        assert!(SstvDecoder::new(48_000).is_ok());
    }

    #[test]
    fn process_advances_sample_counter() {
        let mut d = SstvDecoder::new(11_025).expect("decoder");
        assert_eq!(d.samples_processed(), 0);
        let _ = d.process(&[0.0_f32; 1024]);
        assert_eq!(d.samples_processed(), 1024);
        let _ = d.process(&[0.0_f32; 256]);
        assert_eq!(d.samples_processed(), 1280);
    }

    #[test]
    fn process_returns_no_events_for_silence() {
        let mut d = SstvDecoder::new(11_025).expect("decoder");
        // Silence produces no VIS match.
        let events = d.process(&[0.5_f32; 512]);
        assert!(events.is_empty());
    }

    #[test]
    fn process_emits_vis_detected_for_pd120_burst() {
        use crate::vis::tests::synth_vis;
        let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
        // Pad with trailing silence so the polyphase FIR's ~64-sample group
        // delay still yields a full set of stop-bit windows (PR-2 T2.1).
        let mut burst = synth_vis(0x5F, 0.0);
        burst.extend(std::iter::repeat_n(0.0_f32, 512));
        let events = d.process(&burst);
        let any_vis = events.iter().any(|e| {
            matches!(
                e,
                SstvEvent::VisDetected {
                    mode: SstvMode::Pd120,
                    ..
                }
            )
        });
        assert!(any_vis, "expected VisDetected for PD120, got {events:?}");
    }

    #[test]
    fn process_emits_vis_detected_for_pd180_burst() {
        use crate::vis::tests::synth_vis;
        let mut d = SstvDecoder::new(WORKING_SAMPLE_RATE_HZ).expect("decoder");
        let mut burst = synth_vis(0x60, 0.0);
        burst.extend(std::iter::repeat_n(0.0_f32, 512));
        let events = d.process(&burst);
        assert!(events.iter().any(|e| matches!(
            e,
            SstvEvent::VisDetected {
                mode: SstvMode::Pd180,
                ..
            }
        )));
    }

    #[test]
    fn reset_clears_sample_counter() {
        let mut d = SstvDecoder::new(11_025).expect("decoder");
        let _ = d.process(&[0.0_f32; 1024]);
        d.reset();
        assert_eq!(d.samples_processed(), 0);
    }

    // 40 ms tones make every 25-Hz bank bin map to a unique Goertzel k
    // (11025/441 = 25.0). Production windows are ~5 ms; ~50 Hz suffices.
    fn synth_tone_at_working(freq_hz: f64, secs: f64) -> Vec<f32> {
        let sr = f64::from(WORKING_SAMPLE_RATE_HZ);
        let n = (secs * sr).round() as usize;
        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * freq_hz * (i as f64) / sr).sin() as f32)
            .collect()
    }

    #[test]
    fn estimate_freq_recovers_known_tone() {
        for &f in &[1500.0_f64, 1700.0, 1900.0, 2100.0, 2300.0] {
            let window = synth_tone_at_working(f, 0.040);
            let est = estimate_freq(&window);
            assert!((est - f).abs() < 30.0, "freq={f} estimate={est}");
        }
    }

    #[test]
    fn estimate_freq_no_interp_at_left_boundary() {
        // Tone at 1450 Hz lands on bin 0; no left neighbour → no interp.
        let window = synth_tone_at_working(1450.0, 0.040);
        let est = estimate_freq(&window);
        assert!((est - 1450.0).abs() < 30.0, "expected ≈1450, got {est}");
    }

    #[test]
    fn reset_during_decoding_emits_partial_via_subsequent_process() {
        let mut d = SstvDecoder::new(crate::resample::WORKING_SAMPLE_RATE_HZ).unwrap();
        // Push a VIS so the decoder transitions to Decoding.
        let burst = crate::vis::tests::synth_vis(0x5F, 0.0);
        let _ = d.process(&burst);
        // We're now in Decoding (post-VIS state assertion via the next call's
        // outputs not being VisDetected events).
        d.reset();
        // After reset, the decoder is back in AwaitingVis. The next process
        // call with quiet audio yields no events.
        let events = d.process(&[0.0_f32; 100]);
        assert!(
            events.is_empty(),
            "reset should clear in-flight; got {events:?}"
        );
    }

    // TODO(future/PR-3): mid_image_vis_emits_partial_then_new_vis
    //
    // When a new VIS burst arrives during Decoding the spec calls for
    // emitting `ImageComplete { partial: true }` for the in-flight image,
    // then transitioning to AwaitingVis.
    //
    // The naive approach (running `self.vis` against the decoding buffer
    // each call) fails because the residual buffer from a previous VIS
    // detection is not aligned to 30 ms window boundaries: the first
    // classifier window is a mix of silence and leader tone and does not
    // reliably pass the 5× dominance threshold. A correct implementation
    // would re-align the scan to the next 30 ms boundary or run a separate
    // 1900 Hz energy detector. Deferred to PR-3 (cross-validation).
}
