//! # slowrx
//!
//! Pure-Rust SSTV decoder library — a port of
//! [slowrx](https://github.com/windytan/slowrx) by Oona Räisänen (OH2EIQ).
//! Significant portions of the algorithms are translated from the C source.
//! See the [NOTICE file] for full attribution and license preservation.
//!
//! ## Status
//!
//! `0.3.x` — V2.2 published. PD120/PD180/PD240 + Robot 24/36/72
//! decoding from raw audio. PD120/PD180 validated against ARISS
//! Dec-2017; Robot 36 validated against the ARISS Fram2 corpus
//! (see `tests/ariss_fram2_validation.md`). The public API is
//! `#[non_exhaustive]`-protected for additive growth as V2.x
//! mode-family epics land. See
//! <https://github.com/jasonherald/slowrx.rs/issues/9> for the V2 roadmap.
//!
//! ## Example
//!
//! ```
//! # use slowrx::Error;
//! use slowrx::SstvDecoder;
//!
//! // Construct a decoder at the caller's audio sample rate.
//! let mut decoder = SstvDecoder::new(44_100)?;
//!
//! // Feed audio chunks; consume any events that come back.
//! let audio = vec![0.0_f32; 1024];
//! let _events = decoder.process(&audio);
//! # Ok::<(), Error>(())
//! ```
//!
//! [NOTICE file]: https://github.com/jasonherald/slowrx.rs/blob/main/NOTICE.md

#![warn(missing_docs)]

pub mod decoder;
pub mod error;
pub mod image;
pub mod mode_pd;
pub mod mode_robot;
pub mod mode_scottie;
pub mod modespec;
pub mod resample;
#[allow(dead_code)]
pub(crate) mod snr;
pub(crate) mod sync;
pub mod vis;

/// Translate a frequency in Hz to the nearest FFT bin index using slowrx's
/// C-truncation semantics.
///
/// slowrx's `GetBin` (`common.c:39-41`) is:
/// ```c
/// guint GetBin(double Freq, guint FFTLen) {
///     return (Freq / 44100 * FFTLen);  // implicit double→uint = truncation toward zero
/// }
/// ```
///
/// The implicit `double → guint` cast truncates toward zero.  We replicate
/// this with an `as usize` cast (well-defined for positive doubles: truncates
/// toward zero), which gives the same result as C for all frequencies used
/// in slowrx. **Do NOT change this to `.round()`** — that would deviate from
/// slowrx's bin assignments at 5 of the 8 production frequencies (800, 1200,
/// 1500, 2700, 3400 Hz), shifting SNR-estimator bandwidth divisors and the
/// sync tracker's `Praw`/`Psync` range.
///
/// # Numerical verification (both at slowrx-native 1024/44100 and our 256/11025
/// — same Hz/bin ratio, so bins are identical)
///
/// | Frequency | Expected bin |
/// |-----------|-------------|
/// | 400 Hz    | 9           |
/// | 800 Hz    | 18          |
/// | 1200 Hz   | 27          |
/// | 1500 Hz   | 34          |
/// | 1900 Hz   | 44          |
/// | 2300 Hz   | 53          |
/// | 2700 Hz   | 62          |
/// | 3400 Hz   | 78          |
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
#[inline]
pub(crate) fn get_bin(hz: f64, fft_len: usize, sample_rate_hz: u32) -> usize {
    (hz * fft_len as f64 / f64::from(sample_rate_hz)) as usize
}

#[cfg(test)]
mod tests_common {
    use super::*;

    /// Verify `get_bin` truncation matches slowrx's C `guint GetBin(double, guint)`.
    ///
    /// Both FFT-size/sample-rate pairs share the same Hz/bin ratio
    /// (256/11025 ≈ 1024/44100), so they should produce identical bin indices.
    #[test]
    fn get_bin_matches_slowrx_truncation() {
        // Pairs of (freq_hz, expected_bin) derived from GetBin at slowrx's
        // FFTLen=1024, SR=44100 — identical at our FFTLen=256, SR=11025.
        let cases: &[(f64, usize)] = &[
            (400.0, 9),
            (800.0, 18),
            (1190.0, 27),
            (1200.0, 27),
            (1500.0, 34),
            (1900.0, 44),
            (2300.0, 53),
            (2700.0, 62),
            (3400.0, 78),
        ];
        for &(hz, expected) in cases {
            // Our working rate (256/11025)
            let bin_ours = get_bin(hz, 256, 11025);
            // slowrx native rate (1024/44100) — same ratio, should be identical
            let bin_slowrx = get_bin(hz, 1024, 44100);
            assert_eq!(
                bin_ours, expected,
                "get_bin({hz}, 256, 11025) = {bin_ours}, expected {expected}"
            );
            assert_eq!(
                bin_slowrx, expected,
                "get_bin({hz}, 1024, 44100) = {bin_slowrx}, expected {expected}"
            );
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod pd_test_encoder;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod robot_test_encoder;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod scottie_test_encoder;

pub use crate::decoder::{SstvDecoder, SstvEvent};
pub use crate::error::{Error, Result};
pub use crate::image::SstvImage;
pub use crate::modespec::{
    for_mode, lookup as lookup_vis, ChannelLayout, ModeSpec, SstvMode, SyncPosition,
};
pub use crate::resample::{Resampler, WORKING_SAMPLE_RATE_HZ};

/// Test-support — exposed under the `test-support` feature for integration
/// tests in this crate (e.g., `tests/roundtrip.rs`). NOT part of the stable
/// public API; will be hidden behind `#[doc(hidden)]` until V1 publishes.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod __test_support {
    pub mod vis {
        pub use crate::vis::tests::synth_vis;
    }
    pub mod mode_pd {
        pub use crate::mode_pd::ycbcr_to_rgb;
        pub use crate::pd_test_encoder::encode_pd;
    }
    pub mod mode_robot {
        pub use crate::robot_test_encoder::encode_robot;
    }
    pub mod mode_scottie {
        pub use crate::scottie_test_encoder::encode_scottie;
    }
}
