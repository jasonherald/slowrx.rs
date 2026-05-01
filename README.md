# slowrx.rs

> **A pure-Rust port of [slowrx](https://github.com/windytan/slowrx)** —
> the SSTV decoder by [Oona Räisänen (OH2EIQ)](https://windytan.github.io/).
> The original is excellent; this port aims to bring it to the Rust
> ecosystem with a library-first API while preserving the algorithmic
> work that made slowrx great.

[![CI](https://github.com/jasonherald/slowrx.rs/actions/workflows/ci.yml/badge.svg)](https://github.com/jasonherald/slowrx.rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

## Status

🚧 **Pre-0.1 — under active development.**

The implementation roadmap is tracked in [GitHub Issues](https://github.com/jasonherald/slowrx.rs/issues).
The first published release on [crates.io](https://crates.io/) will land
once V1 (PD120 + PD180) decodes ARISS reception audio cleanly end-to-end.

## What it does

`slowrx.rs` decodes Slow-Scan Television images from a stream of audio
samples. Mode coverage roadmap:

| Mode family | V1 | V2 |
|---|---|---|
| **PD** | PD120, PD180 | PD240 |
| **Robot** | — | Robot 36, Robot 72 |
| **Scottie** | — | Scottie 1, Scottie 2, Scottie DX |
| **Martin** | — | Martin 1, Martin 2 |

VIS header detection is automatic — feed the decoder audio, get images
out as they complete.

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
