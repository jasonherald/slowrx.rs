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

    let raw = read_samples(&mut reader, spec)?;
    let mono = collapse_to_mono(raw, usize::from(spec.channels))?;

    let mut decoder =
        SstvDecoder::new(spec.sample_rate).with_context(|| "construct SstvDecoder")?;
    if !args.quiet {
        eprintln!("decoding {} mono samples...", mono.len());
    }
    let events = decoder.process(&mono);

    let mut image_count: u32 = 0;
    let mut vis_count: u32 = 0;
    let mut line_count: u32 = 0;

    for event in events {
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
    }

    if !args.quiet {
        eprintln!("done: {vis_count} VIS, {line_count} lines, {image_count} image(s)");
    }
    Ok(image_count)
}

fn read_samples(
    reader: &mut hound::WavReader<std::io::BufReader<std::fs::File>>,
    spec: hound::WavSpec,
) -> Result<Vec<f32>> {
    match spec.sample_format {
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample;
            if !matches!(bits, 8 | 16 | 24 | 32) {
                bail!("unsupported integer bit depth: {bits}-bit");
            }
            // Normalize signed PCM by its positive full-scale magnitude.
            #[allow(clippy::cast_precision_loss)]
            let divisor = ((1_i64 << (bits - 1)) - 1) as f32;
            let mut buf = Vec::with_capacity(reader.len() as usize);
            for sample in reader.samples::<i32>() {
                let s = sample.with_context(|| "WAV sample read failed")?;
                #[allow(clippy::cast_precision_loss)]
                let f = (s as f32) / divisor;
                buf.push(f);
            }
            Ok(buf)
        }
        hound::SampleFormat::Float => {
            let mut buf = Vec::with_capacity(reader.len() as usize);
            for sample in reader.samples::<f32>() {
                buf.push(sample.with_context(|| "WAV sample read failed")?);
            }
            Ok(buf)
        }
    }
}

fn collapse_to_mono(samples: Vec<f32>, channels: usize) -> Result<Vec<f32>> {
    if channels <= 1 {
        return Ok(samples);
    }
    if samples.len() % channels != 0 {
        return Err(anyhow!(
            "sample count {} is not divisible by channel count {} (truncated WAV?)",
            samples.len(),
            channels
        ));
    }
    #[allow(clippy::cast_precision_loss)]
    let inv = 1.0 / (channels as f32);
    Ok(samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() * inv)
        .collect())
}

fn mode_tag(mode: SstvMode) -> &'static str {
    match mode {
        SstvMode::Pd120 => "pd120",
        SstvMode::Pd180 => "pd180",
        // SstvMode is non-exhaustive; future modes get a generic tag
        // until the binary's match adds a specific case.
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
