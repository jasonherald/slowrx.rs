//! Internal rational resampler: caller's audio rate → 11025 Hz working rate.
//!
//! Hand-rolled polyphase FIR. We picked this over `rubato` because:
//! 1. Zero extra dependencies (the rest of the crate is `thiserror`-only).
//! 2. Fits comfortably under the ≤ 400 LOC cap on this file.
//! 3. Quality target is "audible loss < 0.1 dB across SSTV-relevant
//!    frequencies (1500-2300 Hz)" — easily met with a 64-tap windowed-sinc
//!    kernel at typical input rates (44.1k, 48k).
//!
//! Translated in spirit from slowrx's resampling done implicitly inside
//! `pcm.c`'s 44.1 kHz read loop.
//!
//! PR-2 Task 2.1 replaces this scaffold with the real polyphase FIR state.

use crate::error::{Error, Result};

/// Working sample rate the decoder operates at internally. Any caller
/// sample rate is resampled to this before processing.
pub const WORKING_SAMPLE_RATE_HZ: u32 = 11_025;

/// Maximum supported caller input sample rate.
pub const MAX_INPUT_SAMPLE_RATE_HZ: u32 = 192_000;

/// Polyphase FIR resampler. Stateful — accumulates input samples and
/// emits output samples at the working rate.
///
/// The FIR state fields will be added in PR-2 Task 2.1.
pub struct Resampler {
    input_rate: u32,
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
        Ok(Self { input_rate })
    }

    /// Push input samples; receive resampled output.
    ///
    /// The current implementation is a passthrough placeholder for PR-0.
    /// PR-2 Task 2.1 wires in the polyphase kernel.
    #[must_use]
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        // PR-0 placeholder: pass-through identity when input_rate
        // already matches the working rate, otherwise empty.
        if self.input_rate == WORKING_SAMPLE_RATE_HZ {
            input.to_vec()
        } else {
            Vec::new()
        }
    }

    /// Caller-provided input sample rate.
    #[must_use]
    pub fn input_rate(&self) -> u32 {
        self.input_rate
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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
        let mut r = Resampler::new(WORKING_SAMPLE_RATE_HZ).unwrap();
        let input = vec![0.1_f32, 0.2, 0.3, 0.4];
        assert_eq!(r.process(&input), input);
    }
}
