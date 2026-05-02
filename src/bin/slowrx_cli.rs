//! `slowrx-cli` — decode SSTV recordings from WAV files to PNG images.
//!
//! Usage:
//!
//! ```text
//! slowrx-cli --input recording.wav --output ./out
//! ```
//!
//! Reads the WAV at `--input`, decodes every SSTV image found, and writes
//! one PNG per `ImageComplete` event to `<output>/img-NNN-{mode}.png`
//! (sequence-numbered starting at 001, lowercased mode tag — matches the
//! rtl-sdr satellite-recorder naming convention).
//!
//! Requires the `cli` feature: `cargo install --features cli`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use slowrx::{SstvDecoder, SstvEvent, SstvImage, SstvMode};

#[derive(Parser, Debug)]
#[command(
    name = "slowrx-cli",
    version,
    about = "Decode SSTV recordings (WAV) to PNG images"
)]
struct Args {
    /// Path to a mono or multi-channel WAV file containing SSTV audio.
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// Output directory. Created if it does not exist. PNGs are written
    /// here as `img-NNN-{mode}.png`.
    #[arg(short, long, value_name = "DIR")]
    output: PathBuf,

    /// Suppress per-event progress output. Errors and the final summary
    /// still go to stderr.
    #[arg(short, long)]
    quiet: bool,
}

fn main() -> ExitCode {
    match run(&Args::parse()) {
        Ok(image_count) => {
            if image_count == 0 {
                eprintln!("warning: no SSTV images decoded from input");
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &Args) -> Result<u32> {
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("create output dir {}", args.output.display()))?;

    let mut reader = hound::WavReader::open(&args.input)
        .with_context(|| format!("open WAV {}", args.input.display()))?;
    let spec = reader.spec();
    if !args.quiet {
        eprintln!(
            "input: {} Hz, {} channel(s), {} bits, {} samples",
            spec.sample_rate,
            spec.channels,
            spec.bits_per_sample,
            reader.duration()
        );
    }

    let mut decoder =
        SstvDecoder::new(spec.sample_rate).with_context(|| "construct SstvDecoder")?;

    // Streaming pipeline: read in fixed-size chunks, normalize + fold to
    // mono on the fly, push each mono chunk into the stateful decoder.
    // This keeps peak memory at ~CHUNK_FRAMES × (channels + 1) × 4 bytes
    // instead of the whole WAV, and lets the decoder emit events as
    // soon as they're available rather than after a full-file read.
    //
    // The decoder is stateful and buffers internally — chunked input
    // produces identical output to a single all-at-once `process` call.
    let channels = usize::from(spec.channels);
    if channels == 0 {
        bail!("WAV reports zero channels");
    }
    if matches!(spec.sample_format, hound::SampleFormat::Int)
        && !matches!(spec.bits_per_sample, 8 | 16 | 24 | 32)
    {
        bail!(
            "unsupported integer bit depth: {}-bit",
            spec.bits_per_sample
        );
    }

    if !args.quiet {
        eprintln!("decoding {} samples (streaming)...", reader.duration());
    }

    let mut image_count: u32 = 0;
    let mut vis_count: u32 = 0;
    let mut line_count: u32 = 0;

    let mut on_event = |event: SstvEvent| -> Result<()> {
        match event {
            SstvEvent::VisDetected {
                mode,
                sample_offset,
                hedr_shift_hz,
            } => {
                vis_count += 1;
                if !args.quiet {
                    eprintln!(
                        "  VIS: mode={mode:?} at sample {sample_offset} (hedr_shift {hedr_shift_hz:+.1} Hz)"
                    );
                }
            }
            SstvEvent::LineDecoded { .. } => {
                line_count += 1;
            }
            SstvEvent::ImageComplete { image, .. } => {
                image_count += 1;
                let path = args
                    .output
                    .join(format!("img-{image_count:03}-{}.png", mode_tag(image.mode)));
                save_image(&image, &path)?;
                if !args.quiet {
                    eprintln!(
                        "  ImageComplete: {} x {} → {}",
                        image.width,
                        image.height,
                        path.display()
                    );
                }
            }
            _ => {}
        }
        Ok(())
    };

    stream_decode(&mut reader, spec, channels, &mut decoder, &mut on_event)?;

    if !args.quiet {
        eprintln!("done: {vis_count} VIS, {line_count} lines, {image_count} image(s)");
    }
    Ok(image_count)
}

/// Frames per streaming chunk. 4096 frames at 16 kHz mono = ~256 ms of
/// audio; at 16 kHz stereo = 16 KB of f32 samples. Tradeoff: smaller
/// chunks mean lower peak memory but more loop iterations and slightly
/// higher decoder overhead per call. 4096 is well past the per-call
/// fixed-cost noise floor and well under any practical memory limit.
const CHUNK_FRAMES: usize = 4096;

/// Stream samples from `reader`, fold each frame to mono on the fly,
/// push fixed-size chunks into the stateful decoder, and forward every
/// emitted event to `on_event`. Returns when EOF is reached or any
/// step fails.
fn stream_decode(
    reader: &mut hound::WavReader<std::io::BufReader<std::fs::File>>,
    spec: hound::WavSpec,
    channels: usize,
    decoder: &mut SstvDecoder,
    on_event: &mut dyn FnMut(SstvEvent) -> Result<()>,
) -> Result<()> {
    let mut frame_buf: Vec<f32> = vec![0.0; channels];
    let mut mono_chunk: Vec<f32> = Vec::with_capacity(CHUNK_FRAMES);
    #[allow(clippy::cast_precision_loss)]
    let inv_channels = 1.0_f32 / (channels as f32);

    match spec.sample_format {
        hound::SampleFormat::Int => {
            #[allow(clippy::cast_precision_loss)]
            let divisor = ((1_i64 << (spec.bits_per_sample - 1)) - 1) as f32;
            let mut samples = reader.samples::<i32>();
            stream_loop(
                &mut samples,
                channels,
                &mut frame_buf,
                &mut mono_chunk,
                inv_channels,
                |s| {
                    #[allow(clippy::cast_precision_loss)]
                    let f = (s as f32) / divisor;
                    f
                },
                decoder,
                on_event,
            )?;
        }
        hound::SampleFormat::Float => {
            let mut samples = reader.samples::<f32>();
            stream_loop(
                &mut samples,
                channels,
                &mut frame_buf,
                &mut mono_chunk,
                inv_channels,
                |s| s,
                decoder,
                on_event,
            )?;
        }
    }
    Ok(())
}

/// Inner loop, generic over the per-sample type and the conversion to
/// `f32`. Reads frames (`channels` samples each) into `frame_buf`, folds
/// to mono into `mono_chunk`, and flushes to `decoder.process()` when
/// the chunk is full. EOF flushes any partial chunk; a partial frame at
/// EOF is reported as a truncated WAV.
#[allow(clippy::too_many_arguments)]
fn stream_loop<S, I, F>(
    samples: &mut I,
    channels: usize,
    frame_buf: &mut [f32],
    mono_chunk: &mut Vec<f32>,
    inv_channels: f32,
    to_f32: F,
    decoder: &mut SstvDecoder,
    on_event: &mut dyn FnMut(SstvEvent) -> Result<()>,
) -> Result<()>
where
    I: Iterator<Item = Result<S, hound::Error>>,
    F: Fn(S) -> f32,
{
    loop {
        // Try to read one full frame.
        let mut filled = 0_usize;
        for slot in frame_buf.iter_mut().take(channels) {
            match samples.next() {
                Some(Ok(s)) => {
                    *slot = to_f32(s);
                    filled += 1;
                }
                Some(Err(e)) => return Err(e).with_context(|| "WAV sample read failed"),
                None => break,
            }
        }
        if filled == 0 {
            // Clean EOF on a frame boundary — flush any pending chunk.
            if !mono_chunk.is_empty() {
                for event in decoder.process(mono_chunk) {
                    on_event(event)?;
                }
                mono_chunk.clear();
            }
            return Ok(());
        }
        if filled < channels {
            return Err(anyhow!(
                "WAV ended mid-frame: read {filled} sample(s) of a {channels}-channel frame (truncated WAV?)"
            ));
        }

        // Average frame to mono.
        let mono = if channels == 1 {
            frame_buf[0]
        } else {
            frame_buf.iter().sum::<f32>() * inv_channels
        };
        mono_chunk.push(mono);

        if mono_chunk.len() >= CHUNK_FRAMES {
            for event in decoder.process(mono_chunk) {
                on_event(event)?;
            }
            mono_chunk.clear();
        }
    }
}

fn mode_tag(mode: SstvMode) -> &'static str {
    match mode {
        SstvMode::Pd120 => "pd120",
        SstvMode::Pd180 => "pd180",
        SstvMode::Pd240 => "pd240",
        SstvMode::Robot24 => "robot24",
        SstvMode::Robot36 => "robot36",
        SstvMode::Robot72 => "robot72",
        // SstvMode is #[non_exhaustive] which forces a wildcard arm in
        // matches even within the same crate. New variants will use
        // this fallback — when adding one, also add an explicit arm
        // above. (V2.1 PD240 trap: missed this match and shipped images
        // as `img-NNN-unknown.png` until 0.2.1. Same trap caught for
        // V2.2 Robot24/36/72 during code review of the dispatch refactor.)
        _ => "unknown",
    }
}

fn save_image(image: &SstvImage, path: &std::path::Path) -> Result<()> {
    let w = image.width;
    let h = image.height;
    let mut buf: Vec<u8> = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for pixel in &image.pixels {
        buf.extend_from_slice(pixel);
    }
    let img: image::ImageBuffer<image::Rgb<u8>, _> = image::ImageBuffer::from_vec(w, h, buf)
        .ok_or_else(|| anyhow!("image buffer size mismatch"))?;
    img.save(path)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
