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
//! [NOTICE file]: https://github.com/jasonherald/slowrx.rs/blob/main/NOTICE.md

pub mod error;

pub use crate::error::{Error, Result};
