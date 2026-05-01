//! # slowrx
//!
//! Pure-Rust SSTV decoder library — a port of
//! [slowrx](https://github.com/windytan/slowrx) by Oona Räisänen (OH2EIQ).
//! Significant portions of the algorithms are translated from the C source.
//! See the [NOTICE file] for full attribution and license preservation.
//!
//! ## Status
//!
//! Pre-0.1 — under active development. Public API is not yet stable.
//! See <https://github.com/jasonherald/slowrx.rs> for the implementation roadmap.
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

pub mod decoder;
pub mod error;
pub mod image;
pub mod mode_pd;
pub mod modespec;
pub mod resample;
pub mod vis;

pub use crate::decoder::{SstvDecoder, SstvEvent};
pub use crate::error::{Error, Result};
pub use crate::image::SstvImage;
pub use crate::modespec::{for_mode, lookup as lookup_vis, ChannelLayout, ModeSpec, SstvMode};
pub use crate::resample::{Resampler, WORKING_SAMPLE_RATE_HZ};
