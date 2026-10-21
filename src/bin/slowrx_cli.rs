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
//! rtl-sdr satellite-recorder naming convention). WAV reading is
//! [`slowrx::wav`]: 8/16/24/32-bit integer or 32-bit float samples, any
//! channel count (folded to mono).
//!
//! # Exit status
//!
//! - `0` — at least one image was written.
//! - `2` — the input decoded without error, but no image was written
//!   (no SSTV found, or every image dropped by `--drop-noise`).
//! - `1` — error: bad arguments, an unreadable or unsupported WAV, or an
//!   output directory / PNG that couldn't be written.
//!
//! Requires the `cli` feature: `cargo install --features cli`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use slowrx::{SstvDecoder, SstvEvent, SstvImage, SstvMode};

/// Exit status: the input decoded, but no image was written.
const EXIT_NO_IMAGES: u8 = 2;
/// Exit status: any error, including bad arguments.
const EXIT_ERROR: u8 = 1;

/// The exit-status contract, shown at the end of `--help`.
const EXIT_STATUS_HELP: &str = "\
Exit status:
  0  at least one image was written
  2  the input decoded, but no image was written
  1  error (bad arguments, unreadable or unsupported WAV, unwritable output)";

#[derive(Parser, Debug)]
#[command(
    name = "slowrx-cli",
    version,
    about = "Decode SSTV recordings (WAV) to PNG images",
    after_help = EXIT_STATUS_HELP
)]
struct Args {
    /// Path to a mono or multi-channel WAV file containing SSTV audio.
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// Output directory (must be a directory). Created if it does not
    /// exist. PNGs are written here as `img-NNN-{mode}.png`.
    #[arg(short, long, value_name = "DIR")]
    output: PathBuf,

    /// Suppress progress output and the final summary. Errors and
    /// warnings still go to stderr.
    #[arg(short, long)]
    quiet: bool,

    /// Mode to assume when a VIS header's leaders arrive but its bits are
    /// lost (e.g. `robot36`, the same names as the output files); images
    /// with no header at all then start from this mode's sync rhythm. Off
    /// by default. See `SstvDecoder::set_fallback_mode`.
    #[arg(long, value_name = "MODE", value_parser = parse_mode)]
    fallback_mode: Option<SstvMode>,

    /// Decode every image as MODE, ignoring the mode bits of VIS headers
    /// (headers still mark where images start; images without one start
    /// from the sync rhythm). Overrides `--fallback-mode`. See
    /// `SstvDecoder::set_forced_mode`.
    #[arg(long, value_name = "MODE", value_parser = parse_mode)]
    mode: Option<SstvMode>,

    /// Don't write images whose line-sync quality says they are noise
    /// (`SyncQuality::looks_like_noise`, e.g. a VIS followed by a faded or
    /// ended transmission). They are still reported on stderr.
    #[arg(long)]
    drop_noise: bool,

    /// Demodulate pixels from the analytic signal: removes the fine diagonal
    /// cross-hatch slowrx's real-signal demod leaves in dark areas, with no
    /// loss of sharpness. Off by default (slowrx's behaviour). See
    /// `SstvDecoder::set_analytic_demod`.
    #[arg(long)]
    analytic_demod: bool,
}

/// Resolve a mode by its `ModeSpec::short_name` (case-insensitive).
fn parse_mode(name: &str) -> std::result::Result<SstvMode, String> {
    let specs: Vec<_> = (0u8..0x80).filter_map(slowrx::lookup_vis).collect();
    specs
        .iter()
        .find(|s| s.short_name.eq_ignore_ascii_case(name))
        .map(|s| s.mode)
        .ok_or_else(|| {
            let names: Vec<_> = specs.iter().map(|s| s.short_name).collect();
            format!(
                "unknown mode '{name}'; expected one of: {}",
                names.join(", ")
            )
        })
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            // `--help` / `--version` land here too, and succeed.
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::from(EXIT_ERROR)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    match run(&args) {
        Ok(0) => {
            eprintln!("warning: no SSTV images decoded from input");
            ExitCode::from(EXIT_NO_IMAGES)
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_ERROR)
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
    slowrx::wav::check_spec(spec)?;

    let mut decoder =
        SstvDecoder::new(spec.sample_rate).with_context(|| "construct SstvDecoder")?;
    decoder.set_fallback_mode(args.fallback_mode);
    decoder.set_forced_mode(args.mode);
    decoder.set_analytic_demod(args.analytic_demod);

    if !args.quiet {
        eprintln!("decoding {} samples (streaming)...", reader.duration());
    }

    // Streams in fixed-size chunks (flat memory, events as they come).
    let mut tally = Tally::default();
    slowrx::wav::decode_wav_stream(&mut reader, &mut decoder, |event| {
        on_event(args, &mut tally, event)
    })?;

    if !args.quiet {
        eprintln!("{}", tally.summary());
    }
    Ok(tally.images)
}

/// Running counts for the end-of-run summary.
#[derive(Default)]
struct Tally {
    images: u32,
    vis: u32,
    fallback: u32,
    sync_starts: u32,
    dropped: u32,
    lines: u32,
}

impl Tally {
    /// The end-of-run summary line.
    fn summary(&self) -> String {
        format!(
            "done: {} VIS ({} assumed), {} sync start(s), {} lines, {} image(s), {} dropped as noise",
            self.vis, self.fallback, self.sync_starts, self.lines, self.images, self.dropped
        )
    }
}

/// Handle one decoder event: count it, report it, and write completed
/// images (unless `--drop-noise` says the image is noise).
fn on_event(args: &Args, tally: &mut Tally, event: SstvEvent) -> Result<()> {
    let note = match event {
        SstvEvent::VisDetected {
            mode,
            sample_offset,
            hedr_shift_hz,
        } => {
            tally.vis += 1;
            format!("  VIS: mode={mode:?} at sample {sample_offset} (hedr_shift {hedr_shift_hz:+.1} Hz)")
        }
        SstvEvent::VisFallback {
            mode,
            sample_offset,
            hedr_shift_hz,
        } => {
            tally.vis += 1;
            tally.fallback += 1;
            format!("  VIS lost, assuming fallback: mode={mode:?} image from sample {sample_offset} (hedr_shift {hedr_shift_hz:+.1} Hz)")
        }
        SstvEvent::SyncStart {
            mode,
            sample_offset,
        } => {
            tally.sync_starts += 1;
            format!("  no VIS, sync rhythm: mode={mode:?} image from sample {sample_offset}")
        }
        SstvEvent::LineDecoded { .. } => {
            tally.lines += 1;
            return Ok(());
        }
        SstvEvent::ImageComplete {
            image,
            sync_quality,
            ..
        } => return on_image(args, tally, &image, sync_quality),
        _ => return Ok(()),
    };
    if !args.quiet {
        eprintln!("{note}");
    }
    Ok(())
}

/// Write (or, with `--drop-noise`, skip) one completed image.
fn on_image(
    args: &Args,
    tally: &mut Tally,
    image: &SstvImage,
    sync_quality: slowrx::SyncQuality,
) -> Result<()> {
    let noise = sync_quality.looks_like_noise();
    if args.drop_noise && noise {
        tally.dropped += 1;
        if !args.quiet {
            eprintln!(
                "  ImageComplete: {} x {} dropped (sync quality {:.2} looks like noise)",
                image.width,
                image.height,
                sync_quality.value()
            );
        }
        return Ok(());
    }
    tally.images += 1;
    let path = args.output.join(format!(
        "img-{:03}-{}.png",
        tally.images,
        slowrx::modespec::for_mode(image.mode).short_name
    ));
    save_image(image, &path)?;
    if !args.quiet {
        eprintln!(
            "  ImageComplete: {} x {} (sync quality {:.2}{}) → {}",
            image.width,
            image.height,
            sync_quality.value(),
            if noise { ", looks like noise" } else { "" },
            path.display()
        );
    }
    Ok(())
}

/// Write `image` as a PNG, repeating each row `line_height` times (2 for
/// Robot 12/8 B/W, whose 120 lines slowrx shows twice each) so the picture
/// keeps its aspect, as slowrx's saved images do.
fn save_image(image: &SstvImage, path: &std::path::Path) -> Result<()> {
    let w = image.width;
    let line_height = slowrx::for_mode(image.mode).line_height.max(1);
    let h = image.height * line_height;
    let mut buf: Vec<u8> = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for row in image.pixels.chunks(w.max(1) as usize) {
        for _ in 0..line_height {
            buf.extend(row.iter().flatten());
        }
    }
    let img: image::ImageBuffer<image::Rgb<u8>, _> = image::ImageBuffer::from_vec(w, h, buf)
        .ok_or_else(|| anyhow!("image buffer size mismatch"))?;
    img.save(path)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {}
