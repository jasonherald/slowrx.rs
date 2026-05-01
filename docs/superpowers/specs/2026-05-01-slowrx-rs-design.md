# slowrx.rs V1 — Design Specification

**Status:** Approved 2026-05-01. Ready for implementation planning via the
[superpowers:writing-plans](https://github.com/anthropic-ai/superpowers) skill.

**Goal:** Build a pure-Rust SSTV (Slow-Scan TV) decoder library — a faithful
port of [slowrx](https://github.com/windytan/slowrx) by Oona Räisänen (OH2EIQ) —
that ships V1 with PD120 + PD180 mode coverage and publishes 0.1.0 to crates.io.

**Out of scope for this spec:**

- Modes other than PD120 / PD180 — V2 epic tracks Robot 36/72, Scottie 1/2/DX,
  Martin 1/2, PD240 as separate follow-up PRs.
- HF SSTV reception — RTL-SDR hardware tunes 24 MHz–1.766 GHz, missing the HF
  ham bands. SSTV on 145.800 MHz (ISS) is the on-target use case.
- GUI / live viewer / file management — slowrx.rs is a library; rendering and
  I/O are caller responsibilities.
- Integration with `jasonherald/rtl-sdr` — that work happens in a separate
  ticket against the rtl-sdr repo, post-publish.

## Attribution

`slowrx.rs` is a Rust port of [slowrx](https://github.com/windytan/slowrx) by
Oona Räisänen (OH2EIQ). Significant portions of this crate's algorithms — VIS
detection, mode-specification tables, frequency-to-pixel mapping, sync
correlation — are translated directly from slowrx's C source. slowrx is
distributed under the ISC License; this Rust port is distributed under the MIT
License with the ISC notice preserved (see `NOTICE.md` in the repo root).

Per-file headers in `src/` identify which modules are direct translations and
credit the corresponding slowrx file.

## Architecture overview

```text
audio chunks (any rate, mono f32)
        │
        ▼
RationalResampler ──────► working audio (11025 Hz, mono f32)
                                  │
                                  ▼
                          ┌───────────────┐
                          │  SstvDecoder  │
                          └───────┬───────┘
                                  │
                ┌─────────────────┼─────────────────┐
                ▼                 ▼                 ▼
         VisDetector       Decoder loop       SstvImage buffer
        (Goertzel +)      (sync correlate    (pixel writer,
        (parity)           + line clock +    PNG export)
                           mode dispatch)
                                  │
                                  ▼
                          SstvEvent stream
                          ├─ VisDetected(mode)
                          ├─ LineDecoded{mode, line, pixels}
                          └─ ImageComplete(SstvImage)
```

The decoder is a continuous state machine: feed it audio, get events out.
It cycles through `AwaitingVis → Decoding(mode) → ImageComplete` for every
image transmitted, returning to `AwaitingVis` automatically. Multi-image
sequences (like ARISS events) produce one `ImageComplete` event per image
without intervention.

**Pure DSP** — no threading, no I/O. Callers control buffering, file output,
and rendering. The crate is `#![forbid(unsafe_code)]` and panics only on
caller-violated invariants (e.g., constructing a decoder with sample rate ≤ 0).

## File layout

Hard cap: ≤ 500 LOC per file. If any file approaches that during
implementation, split before merging.

```text
src/
  lib.rs          — public API + re-exports                            (≤ 150 LOC)
  error.rs        — thiserror-derived Error + Result type aliases      (≤ 100 LOC)
  decoder.rs      — generic SstvDecoder state machine, sync correlation,
                    line-clock advance (mode-agnostic)                  (≤ 400 LOC)
  vis.rs          — VIS header detection: Goertzel filters at the
                    relevant tone frequencies, parity verification,
                    mode-code lookup                                    (≤ 400 LOC)
  modespec.rs     — mode-spec table covering all V1 + V2 modes
                    (constants only — no runtime logic). V2 modes
                    appear with `unimplemented` decoder hooks until
                    their PR lands.                                     (≤ 300 LOC)
  mode_pd.rs      — PD-family chroma logic (YCbCr line pairing,
                    horizontal subsampling)                             (≤ 400 LOC)
  image.rs        — SstvImage pixel buffer + writer + PNG export       (≤ 200 LOC)
  resample.rs     — Internal rational resampler (caller rate →
                    11025 Hz working rate). Hand-rolled FIR or
                    pulled from rubato — decided in PR-0.               (≤ 400 LOC)

V2 adds (no churn to V1 files):
  mode_robot.rs   — Robot 36 / 72
  mode_scottie.rs — Scottie 1 / 2 / DX
  mode_martin.rs  — Martin 1 / 2

tests/
  roundtrip.rs           — synthetic encode → decode → assert
  slowrx_validate.rs     — cross-validation against slowrx C output
  fixtures/iss-sstv/     — real-ARISS WAV + reference PNG corpus
                            (committed; bootstrapped from publicly
                             shared ARISS recordings)

examples/
  decode_wav.rs          — read a WAV from argv[1], save PNGs to argv[2]
                            (matches slowrx's CLI behavior)
```

## Public API

The surface is small and stable from day 1. PR-0 establishes these signatures
with stub implementations; subsequent PRs fill in the bodies.

```rust
// lib.rs re-exports

pub use crate::decoder::{SstvDecoder, SstvEvent};
pub use crate::error::{Error, Result};
pub use crate::image::SstvImage;
pub use crate::modespec::{ModeSpec, SstvMode};

/// Working sample rate the decoder operates at internally. Any caller
/// sample rate is resampled to this before processing.
pub const WORKING_SAMPLE_RATE_HZ: f64 = 11_025.0;
```

```rust
// modespec.rs

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SstvMode {
    Pd120,
    Pd180,
    // V2 variants:
    // Pd240, Robot36, Robot72, Scottie1, Scottie2, ScottieDx, Martin1, Martin2,
}

pub struct ModeSpec {
    pub mode: SstvMode,
    pub vis_code: u8,           // 7-bit VIS identifier
    pub line_pixels: u32,       // pixels per visible line
    pub image_lines: u32,       // total visible lines per image
    pub line_duration_ms: f64,  // total time per line including sync + porches
    pub channel_layout: ChannelLayout, // PD: YCbCr-paired; future: Robot, Scottie, Martin
    // ... timing details (sync pulse duration, porch durations, scan durations
    //     per colour channel, frequency mapping bounds)
}

pub fn lookup(code: u8) -> Option<ModeSpec> { ... }
```

```rust
// decoder.rs

pub struct SstvDecoder {
    /* internal */
}

impl SstvDecoder {
    /// Construct a decoder. `input_sample_rate_hz` is the caller's audio
    /// rate; the decoder resamples internally to WORKING_SAMPLE_RATE_HZ.
    ///
    /// Returns `Err(Error::InvalidSampleRate)` if rate is 0 or > 192000.
    pub fn new(input_sample_rate_hz: u32) -> Result<Self>;

    /// Process a chunk of mono `f32` audio samples in caller's rate.
    /// Drains internal buffers; returned events were produced during
    /// this call's processing window.
    pub fn process(&mut self, audio: &[f32]) -> Vec<SstvEvent>;

    /// Reset the decoder to AwaitingVis. Any in-progress image is
    /// discarded. Use between known signal boundaries (e.g., a satellite
    /// pass ending) to ensure the next pass starts clean.
    pub fn reset(&mut self);

    /// Total samples processed since the decoder was constructed (or
    /// last reset). Useful for diagnostics + for callers that want to
    /// align audio-WAV recordings to decoder events.
    pub fn samples_processed(&self) -> u64;
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SstvEvent {
    /// VIS header parsed, mode dispatched.
    VisDetected { mode: SstvMode, sample_offset: u64 },

    /// One scan line completed. Caller may render incrementally.
    LineDecoded { mode: SstvMode, line_index: u32, pixels: Vec<[u8; 3]> },

    /// Image complete (LineDecoded for the final line was just emitted).
    /// `partial: true` if the image ended via reset() rather than a
    /// natural line count.
    ImageComplete { image: SstvImage, partial: bool },
}
```

```rust
// image.rs

pub struct SstvImage {
    pub mode: SstvMode,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 3]>, // row-major RGB
}

impl SstvImage {
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 3]>;
    pub fn put_pixel(&mut self, x: u32, y: u32, rgb: [u8; 3]);
}
```

PNG export is intentionally **not** in the library — it would force a heavy
`image` crate dependency on every consumer. Callers handle PNG encoding with
their preferred crate (`image`, `png`, etc.). The `slowrx-cli` binary and
`examples/decode_wav.rs` demonstrate the conversion in ~10 lines using the
`image` crate.

```rust
// error.rs

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid sample rate: {got} (must be > 0 and ≤ 192000)")]
    InvalidSampleRate { got: u32 },

    #[error("VIS code {0:#x} does not map to a known SSTV mode")]
    UnknownVisCode(u8),
}

pub type Result<T> = std::result::Result<T, Error>;
```

The library Error type is intentionally minimal — only failure modes the
**library itself** can produce. PNG encoding, WAV reading, and I/O errors
belong to the CLI / examples / tests, which use their own error wrappers
(or `anyhow` for the CLI binary's main).

## Decoder lifecycle

```text
┌──────────────┐
│ AwaitingVis  │ ◀────────────┐
└──────┬───────┘              │
       │ VIS burst recognized │
       │ (VIS bits decoded,   │
       │  parity OK,          │
       │  mode looked up)     │
       ▼                      │
┌──────────────┐              │
│  Decoding    │              │
│   (mode)     │              │
└──────┬───────┘              │
       │ Final line emitted   │
       │ for this mode        │
       ▼                      │
┌──────────────┐              │
│ImageComplete │              │
│  (one tick)  │ ─────────────┘
└──────────────┘  (return to AwaitingVis automatically)
```

**State details:**

- `AwaitingVis` — Goertzel filters at 1900 / 1200 / 1100 / 1300 Hz are running
  against a sliding audio window. When the leader-break-data-stop pattern is
  recognized and parity validates, transition to `Decoding(mode)`.
- `Decoding(mode)` — line-clock advance per `modespec`. Sync correlation finds
  the start of each line within ±1 sample. Pixels written to the in-flight
  `SstvImage` via per-mode chroma logic. After `image_lines` lines, transition
  to `ImageComplete`.
- `ImageComplete` — emits the `ImageComplete` event with the buffer, one tick.
  Auto-transition back to `AwaitingVis` on the next `process()` call.

**Mid-image VIS detection:** if a VIS burst is detected while in `Decoding`
state, the in-flight image is closed as `partial = true` and the new image
starts immediately. Handles the case where the first image's data corrupted
mid-decode and the next VIS still arrives clean.

**Reset semantics:** `reset()` discards any in-flight image, clears the
Goertzel buffers, returns to `AwaitingVis`. Useful when the caller knows
signal-fade boundaries (satellite pass ending, channel change).

## Cross-validation strategy (slowrx parity)

Two tiers — bit-exact where the algorithm is discrete, tolerance-windowed
where floating-point operation ordering creates legitimate small differences.

### Tier A — bit-exact required

Any mismatch is a bug. No tolerance.

| Quantity | Why bit-exact |
|---|---|
| VIS bit-decode result (7-bit code) | Discrete bit decisions per Goertzel power threshold |
| Mode dispatch (slowrx mode ID → our `SstvMode`) | Deterministic table lookup |
| Image dimensions (width × height) | Constants from `modespec.rs` |
| Line count detected per pass | Discrete counter |

### Tier B — tolerance match

FP operation ordering between Rust f64 and C double creates small differences
in derived values. Tight but non-zero tolerances:

| Quantity | Tolerance |
|---|---|
| Pixel value per channel | max abs diff ≤ 2 of 255, mean abs diff < 0.5 |
| Line-start sample (sync alignment) | ±2 audio samples — sub-sample peak interpolation differs slightly between Rust f64 and C double |

Anything outside the tolerance is a regression — tighter than perceptual
equivalence (~10 of 255), loose enough to absorb FFT-ordering differences.

### Cross-validation harness

`tests/slowrx_validate.rs`:

1. Pre-step: `make` slowrx from `original/slowrx/`. Cached in CI between runs.
2. For each fixture in `tests/fixtures/iss-sstv/`:
   - Run slowrx on the WAV → reference PNG(s).
   - Run our decoder on the same WAV → our PNG(s).
   - Tier A asserts: same image count, same dimensions per image, same mode.
   - Tier B asserts: pixel diff within window. On failure, write a diff
     visualization PNG to `target/slowrx-diff/{fixture}-{N}.png` so failures
     are debuggable.
3. The harness is enabled by default in CI; locally it requires
   `original/slowrx/` to exist (build skipped if missing — emits skip message).

### Coverage gate

`cargo llvm-cov --fail-under-lines 92 --fail-under-regions 92` enforced **per
file** (not just per crate — averaging masks thinly-tested modules). Enabled
in PR-1 once non-trivial code lands.

## Testing strategy (5 layers)

| Layer | Scope | Speed | Purpose |
|---|---|---|---|
| **1. Unit** | per-module (`vis.rs`, `decoder.rs`, etc.) | <100 ms | Pure-function correctness |
| **2. Synthetic round-trip** | `tests/roundtrip.rs` | <500 ms | Encode-then-decode self-consistency, one per mode |
| **3. slowrx cross-validation** | `tests/slowrx_validate.rs` | ~10 s | Tier A + Tier B parity against C reference |
| **4. Real-ARISS regression** | `tests/fixtures/iss-sstv/*.{wav,expected.png}` | ~5 s | Production-audio pinning |
| **5. Property tests** | `proptest` against VIS detector | ~2 s | Robustness against malformed input |

## Dependencies

Minimal — the crate is a pure decoder.

```toml
# Always-on:
thiserror = "2"

# Optional, gated on `cli` feature:
hound       = { version = "3",  optional = true }
image       = { version = "0.25", optional = true }
clap        = { version = "4",  features = ["derive"], optional = true }

# Test-only:
[dev-dependencies]
proptest = "1"
hound    = "3"           # round-trip + slowrx-validate read WAVs
image    = "0.25"        # slowrx-validate compares PNGs
```

**Resampler decision deferred to PR-0:** either a hand-rolled polyphase FIR
(~150 LOC, no dep) or `rubato` (well-maintained MIT-licensed crate, ~50 LOC of
glue). PR-0 prototypes both and picks the simpler one consistent with the
≤400 LOC cap on `resample.rs`.

## PR decomposition

V1 ships in 6 PRs into the new repo, terminating in `cargo publish 0.1.0`.

| PR | Scope | LOC est. | New gates |
|---|---|---|---|
| **PR-0** | API skeleton: `lib.rs`, `error.rs`, `modespec.rs` (V1 + V2 entries with `unimplemented` hooks for V2), `image.rs` scaffold, `decoder.rs` scaffold (state machine shell, no real decoding), `resample.rs` (hand-rolled FIR or rubato glue). Doc tests on every public item. | ~400 | unchanged |
| **PR-1** | `vis.rs` — Goertzel detector, parity check, mode lookup. Synthetic VIS test corpus. | ~500 + tests | coverage gate at 92% |
| **PR-2** | `mode_pd.rs` + populated PD120/180 rows in `modespec.rs` + `decoder.rs` line-clock + sync correlation. End-to-end "synthesize VIS+PD120 image, decode it, assert pixel match" round-trip. | ~700 + tests | unchanged |
| **PR-3** | `tests/slowrx_validate.rs` cross-validation harness. CI step that builds `original/slowrx`. Initial 2-3 ARISS fixtures committed under `tests/fixtures/iss-sstv/`. | ~400 + ~5 MB fixtures | adds slowrx-validate CI job |
| **PR-4** | `examples/decode_wav.rs` and a `[[bin]]` `slowrx-cli` target gated on the `cli` feature. CLI accepts `--input WAV --output DIR`. | ~300 | unchanged |
| **PR-5** | Pre-publish polish: README usage example, `CHANGELOG.md` initial entry, version bump to 0.1.0, complete rustdoc on every public item, `cargo publish --dry-run` clean. | ~200 | adds `cargo publish --dry-run` gate |
| 🚀 | `cargo publish` (manual, after PR-5 merges) | — | crates.io page live |

**Concurrent post-publish tracks:**

- **V2 mode epic** (filed in slowrx.rs as label `epic` + `v2`): one PR per
  mode-family. Robot, Scottie, Martin, PD240. Each adds its own `mode_*.rs`
  file, populates the corresponding `modespec.rs` rows, ships fixtures.
  Estimated ~300-400 LOC + ~2 MB fixtures per PR. Modular by construction;
  V1 files don't churn.
- **rtl-sdr integration epic** (filed in `jasonherald/rtl-sdr` as label
  `epic`): adds `slowrx = "0.1"` dependency, wires the live viewer
  (`sstv_viewer.rs`), recorder extension (`PassOutput::SstvDir`), Aviation
  panel toggle, ARISS auto-record. Reuses Sections 4-5 from the original
  brainstorm. Independent of V2 mode work — rtl-sdr can ship integration
  with V1 modes and pick up V2 modes via `cargo update`.

## V2 roadmap (out of V1 scope, sketched for context)

| Mode | Notes |
|---|---|
| PD240 | Trivial — same chroma as PD120/180, just a different timing row |
| Robot 36 | Different chroma encoding (YUV alternating, not paired). New `mode_robot.rs`. |
| Robot 72 | Same encoding as Robot 36, different timings. Extends `mode_robot.rs`. |
| Scottie 1 / 2 / DX | RGB sequential per line. New `mode_scottie.rs`. |
| Martin 1 / 2 | Similar to Scottie but with different sync placement. New `mode_martin.rs`. |

V2 is community-grade comprehensive coverage. V1 ships against ARISS reception;
V2 extends to general SSTV reception once rtl-sdr-side integration is live.

## Risks + mitigations

| Risk | Mitigation |
|---|---|
| FP-ordering pixel diffs > Tier B tolerance | If common, widen Tier B to ±3/255; if rare-and-fixable, fix in our impl |
| slowrx CI build breaks (its `make` requires libgtk-3-dev, libasound2-dev, libfftw3-dev) | Pin slowrx commit hash; cache the slowrx build; if breakage persists, vendor a single pure-decoder C harness from slowrx's `vis.c` + `video.c` only |
| ARISS fixture availability | rtl-sdr cannot capture SSTV until **after** V1 publishes (rtl-sdr integration depends on slowrx 0.1). For V1, PR-3 ships with 2-3 publicly-shared ARISS WAVs (CC-licensed via the SSTV community) plus the synthetic round-trip corpus. Real-world ARISS captures from rtl-sdr land in a follow-up "expand fixture corpus" PR after rtl-sdr integration ships. |
| crates.io name `slowrx` taken before PR-5 publish | Verified available 2026-05-01; Cargo.toml asserts the name; the publish-dry-run gate in PR-5 catches any name conflict before manual publish |
| Resampler rolling our own = bug surface | PR-0 prototype with rubato as fallback; coverage gate forces tests; cross-validate against slowrx's own resampling output |

## Success criteria

V1 is done when:

1. `cargo publish --dry-run` clean for `slowrx 0.1.0`.
2. `cargo llvm-cov --fail-under-lines 92 --fail-under-regions 92` green per file.
3. `tests/slowrx_validate.rs` green on the committed ARISS fixture corpus,
   Tier A + Tier B both passing.
4. CI matrix (Linux, MSRV 1.85) green.
5. `examples/decode_wav.rs` runs end-to-end on a real ARISS WAV and produces
   PNG output matching the reference within Tier B tolerance.
6. `lib.rs` top-level rustdoc has a `## Example` section with a working
   usage snippet that runs as a doctest. README quotes from this snippet
   to avoid duplication.
7. `cargo doc` clean with no rustdoc warnings.

After V1 ships, the rtl-sdr integration ticket is filed against
`jasonherald/rtl-sdr` to wire `slowrx = "0.1"` into the in-app SSTV pipeline.
