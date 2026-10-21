//! WAV input: stream a WAV file through an [`SstvDecoder`].
//!
//! Compiled with the `cli` feature. This is the reading half of the
//! `slowrx-cli` binary, public so other front ends (and the `decode_wav`
//! example) get the same handling without re-implementing it:
//!
//! - **Sample formats:** 8-, 16-, 24- and 32-bit integer samples are
//!   normalised to `[-1, 1]` (dividing by `2^(bits-1) - 1`, so the largest
//!   positive sample is exactly `1.0`); 32-bit float samples pass through
//!   unscaled. [`check_spec`] rejects anything else up front.
//! - **Channels:** any number, folded to mono by averaging each frame
//!   ([`fold_to_mono`]).
//! - **Streaming:** samples are read and decoded in chunks of 4096 frames,
//!   so memory stays flat however long the recording is, and events reach
//!   the caller as soon as the decoder emits them. The decoder buffers
//!   internally, so the chunking doesn't change what it decodes.
//!
//! ```no_run
//! use slowrx::{wav, SstvDecoder, SstvEvent};
//!
//! let mut reader = hound::WavReader::open("recording.wav")?;
//! let mut decoder = SstvDecoder::new(reader.spec().sample_rate)?;
//! wav::decode_wav_stream(&mut reader, &mut decoder, |event| {
//!     if let SstvEvent::ImageComplete { image, .. } = event {
//!         println!("{:?}: {} x {}", image.mode, image.width, image.height);
//!     }
//!     Ok::<(), Box<dyn std::error::Error>>(())
//! })?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::io::Read;

use crate::error::{Error, Result};
use crate::{SstvDecoder, SstvEvent};

/// Frames per streaming chunk: ~256 ms of audio at 16 kHz, held as 16 KB
/// of mono `f32` whatever the channel count. Tradeoff: smaller chunks
/// mean lower peak memory but more loop iterations and slightly higher
/// decoder overhead per call. 4096 is well past the per-call fixed-cost
/// noise floor and well under any practical memory limit.
const CHUNK_FRAMES: usize = 4096;

/// Check that this module can read `spec`'s samples: integer samples of
/// 8, 16, 24 or 32 bits, or 32-bit float samples.
///
/// [`decode_wav_stream`] runs this before reading anything; call it
/// yourself to reject a file before doing other work with it.
///
/// # Errors
///
/// [`Error::NoAudioChannels`] for a zero-channel spec;
/// [`Error::UnsupportedBitDepth`] for any other bit depth.
pub fn check_spec(spec: hound::WavSpec) -> Result<()> {
    if spec.channels == 0 {
        return Err(Error::NoAudioChannels);
    }
    let float = spec.sample_format == hound::SampleFormat::Float;
    let supported = if float {
        spec.bits_per_sample == 32
    } else {
        matches!(spec.bits_per_sample, 8 | 16 | 24 | 32)
    };
    if supported {
        Ok(())
    } else {
        Err(Error::UnsupportedBitDepth {
            bits: spec.bits_per_sample,
            float,
        })
    }
}

/// Fold one interleaved frame (one sample per channel) to a mono sample
/// by averaging the channels. A one-channel frame passes through
/// unchanged; an empty frame folds to `0.0`.
#[must_use]
pub fn fold_to_mono(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    // Channel counts come from a `u16`, so they're exact in `f32`.
    #[allow(clippy::cast_precision_loss)]
    let channels = frame.len() as f32;
    frame.iter().sum::<f32>() / channels
}

/// Stream every sample of `reader` through `decoder`, handing each event
/// it emits to `on_event`.
///
/// Reads from the reader's current position to the end of its data,
/// normalising and folding to mono on the fly (see the [module
/// docs](self)). The decoder keeps its state, so a caller can stream
/// several files through one decoder, or keep feeding it afterwards.
///
/// `E` is the caller's error type: errors from `on_event` stop the stream
/// and are returned unchanged, and this module's own errors are converted
/// with `From<slowrx::Error>` — so `anyhow::Error`,
/// `Box<dyn std::error::Error>` and [`Error`] itself all work.
///
/// # Errors
///
/// - [`Error::NoAudioChannels`] / [`Error::UnsupportedBitDepth`]: see
///   [`check_spec`].
/// - [`Error::UnsupportedSampleLayout`]: the samples are padded into a
///   wider container than `hound` can read (e.g. 16-bit samples in
///   4-byte slots).
/// - [`Error::WavEndedMidFrame`]: the data stopped part-way through a
///   multi-channel frame.
/// - [`Error::WavRead`]: a sample couldn't be read (I/O error, truncated
///   or malformed file).
/// - Whatever `on_event` returns.
pub fn decode_wav_stream<R: Read, E: From<Error>>(
    reader: &mut hound::WavReader<R>,
    decoder: &mut SstvDecoder,
    mut on_event: impl FnMut(SstvEvent) -> std::result::Result<(), E>,
) -> std::result::Result<(), E> {
    for_each_mono_chunk(reader, |chunk: &[f32]| {
        decoder
            .process(chunk)
            .into_iter()
            .try_for_each(&mut on_event)
    })
}

/// Read `reader` to the end as normalised mono chunks of at most
/// [`CHUNK_FRAMES`] frames, handing each to `sink`.
fn for_each_mono_chunk<R: Read, E: From<Error>>(
    reader: &mut hound::WavReader<R>,
    mut sink: impl FnMut(&[f32]) -> std::result::Result<(), E>,
) -> std::result::Result<(), E> {
    let spec = reader.spec();
    check_spec(spec)?;
    match spec.sample_format {
        hound::SampleFormat::Int => {
            let full_scale = int_full_scale(spec.bits_per_sample);
            // 24- and 32-bit samples lose low bits in `f32`; far below
            // any radio noise floor.
            #[allow(clippy::cast_precision_loss)]
            let to_f32 = |s: i32| s as f32 / full_scale;
            fold_frames(reader.samples::<i32>(), spec, to_f32, &mut sink)
        }
        hound::SampleFormat::Float => fold_frames(reader.samples::<f32>(), spec, |s| s, &mut sink),
    }
}

/// The value a `bits`-bit integer sample is divided by to normalise it:
/// `2^(bits-1) - 1`, the largest positive sample. `bits` is one of
/// 8 / 16 / 24 / 32 ([`check_spec`]); the clamp only keeps the shift in
/// range for anything else.
fn int_full_scale(bits: u16) -> f32 {
    let shift = u32::from(bits.saturating_sub(1)).clamp(7, 31);
    // 2^31 - 1 rounds to 2^31 in `f32`, as the 32-bit full-scale sample
    // does, so that still normalises to exactly 1.0.
    #[allow(clippy::cast_precision_loss)]
    let full_scale = ((1_i64 << shift) - 1) as f32;
    full_scale
}

/// Fold interleaved `samples` (`spec.channels` per frame) to mono and
/// hand the mono audio to `sink` in chunks of at most [`CHUNK_FRAMES`].
/// A clean end of data flushes the partial chunk; a partial final frame
/// is an error.
fn fold_frames<S, E: From<Error>>(
    mut samples: impl Iterator<Item = hound::Result<S>>,
    spec: hound::WavSpec,
    to_f32: impl Fn(S) -> f32,
    sink: &mut impl FnMut(&[f32]) -> std::result::Result<(), E>,
) -> std::result::Result<(), E> {
    let channels = usize::from(spec.channels);
    let mut frame = vec![0.0_f32; channels];
    let mut chunk = Vec::with_capacity(CHUNK_FRAMES);
    loop {
        let filled = read_frame(&mut samples, &mut frame, &to_f32, spec.bits_per_sample)?;
        if filled == 0 {
            // Clean end of data on a frame boundary: flush what's left.
            if !chunk.is_empty() {
                sink(&chunk)?;
            }
            return Ok(());
        }
        if filled < channels {
            return Err(Error::WavEndedMidFrame {
                samples: filled,
                channels: spec.channels,
            }
            .into());
        }
        chunk.push(fold_to_mono(&frame));
        if chunk.len() == CHUNK_FRAMES {
            sink(&chunk)?;
            chunk.clear();
        }
    }
}

/// Fill `frame` from `samples`, returning how many slots were filled
/// (fewer than `frame.len()` only at the end of the data).
fn read_frame<S>(
    samples: &mut impl Iterator<Item = hound::Result<S>>,
    frame: &mut [f32],
    to_f32: &impl Fn(S) -> f32,
    bits: u16,
) -> Result<usize> {
    for (filled, slot) in frame.iter_mut().enumerate() {
        match samples.next() {
            Some(Ok(s)) => *slot = to_f32(s),
            Some(Err(e)) => return Err(read_error(e, bits)),
            None => return Ok(filled),
        }
    }
    Ok(frame.len())
}

/// Map a `hound` sample-read error. `hound` picks its sample reader by
/// (container bytes, bits): a bit depth [`check_spec`] accepts can still
/// sit in a padded container it has no reader for (`Unsupported`, or
/// `TooWide` for containers over 4 bytes) — report that layout rather
/// than hound's generic message.
fn read_error(err: hound::Error, bits: u16) -> Error {
    match err {
        hound::Error::Unsupported | hound::Error::TooWide => {
            Error::UnsupportedSampleLayout { bits }
        }
        other => Error::WavRead(other),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::cast_possible_truncation
)]
mod tests {
    use std::io::Cursor;

    use hound::SampleFormat::{Float, Int};

    use super::*;

    type MemReader = hound::WavReader<Cursor<Vec<u8>>>;

    fn spec(channels: u16, bits: u16, sample_format: hound::SampleFormat) -> hound::WavSpec {
        hound::WavSpec {
            channels,
            sample_rate: crate::WORKING_SAMPLE_RATE_HZ,
            bits_per_sample: bits,
            sample_format,
        }
    }

    /// An in-memory WAV of `samples` (interleaved), written by `hound`.
    fn wav<S: hound::Sample + Copy>(spec: hound::WavSpec, samples: &[S]) -> MemReader {
        let mut bytes = Cursor::new(Vec::new());
        let mut w = hound::WavWriter::new(&mut bytes, spec).unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
        hound::WavReader::new(Cursor::new(bytes.into_inner())).unwrap()
    }

    /// A PCM WAV `hound` can't write: any `bits` / `block_align` pairing.
    fn raw_pcm(channels: u16, bits: u16, block_align: u16, data: &[u8]) -> MemReader {
        let rate = crate::WORKING_SAMPLE_RATE_HZ;
        let data_len = u32::try_from(data.len()).unwrap();
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16_u32.to_le_bytes()); // PCMWAVEFORMAT chunk
        b.extend_from_slice(&1_u16.to_le_bytes()); // WAVE_FORMAT_PCM
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
        b.extend_from_slice(&block_align.to_le_bytes());
        b.extend_from_slice(&bits.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        b.extend_from_slice(data);
        hound::WavReader::new(Cursor::new(b)).unwrap()
    }

    /// Every mono sample `reader` yields, in order.
    fn mono(reader: &mut MemReader) -> Result<Vec<f32>> {
        let mut out = Vec::new();
        for_each_mono_chunk(reader, |chunk: &[f32]| {
            out.extend_from_slice(chunk);
            Ok::<(), Error>(())
        })?;
        Ok(out)
    }

    #[test]
    fn fold_to_mono_averages_each_frame() {
        assert_eq!(fold_to_mono(&[0.75]), 0.75);
        assert_eq!(fold_to_mono(&[0.5, -0.25]), 0.125);
        assert_eq!(fold_to_mono(&[0.25, 0.5, 0.75]), 0.5);
        assert_eq!(fold_to_mono(&[]), 0.0);
    }

    #[test]
    fn stereo_and_three_channel_wavs_fold_to_mono() {
        let mut stereo = wav(spec(2, 16, Int), &[i16::MAX, 0, i16::MAX, -i16::MAX]);
        assert_eq!(mono(&mut stereo).unwrap(), [0.5, 0.0]);
        let three_ch = [0.25_f32, 0.5, 0.75, -0.25, -0.5, -0.75];
        let mut three = wav(spec(3, 32, Float), &three_ch);
        assert_eq!(mono(&mut three).unwrap(), [0.5, -0.5]);
    }

    #[test]
    fn integer_samples_normalise_to_full_scale_at_every_bit_depth() {
        for bits in [8_u16, 16, 24, 32] {
            let max = i32::try_from((1_i64 << (bits - 1)) - 1).unwrap();
            let mut r = wav(spec(1, bits, Int), &[max, 0, -max, max / 2]);
            let got = mono(&mut r).unwrap();
            assert_eq!(got[..3], [1.0, 0.0, -1.0], "{bits}-bit");
            assert!(
                (got[3] - 0.5).abs() < 0.01,
                "{bits}-bit half scale: {}",
                got[3]
            );
        }
    }

    #[test]
    fn float_samples_pass_through_unscaled() {
        let mut r = wav(spec(1, 32, Float), &[0.5_f32, -0.25, 1.5]);
        assert_eq!(mono(&mut r).unwrap(), [0.5, -0.25, 1.5]);
    }

    #[test]
    fn audio_arrives_in_bounded_chunks() {
        let samples = vec![0_i16; CHUNK_FRAMES + 10];
        let mut r = wav(spec(1, 16, Int), &samples);
        let mut lens = Vec::new();
        for_each_mono_chunk(&mut r, |chunk: &[f32]| {
            lens.push(chunk.len());
            Ok::<(), Error>(())
        })
        .unwrap();
        assert_eq!(lens, [CHUNK_FRAMES, 10]);
    }

    #[test]
    fn check_spec_gates_the_bit_depth() {
        for bits in [8, 16, 24, 32] {
            assert!(check_spec(spec(1, bits, Int)).is_ok(), "{bits}-bit int");
        }
        assert!(check_spec(spec(2, 32, Float)).is_ok());
        let int40 = check_spec(spec(1, 40, Int)).unwrap_err();
        assert!(matches!(
            int40,
            Error::UnsupportedBitDepth {
                bits: 40,
                float: false
            }
        ));
        let float64 = check_spec(spec(1, 64, Float)).unwrap_err();
        assert!(matches!(
            float64,
            Error::UnsupportedBitDepth {
                bits: 64,
                float: true
            }
        ));
    }

    /// `hound` refuses to open a zero-channel file, but `check_spec` is
    /// public and takes any spec; an empty frame must not read as a clean
    /// end of stream (review of #123).
    #[test]
    fn check_spec_rejects_a_spec_with_no_channels() {
        assert!(matches!(
            check_spec(spec(0, 16, Int)),
            Err(Error::NoAudioChannels)
        ));
    }

    #[test]
    fn an_unsupported_bit_depth_is_rejected_before_reading() {
        // 40-bit samples: hound opens the file, the gate refuses it.
        let mut r = raw_pcm(1, 40, 5, &[0; 10]);
        let err = mono(&mut r).unwrap_err();
        assert_eq!(err.to_string(), "unsupported integer bit depth: 40-bit");
    }

    #[test]
    fn a_padded_container_reports_the_layout() {
        // 16-bit samples in 4-byte slots pass the gate, then hound can't
        // read them (C14).
        let mut r = raw_pcm(1, 16, 4, &[0; 8]);
        let err = mono(&mut r).unwrap_err();
        assert!(
            matches!(err, Error::UnsupportedSampleLayout { bits: 16 }),
            "{err}"
        );
        assert!(err
            .to_string()
            .starts_with("unsupported sample container layout"));
    }

    #[test]
    fn a_partial_final_frame_is_reported_as_truncated() {
        let samples = [Ok(0.5_f32), Ok(0.5), Ok(0.25)].into_iter();
        let mut sink = |_: &[f32]| Ok::<(), Error>(());
        let err = fold_frames(samples, spec(2, 32, Float), |s| s, &mut sink).unwrap_err();
        assert!(matches!(
            err,
            Error::WavEndedMidFrame {
                samples: 1,
                channels: 2
            }
        ));
    }

    #[test]
    fn a_sample_read_error_is_passed_on_as_its_source() {
        let samples = [Ok(0.5_f32), Err(hound::Error::FormatError("bad"))].into_iter();
        let mut sink = |_: &[f32]| Ok::<(), Error>(());
        let err = fold_frames(samples, spec(1, 32, Float), |s| s, &mut sink).unwrap_err();
        assert!(
            matches!(err, Error::WavRead(hound::Error::FormatError("bad"))),
            "{err}"
        );
    }

    /// A Robot 36 VIS (then silence) as a 16-bit stereo WAV.
    fn vis_wav() -> MemReader {
        let mut audio = crate::vis::tests::synth_vis(0x08, 0.0);
        audio.extend(std::iter::repeat_n(0.0_f32, 2 * CHUNK_FRAMES));
        let stereo: Vec<i16> = audio
            .iter()
            .flat_map(|&s| {
                let v = (s * f32::from(i16::MAX)) as i16;
                [v, v]
            })
            .collect();
        wav(spec(2, 16, Int), &stereo)
    }

    #[test]
    fn decode_wav_stream_forwards_decoder_events() {
        let mut r = vis_wav();
        let mut decoder = SstvDecoder::new(r.spec().sample_rate).unwrap();
        let mut events = Vec::new();
        decode_wav_stream(&mut r, &mut decoder, |e| {
            events.push(e);
            Ok::<(), Error>(())
        })
        .unwrap();
        assert!(
            events.iter().any(|e| matches!(
                e,
                SstvEvent::VisDetected {
                    mode: crate::SstvMode::Robot36,
                    ..
                }
            )),
            "{events:?}"
        );
    }

    #[test]
    fn an_event_handler_error_stops_the_stream_and_is_returned() {
        #[derive(Debug)]
        enum Caller {
            Stop,
            Slowrx,
        }
        impl From<Error> for Caller {
            fn from(_: Error) -> Self {
                Self::Slowrx
            }
        }
        let mut r = vis_wav();
        let mut decoder = SstvDecoder::new(r.spec().sample_rate).unwrap();
        let mut calls = 0;
        let res = decode_wav_stream(&mut r, &mut decoder, |_| {
            calls += 1;
            Err(Caller::Stop)
        });
        assert!(matches!(res, Err(Caller::Stop)), "{res:?}");
        assert_eq!(calls, 1);
        // A WAV error converts into the caller's error type.
        let mut bad = raw_pcm(1, 40, 5, &[0; 10]);
        let res = decode_wav_stream(&mut bad, &mut decoder, |_| Ok(()));
        assert!(matches!(res, Err(Caller::Slowrx)), "{res:?}");
    }
}
