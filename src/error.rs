//! Error types for the slowrx crate.
//!
//! Library-only failure modes — anything I/O or codec-shaped belongs to
//! the caller (CLI, examples, integration tests use their own wrappers).

/// Crate-wide error type. Implements [`std::error::Error`] via `thiserror`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Caller-supplied sample rate is outside the supported range.
    #[error(
        "invalid sample rate: {got} (must be > 0 and ≤ {max})",
        max = crate::resample::MAX_INPUT_SAMPLE_RATE_HZ
    )]
    InvalidSampleRate {
        /// The rate the caller passed.
        got: u32,
    },
}

/// Convenient `Result` alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Compile-time assertion that `Error: Send + Sync + 'static`. Required by
/// `anyhow::Error` and `Box<dyn std::error::Error + Send + Sync + 'static>`
/// consumers; a future `Error` variant carrying a non-`Send` type would silently
/// break them, so we make the requirement load-bearing here. (Audit #92 C12.)
const _: fn() = || {
    fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    assert_send_sync_static::<Error>();
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_sample_rate_renders_with_value() {
        let e = Error::InvalidSampleRate { got: 0 };
        assert_eq!(
            e.to_string(),
            format!(
                "invalid sample rate: 0 (must be > 0 and ≤ {})",
                crate::resample::MAX_INPUT_SAMPLE_RATE_HZ
            )
        );
    }
}
