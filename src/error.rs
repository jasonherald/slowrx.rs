//! Error types for the slowrx crate.
//!
//! Library failure modes. The decoder itself only fails on bad
//! configuration; the WAV variants (with the `cli` feature) come from
//! [`crate::wav`]. Other I/O and codec work (PNG output, opening files)
//! belongs to the caller.

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

    /// The WAV's sample format is one [`crate::wav`] can't read: integer
    /// samples must be 8, 16, 24 or 32 bits, float samples 32 bits.
    /// (`cli` feature.)
    #[cfg(feature = "cli")]
    #[error(
        "unsupported {} bit depth: {bits}-bit",
        if *.float { "float" } else { "integer" }
    )]
    UnsupportedBitDepth {
        /// The WAV's bits per sample.
        bits: u16,
        /// `true` for floating-point samples, `false` for integer samples.
        float: bool,
    },

    /// The WAV declares no audio channels. `hound` refuses to open such
    /// files, so this only guards [`crate::wav::check_spec`] against a spec
    /// built by hand. (`cli` feature.)
    #[cfg(feature = "cli")]
    #[error("WAV has no audio channels")]
    NoAudioChannels,

    /// The WAV's bit depth is supported but its samples are padded into a
    /// wider container (e.g. 16-bit samples in 4-byte slots), which
    /// `hound` can't read. (`cli` feature.)
    #[cfg(feature = "cli")]
    #[error("unsupported sample container layout: {bits}-bit samples in a padded container")]
    UnsupportedSampleLayout {
        /// The WAV's bits per sample.
        bits: u16,
    },

    /// The WAV's data ended part-way through a multi-channel frame.
    /// `hound` already refuses to open such files, so this only guards
    /// [`crate::wav`] against a reader that doesn't. (`cli` feature.)
    #[cfg(feature = "cli")]
    #[error(
        "WAV ended mid-frame: read {samples} sample(s) of a {channels}-channel frame (truncated WAV?)"
    )]
    WavEndedMidFrame {
        /// Samples of the final frame that were present.
        samples: usize,
        /// Samples per frame (the WAV's channel count).
        channels: u16,
    },

    /// Reading a WAV sample failed: an I/O error (including a file
    /// shorter than its header says) or malformed data. (`cli` feature.)
    #[cfg(feature = "cli")]
    #[error("WAV sample read failed")]
    WavRead(#[source] hound::Error),
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

    #[cfg(feature = "cli")]
    #[test]
    fn wav_errors_render_with_their_values() {
        let int = Error::UnsupportedBitDepth {
            bits: 40,
            float: false,
        };
        assert_eq!(int.to_string(), "unsupported integer bit depth: 40-bit");
        let float = Error::UnsupportedBitDepth {
            bits: 64,
            float: true,
        };
        assert_eq!(float.to_string(), "unsupported float bit depth: 64-bit");
        let layout = Error::UnsupportedSampleLayout { bits: 16 };
        assert_eq!(
            layout.to_string(),
            "unsupported sample container layout: 16-bit samples in a padded container"
        );
        let mid = Error::WavEndedMidFrame {
            samples: 1,
            channels: 2,
        };
        assert_eq!(
            mid.to_string(),
            "WAV ended mid-frame: read 1 sample(s) of a 2-channel frame (truncated WAV?)"
        );
    }

    #[cfg(feature = "cli")]
    #[test]
    fn wav_read_error_keeps_the_hound_error_as_its_source() {
        use std::error::Error as _;
        let e = Error::WavRead(hound::Error::FormatError("bad"));
        assert_eq!(e.to_string(), "WAV sample read failed");
        let source = e.source().map(ToString::to_string);
        assert_eq!(source.as_deref(), Some("Ill-formed WAVE file: bad"));
    }
}
