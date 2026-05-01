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
        /// Decoder-relative sample offset where VIS finished.
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
#[derive(Clone, Debug)]
enum State {
    AwaitingVis,
    // PR-1 fills in:
    // Decoding { mode: SstvMode, line: u32, line_start_sample: u64 },
}

/// Streaming SSTV decoder. Push audio buffers in via
/// [`Self::process`]; consume the returned events.
pub struct SstvDecoder {
    resampler: Resampler,
    state: State,
    samples_processed: u64,
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
            state: State::AwaitingVis,
            samples_processed: 0,
        })
    }

    /// Process a chunk of mono `f32` audio samples in caller's rate.
    ///
    /// Returns events produced during this call's processing window.
    /// In PR-0 this returns an empty `Vec` (decoder is a state-machine
    /// shell); PR-1/PR-2 plug in real detection + decoding.
    pub fn process(&mut self, audio: &[f32]) -> Vec<SstvEvent> {
        let _resampled = self.resampler.process(audio);
        self.samples_processed = self.samples_processed.saturating_add(audio.len() as u64);
        // PR-1 + PR-2 fill in real event production.
        Vec::new()
    }

    /// Reset to `AwaitingVis`; discard any in-flight image.
    pub fn reset(&mut self) {
        self.state = State::AwaitingVis;
        self.samples_processed = 0;
    }

    /// Total samples processed since construction (or last `reset`).
    #[must_use]
    pub fn samples_processed(&self) -> u64 {
        self.samples_processed
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::resample::MAX_INPUT_SAMPLE_RATE_HZ;

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
    fn process_returns_no_events_in_pr0_skeleton() {
        let mut d = SstvDecoder::new(11_025).expect("decoder");
        // Even with audio fed in, PR-0 emits nothing.
        let events = d.process(&[0.5_f32; 512]);
        assert!(events.is_empty());
    }

    #[test]
    fn reset_clears_sample_counter() {
        let mut d = SstvDecoder::new(11_025).expect("decoder");
        let _ = d.process(&[0.0_f32; 1024]);
        d.reset();
        assert_eq!(d.samples_processed(), 0);
    }
}
