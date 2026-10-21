//! Minimal usage example: stream a WAV through `SstvDecoder` with
//! [`slowrx::wav::decode_wav_stream`], writing each image as a PNG.
//!
//! For a polished tool with `--input`/`--output` flags, mode options and
//! progress reporting, use the `slowrx-cli` binary instead:
//!
//! ```text
//! cargo install slowrx --features cli
//! slowrx-cli --input recording.wav --output ./out
//! ```
//!
//! This example exists to show the smallest amount of glue needed to
//! drive the library API. `slowrx::wav` handles the WAV side (any channel
//! count, 8/16/24/32-bit integer or 32-bit float samples). Exit status
//! follows `slowrx-cli`: 0 = images written, 2 = none, 1 = error.
//!
//! ```text
//! cargo run --features cli --example decode_wav -- recording.wav
//! ```

use std::env;
use std::error::Error;
use std::process::ExitCode;

use slowrx::{SstvDecoder, SstvEvent, SstvImage};

fn main() -> Result<ExitCode, Box<dyn Error>> {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: decode_wav <input.wav>");
        return Ok(ExitCode::from(1));
    };

    let mut reader = hound::WavReader::open(&path)?;
    let mut decoder = SstvDecoder::new(reader.spec().sample_rate)?;

    let mut written = 0_u32;
    slowrx::wav::decode_wav_stream(&mut reader, &mut decoder, |event| {
        if let SstvEvent::ImageComplete { image, .. } = event {
            written += 1;
            let out = format!("decoded-{written:03}.png");
            save_png(&image, &out)?;
            println!("wrote {out}");
        }
        Ok::<(), Box<dyn Error>>(())
    })?;

    if written == 0 {
        eprintln!("no SSTV images decoded");
        return Ok(ExitCode::from(2));
    }
    Ok(ExitCode::SUCCESS)
}

fn save_png(image: &SstvImage, out: &str) -> Result<(), Box<dyn Error>> {
    let buf: Vec<u8> = image.pixels.iter().flatten().copied().collect();
    let png = image::RgbImage::from_vec(image.width, image.height, buf)
        .ok_or("image buffer size mismatch")?;
    png.save(out)?;
    Ok(())
}
