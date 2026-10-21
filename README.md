# slowrx.rs

> **A pure-Rust port of [slowrx](https://github.com/windytan/slowrx)** —
> the SSTV decoder by [Oona Räisänen (OH2EIQ)](https://windytan.github.io/).
> The original is excellent; this port aims to bring it to the Rust
> ecosystem with a library-first API while preserving the algorithmic
> work that made slowrx great.

[![Crates.io](https://img.shields.io/crates/v/slowrx.svg)](https://crates.io/crates/slowrx)
[![Docs.rs](https://docs.rs/slowrx/badge.svg)](https://docs.rs/slowrx)
[![CI](https://github.com/jasonherald/slowrx.rs/actions/workflows/ci.yml/badge.svg)](https://github.com/jasonherald/slowrx.rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

## Status

🛰️ **0.7.0** starts images from a mode's line-sync rhythm when no header is heard, adds a forced-mode option and a per-image sync-quality score that flags images decoded from noise; **0.6.0** added the opt-in lost-VIS fallback (see below). All on top of **V2.4**: PD120, PD180, PD240, Robot 24, Robot 36, Robot 72, Scottie 1, Scottie 2, Scottie DX, Martin 1, and Martin 2 decoding from raw audio. All four V2 mode families landed; the [V2 roadmap](https://github.com/jasonherald/slowrx.rs/issues/9) is closed. Post-V2.4 cleanup (code-review audit backlog, [epic #97](https://github.com/jasonherald/slowrx.rs/issues/97)) is in progress.

PD120, PD180, and Robot 36 are validated end-to-end against real-radio
captures: PD120/PD180 against the ARISS Dec-2017 corpus (6 of 7
fixtures decode to images visually matching the reference JPGs),
Robot 36 against the ARISS Fram2 corpus (all 12 fixtures decode to
images visually matching the reference JPGs — see
[`tests/ariss_fram2_validation.md`](https://github.com/jasonherald/slowrx.rs/blob/main/tests/ariss_fram2_validation.md)). PD240, Robot 24, Robot 72,
Scottie 1/2/DX, and Martin 1/2 ship with synthetic round-trip coverage
only — real-radio fixtures for those modes are pending. Robot 24
inherits Robot 36's real-radio evidence by structural identity (same
decoder code path, only LineTime differs). Scottie + Martin share an
RGB-sequential decoder dispatched on `SyncPosition` (Scottie sync
sits mid-line; Martin sync at line start, the standard SSTV
convention).

## Install

```bash
# library
cargo add slowrx

# CLI tool (decodes WAV → PNG)
cargo install slowrx --features cli
```

## Quick start

```rust
use slowrx::{SstvDecoder, SstvEvent};

// Construct a decoder at the caller's audio sample rate.
let mut decoder = SstvDecoder::new(44_100).expect("valid sample rate");

// Feed audio chunks; consume events as images complete.
let audio: Vec<f32> = vec![0.0; 1024]; // mono samples in [-1.0, 1.0]
for event in decoder.process(&audio) {
    if let SstvEvent::ImageComplete { image, .. } = event {
        println!("decoded {}×{} {:?} image",
                 image.width, image.height, image.mode);
    }
}
```

CLI:

```bash
slowrx-cli --input recording.wav --output ./out
# → out/img-001-pd120.png, out/img-002-robot36.png, ...
```

Exit status: `0` = at least one image written, `2` = no images found,
`1` = error. With the `cli` feature, the library's `slowrx::wav` module
streams a WAV file through a decoder the same way the CLI does.

## What it does

`slowrx.rs` decodes Slow-Scan Television images from a stream of audio
samples. Mode coverage:

| Mode family | Shipped |
|---|---|
| **PD** | PD120, PD180, PD240 |
| **Robot** | Robot 24, Robot 36, Robot 72 |
| **Scottie** | Scottie 1, Scottie 2, Scottie DX |
| **Martin** | Martin 1, Martin 2 |

VIS header detection is automatic — feed the decoder audio, get images
out as they complete.

If a fade wipes out a header's VIS bits, the image after it is normally lost.
Callers that know which mode a station uses can opt in to a fallback, so the
decoder starts the image from the header's leader timing anyway:

```rust
# use slowrx::{SstvDecoder, SstvMode};
# let mut decoder = SstvDecoder::new(44_100).expect("valid sample rate");
decoder.set_fallback_mode(Some(SstvMode::Robot36)); // emits SstvEvent::VisFallback
```

The CLI equivalent is `slowrx-cli --fallback-mode robot36 …`.

With a fallback set, the decoder also catches images that have no header at
all: a fully faded header, or a transmission the receiver joined mid-image.
Once that mode's line-sync pulses line up for about two seconds, it starts the
image at the first synced line and emits `SstvEvent::SyncStart`. A header,
when heard, always wins.

If you already know the mode — a recording, or a station that only sends one —
you can force it. Headers still mark where images start, but their mode bits
are ignored:

```rust
# use slowrx::{SstvDecoder, SstvMode};
# let mut decoder = SstvDecoder::new(44_100).expect("valid sample rate");
decoder.set_forced_mode(Some(SstvMode::Pd120)); // every image decodes as PD120
```

The CLI equivalent is `slowrx-cli --mode pd120 …`.

Each `SstvEvent::ImageComplete` also carries a `sync_quality` score. On
weak signals, a VIS can be followed by noise, and the decoder still returns
a full-size "image" of it; `sync_quality.looks_like_noise()` flags those
so you can skip them (`slowrx-cli --drop-noise`).

## Why a Rust port?

Several mature C/C++ SSTV decoders exist (slowrx, QSSTV, MMSSTV) but no
production-quality pure-Rust library is available on crates.io. This
crate aims to fill that gap with:

- **Library-first design** — no GUI dependency. The crate processes
  audio buffers and emits image-data events; callers render them however
  they want (Cairo, web canvas, terminal ASCII, file output).
- **Modern Rust idioms** — no `unsafe`, structured error types via
  `thiserror`, builder-style configuration.
- **Comprehensive tests** — round-trip synthetic encoding, regression
  fixtures from real ARISS receptions, cross-validation against slowrx
  on shared test corpora.

## Acknowledgements

This crate would not exist without [slowrx](https://github.com/windytan/slowrx)
by **Oona Räisänen (OH2EIQ)**. slowrx's clear, well-documented C source
served as both the reference implementation and the sanity check during
this port. Algorithm choices, mode-specification tables, frequency-to-pixel
mappings, and overall architecture are all directly inspired by — and in
many places translated from — slowrx.

If you use this crate, please consider also acknowledging the original
project. The SSTV community owes a real debt to Oona's work.

## License

Released under the [MIT License](./LICENSE).

slowrx is distributed under the ISC License. The MIT/ISC pairing is
intentional — see [NOTICE.md](./NOTICE.md) for the full attribution and
license preservation notice.
