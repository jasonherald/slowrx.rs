//! Error types for the slowrx crate.
//!
//! Library-only failure modes — anything I/O or codec-shaped belongs to
//! the caller (CLI, examples, integration tests use their own wrappers).

/// Crate-wide error type.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Caller-supplied sample rate is outside the supported range.
    #[error("invalid sample rate: {got} (must be > 0 and ≤ 192000)")]
    InvalidSampleRate {
        /// The rate the caller passed.
        got: u32,
    },

    /// VIS code does not map to a known SSTV mode.
    ///
    /// The `u8` value is the raw 7-bit VIS byte read from the audio stream.
    #[error("VIS code {0:#04x} does not map to a known SSTV mode")]
    UnknownVisCode(u8),
}

/// Convenient `Result` alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_sample_rate_renders_with_value() {
        let e = Error::InvalidSampleRate { got: 0 };
        assert_eq!(
            e.to_string(),
            "invalid sample rate: 0 (must be > 0 and ≤ 192000)"
        );
    }

    #[test]
    fn unknown_vis_code_renders_in_hex() {
        let e = Error::UnknownVisCode(0x42);
        assert_eq!(
            e.to_string(),
            "VIS code 0x42 does not map to a known SSTV mode"
        );
    }
}
