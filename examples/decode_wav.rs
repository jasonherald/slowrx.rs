//! Minimal usage example: read a 16-bit mono WAV, push it through
//! `SstvDecoder`, write PNGs.
//!
//! For a polished tool with `--input`/`--output` flags, multi-channel
//! audio, all integer bit depths, and proper error reporting, use the
//! `slowrx-cli` binary instead:
//!
//! ```text
//! cargo install slowrx --features cli
//! slowrx-cli --input recording.wav --output ./out
//! ```
//!
//! This example exists to show the smallest amount of glue needed to
//! drive the library API. It assumes a 16-bit mono WAV.
//!
//! ```text
//! cargo run --features cli --example decode_wav -- recording.wav
//! ```

use std::env;
use std::process::ExitCode;

use slowrx::{SstvDecoder, SstvEvent};

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: decode_wav <input.wav>");
        return ExitCode::from(2);
    };

    let mut reader = match hound::WavReader::open(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: open {path}: {e}");
            return ExitCode::from(1);
        }
    };
    let spec = reader.spec();
    if spec.channels != 1 || spec.bits_per_sample != 16 {
        eprintln!(
            "error: this example expects 16-bit mono; got {} ch, {} bits — use slowrx-cli",
            spec.channels, spec.bits_per_sample
        );
        return ExitCode::from(1);
    }

    let max = f32::from(i16::MAX);
    let samples: Vec<f32> = match reader
        .samples::<i16>()
        .map(|r| r.map(|s| f32::from(s) / max))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: WAV sample read: {e}");
            return ExitCode::from(1);
        }
    };

    let mut decoder = match SstvDecoder::new(spec.sample_rate) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: SstvDecoder::new({}): {e}", spec.sample_rate);
            return ExitCode::from(1);
        }
    };

    let mut idx = 0_u32;
    for event in decoder.process(&samples) {
        if let SstvEvent::ImageComplete { image, .. } = event {
            idx += 1;
            let out = format!("decoded-{idx:03}.png");
            let mut buf = Vec::with_capacity((image.width as usize) * (image.height as usize) * 3);
            for px in &image.pixels {
                buf.extend_from_slice(px);
            }
            let Some(img) =
                image::ImageBuffer::<image::Rgb<u8>, _>::from_vec(image.width, image.height, buf)
            else {
                eprintln!("error: image buffer size mismatch");
                return ExitCode::from(1);
            };
            if let Err(e) = img.save(&out) {
                eprintln!("error: write {out}: {e}");
                return ExitCode::from(1);
            }
            println!("wrote {out}");
        }
    }

    if idx == 0 {
        eprintln!("no SSTV images decoded");
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
