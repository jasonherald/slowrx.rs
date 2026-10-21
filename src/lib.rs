//! # slowrx
//!
//! Pure-Rust SSTV decoder library — a port of
//! [slowrx](https://github.com/windytan/slowrx) by Oona Räisänen (OH2EIQ).
//! Significant portions of the algorithms are translated from the C source.
//! See the [NOTICE file] for full attribution and license preservation.
//!
//! ## Status
//!
//! PD120 / PD180 / PD240, Robot 24 / 36 / 72, Robot 24 / 12 / 8 B/W,
//! Scottie 1 / 2 / DX, Martin 1 / 2, and Wraase SC-2 120 / 180 decode from
//! raw audio. PD120 and PD180 are validated against the ARISS Dec-2017
//! capture set; Robot 36 is validated against the ARISS Fram2 corpus (see
//! [`tests/ariss_fram2_validation.md`][fram2]). The Scottie, Martin,
//! Wraase SC-2 and Robot B/W modes are synthetic round-trip-validated only
//! — no reference WAVs are available for them. The public API is
//! `#[non_exhaustive]`-protected for additive growth. See
//! <https://github.com/jasonherald/slowrx.rs/issues/9> for the
//! roadmap.
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
//! [fram2]: https://github.com/jasonherald/slowrx.rs/blob/main/tests/ariss_fram2_validation.md

#![warn(missing_docs)]

pub(crate) mod demod;
pub(crate) mod dsp;

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
pub(crate) mod sync_start;
pub mod vis;
#[cfg(feature = "cli")]
pub mod wav;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_tone;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod pd_test_encoder;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod robot_test_encoder;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod scottie_test_encoder;

pub use crate::decoder::{SstvDecoder, SstvEvent};
pub use crate::error::{Error, Result};
pub use crate::image::{SstvImage, SyncQuality};
pub use crate::modespec::{
    for_mode, lookup as lookup_vis, ChannelLayout, ColorEncoding, ModeSpec, SstvMode, SyncPosition,
};
pub use crate::resample::{Resampler, MAX_INPUT_SAMPLE_RATE_HZ, WORKING_SAMPLE_RATE_HZ};

/// Test-support — exposed under the `test-support` feature for integration
/// tests in this crate (e.g., `tests/roundtrip.rs`). NOT part of the stable
/// public API; the module is `#[doc(hidden)]` and the items inside are thin
/// wrappers around `pub(crate)` internals. The genuinely public types they
/// wrap (`SstvMode`, `ChannelLayout`, `SyncPosition`, `SstvEvent`, etc.)
/// carry `#[non_exhaustive]` so additive growth there is non-breaking.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod __test_support {
    pub mod vis {
        pub use crate::vis::tests::{lost_bits_header, synth_vis};
    }
    pub mod noise {
        /// Deterministic full-scale white noise — `n` samples, LCG-seeded.
        #[doc(hidden)]
        #[must_use]
        pub fn white(n: usize, seed: u32) -> Vec<f32> {
            crate::test_tone::white_noise(n, seed)
        }
    }
    pub mod mode_pd {
        pub use crate::demod::ycbcr_to_rgb;

        /// Thin wrapper around the now-`pub(crate)` `crate::pd_test_encoder::encode_pd`.
        /// `__test_support` is the sole consumer-facing path for the synthetic
        /// PD encoder (#86 B10).
        #[doc(hidden)]
        #[must_use]
        pub fn encode_pd(mode: crate::modespec::SstvMode, ycrcb: &[[u8; 3]]) -> Vec<f32> {
            crate::pd_test_encoder::encode_pd(mode, ycrcb)
        }
    }
    pub mod mode_robot {
        /// Thin wrapper around the now-`pub(crate)` `crate::robot_test_encoder::encode_robot`.
        /// `__test_support` is the sole consumer-facing path for the synthetic
        /// Robot encoder (#86 B10). Covers the colour Robot modes and
        /// Robot 24 / 12 / 8 B/W (which send only the `Y` of each triple).
        #[doc(hidden)]
        #[must_use]
        pub fn encode_robot(mode: crate::modespec::SstvMode, ycrcb: &[[u8; 3]]) -> Vec<f32> {
            crate::robot_test_encoder::encode_robot(mode, ycrcb)
        }

        /// The standard synthetic Robot test image (`Y'CbCr`) the
        /// round-trip and fallback tests encode.
        #[doc(hidden)]
        #[must_use]
        pub fn synthetic_ycrcb(mode: crate::modespec::SstvMode) -> Vec<[u8; 3]> {
            crate::robot_test_encoder::synthetic_ycrcb(mode)
        }
    }
    pub mod mode_scottie {
        /// Thin wrapper around the now-`pub(crate)` `crate::scottie_test_encoder::encode_scottie`.
        /// `__test_support` is the sole consumer-facing path for the synthetic
        /// Scottie/Martin/Wraase SC-2 encoder (#86 B10).
        #[doc(hidden)]
        #[must_use]
        pub fn encode_scottie(mode: crate::modespec::SstvMode, rgb: &[[u8; 3]]) -> Vec<f32> {
            crate::scottie_test_encoder::encode_scottie(mode, rgb)
        }
    }
}
