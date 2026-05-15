# Issue #92 — API hygiene sweep — Design

**Issue:** [#92](https://github.com/jasonherald/slowrx.rs/issues/92) (audit bundle 8 of 12 — IDs C3, C4, C7, C8, C9, C12, D7, D8, D9).

**Scope:** grab-bag of small public-surface polish. Eight `#[must_use]` additions; three new derives + one hand-written `Debug`; re-export `MAX_INPUT_SAMPLE_RATE_HZ` and make `Error::InvalidSampleRate`'s message dynamic; compile-time `Error: Send + Sync + 'static` static-assert; `HannBank` to `[Box<[f32]>; 7]` via `array::from_fn` with a documented panic precondition on `HannBank::get`; drop the `.max(FFT_LEN)` over-allocation at four scratch sites (~26 KiB saved per decoder); two micro-opts (`fft_buf.fill`, `with_capacity` in the resampler hot path). **D7 (shared FFT plan) deferred as YAGNI** — the audit calls it low priority; rustfft plan construction is microseconds-fast one-shot work and sharing buys plumbing complexity without runtime benefit.

---

## Background — audit findings

- **C3 — `#[must_use]` on value-returning fns.** Caller dropping the result is a latent bug for `Resampler::process` (lost audio), `Resampler::new` / `SnrEstimator::new` / `VisDetector::new` / `SyncTracker::new` (silent constructor failure), `ChannelDemod::pixel_freq` / `SnrEstimator::estimate` (lost demod / SNR estimate), and `find_sync` (lost slant correction).
- **C4 — derives.** `SstvEvent` / `SstvImage` / `SyncResult` already have `Debug` but not `PartialEq`; adding it makes them test-friendly and pattern-match-comparable. `Resampler` is pure scalar (no FFT plan inside the polyphase bank), so `Debug` derives cleanly. `SstvDecoder` has fields with `rustfft`-bearing types that block a `Debug` derive, so the impl is hand-written: rate / state / sample counters only, with `.finish_non_exhaustive()` signalling the omitted fields.
- **C9 — `MAX_INPUT_SAMPLE_RATE_HZ` asymmetry.** `lib.rs:74` re-exports `WORKING_SAMPLE_RATE_HZ` but not `MAX_INPUT_SAMPLE_RATE_HZ`. External callers who want to validate caller-side before passing to `Resampler::new` have to reach into `slowrx::resample::MAX_INPUT_SAMPLE_RATE_HZ`. Re-export at the crate root. Also: `error.rs:11`'s `#[error(...)]` hardcodes `192000` — drift waiting to happen if the cap ever changes. Use `{max = crate::resample::MAX_INPUT_SAMPLE_RATE_HZ}` in the `thiserror` format string.
- **C12 — `Error: Send + Sync + 'static`.** `anyhow::Error` and `Box<dyn std::error::Error + Send + Sync + 'static>` consumers depend on this; today `Error` is a single `#[non_exhaustive]` enum with only a `u32` payload (trivially `Send + Sync + 'static`), but a future variant carrying a non-`Send` type (e.g. an `Rc<...>`) would silently break those consumers. A `const _: fn() = || { fn check<T: Send + Sync + 'static>(){} check::<Error>(); };` static-assert turns this into a compile error at variant-add time. Plus one doc-line at the top of the `Error` enum noting it implements `std::error::Error` (currently implicit via `thiserror`).
- **C7 — `HannBank` shape.** Currently `[Vec<f32>; 7]` built by 7 hand-listed `build_hann` calls. `Vec` carries a capacity field that's always equal to length (the bank is immutable post-construction); `Box<[f32]>` drops that overhead. Build via `std::array::from_fn(|idx| build_hann(HANN_LENS[idx]).into_boxed_slice())`. Document the `HannBank::get` panic precondition (caller out-of-range = programmer error; `window_idx_for_snr` returns `0..=6` by construction).
- **C8 — scratch over-allocation.** Four FFT-using struct constructors do `vec![Complex::ZERO; scratch_len.max(FFT_LEN)]`. For `rustfft`'s power-of-two paths (which both 1024 and 256 take), `get_inplace_scratch_len()` returns 0, so `.max(FFT_LEN)` allocates `FFT_LEN × 8 bytes ≈ 8 KiB` of dead scratch per site. Drop the `.max()`. Net win: ~26 KiB per `SstvDecoder` (3 × 8 KiB at FFT_LEN=1024 for `SnrEstimator` / `ChannelDemod` / `VisDetector`, 1 × 2 KiB at SYNC_FFT_LEN=256 for `SyncTracker`).
- **D7 — shared FFT plan. Deferred.** Audit framing: "Low priority." `rustfft::FftPlanner::new().plan_fft_forward(1024)` is microseconds-fast on a one-shot basis at decoder construction; sharing requires either plumbing an `Arc<dyn Fft<f32>>` into both `SnrEstimator::new` and `ChannelDemod::new` (constructor-signature change, plumbing through `SstvDecoder::new`) or restructuring ownership to put the planner in `SstvDecoder`. Either path adds complexity without measurable runtime win (the `Arc<dyn Fft>` dispatch is identical whether the underlying plan is shared or not). The CHANGELOG note documents the deferral.
- **D8 — manual zero loop → `fft_buf.fill`.** `src/demod.rs:269` area uses a for-loop to zero `fft_buf` before re-populating with windowed audio. `fft_buf.fill(Complex { re: 0.0, im: 0.0 })` is one-line idiomatic Rust; release-mode codegen is identical. Pure readability cleanup.
- **D9 — `with_capacity` in hot-path push loops.** `Resampler::process` builds its output `Vec<f32>` with `Vec::new()` then pushes per output sample; final length is `(buf.len() / self.stride).ceil()` ± 1 (phase carry-over). Pre-sizing saves at most one reallocation per `process` call but the resampler runs on every audio chunk — worth doing. The test encoders' push loops (`pd_test_encoder`, `robot_test_encoder`, `scottie_test_encoder`) also have computable final lengths but run once per test — skip them; the audit's "where the final length is computable" framing is about hot paths.

---

## Architecture

### A — Public API surface

**C3 `#[must_use]` additions** at eight call sites:

| File | Item |
|------|------|
| `src/resample.rs` | `Resampler::new` (add if absent; idempotent if already present) |
| `src/resample.rs` | `Resampler::process` |
| `src/snr.rs` | `SnrEstimator::new` |
| `src/snr.rs` | `SnrEstimator::estimate` |
| `src/demod.rs` | `ChannelDemod::pixel_freq` |
| `src/vis.rs` | `VisDetector::new` |
| `src/sync.rs` | `SyncTracker::new` |
| `src/sync.rs` | `find_sync` |

Verification step in the plan: `grep -n "#\[must_use\]" src/*.rs` before/after to confirm net new annotations. If any of these already have `#[must_use]`, that arm is a no-op (idempotent).

**C4 derives:**

- `SstvEvent` (`src/decoder.rs:17` — currently `#[derive(Clone, Debug)]`): add `PartialEq`. The enum's variants carry `String` (`PartialEq`-able), `u8`, `u64`, `f64` (`PartialEq` but not `Eq`), `Box<SstvImage>` (becomes `PartialEq`-able transitively). `Eq` is NOT added (f64 blocks).
- `SstvImage` (`src/image.rs:13` — currently `#[derive(Clone, Debug)]`): add `PartialEq` + `Eq`. Field types (`SstvMode`, `u32`, `Vec<[u8; 3]>`) all support `Eq`; clippy's `derive_partial_eq_without_eq` lint expects `Eq` whenever the fields permit it. Adding `Eq` is a free polish and avoids an allow attribute.
- `SyncResult` (`src/sync.rs:194` — currently `#[derive(Clone, Copy, Debug)]`): add `PartialEq`. Fields: `f64`, `i64`, `Option<f64>`. `Eq` blocked by `f64`.
- `Resampler` (`src/resample.rs:50`): add `Debug` derive. Currently has no Debug. Fields: `input_rate: u32`, `stride: f64`, `phase: f64`, `tail: Vec<f32>`, `taps: Box<[[f32; FIR_TAPS]; NUM_PHASES]>`. All Debug-able; the `taps` print will be 256 rows × 64 f32s — verbose, but Debug output is a debugging aid, not a user-facing API. Acceptable.
- `SstvDecoder` (`src/decoder.rs` — currently has no Debug): hand-written impl. The struct has fields with `rustfft::Fft<f32>` plans inside (`resampler`'s FIR taps are fine; `vis`, `channel_demod`, `snr_est` all hold `Arc<dyn rustfft::Fft<f32>>` which does NOT implement `Debug`). The hand-written impl prints the four scalar fields and `finish_non_exhaustive()`:

```rust
impl std::fmt::Debug for SstvDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SstvDecoder")
            .field("rate", &self.resampler.input_rate())
            .field("state", &self.state)
            .field("samples_processed", &self.samples_processed)
            .field("working_samples_emitted", &self.working_samples_emitted)
            .finish_non_exhaustive()
    }
}
```

**C9 changes:**

1. `src/lib.rs:74`:
   ```rust
   // Before:
   pub use crate::resample::{Resampler, WORKING_SAMPLE_RATE_HZ};
   // After:
   pub use crate::resample::{MAX_INPUT_SAMPLE_RATE_HZ, Resampler, WORKING_SAMPLE_RATE_HZ};
   ```
   (Alphabetized to match Rust convention.)

2. `src/error.rs:10-12`:
   ```rust
   // Before:
   #[error("invalid sample rate: {got} (must be > 0 and ≤ 192000)")]
   InvalidSampleRate {
       got: u32,
   },
   // After:
   #[error(
       "invalid sample rate: {got} (must be > 0 and ≤ {max})",
       max = crate::resample::MAX_INPUT_SAMPLE_RATE_HZ
   )]
   InvalidSampleRate {
       got: u32,
   },
   ```

3. Existing test `invalid_sample_rate_renders_with_value` (`src/error.rs:tests`) — update the expected string. Currently:
   ```rust
   assert_eq!(
       e.to_string(),
       "invalid sample rate: 0 (must be > 0 and ≤ 192000)"
   );
   ```
   becomes:
   ```rust
   assert_eq!(
       e.to_string(),
       format!(
           "invalid sample rate: 0 (must be > 0 and ≤ {})",
           crate::resample::MAX_INPUT_SAMPLE_RATE_HZ
       )
   );
   ```
   The test asserts the formatting end-to-end; future cap changes flow through automatically.

**C12 — static-assert + doc note** at the bottom of `src/error.rs`:

```rust
/// Compile-time assertion that `Error: Send + Sync + 'static`. Required by
/// `anyhow::Error` and `Box<dyn std::error::Error + Send + Sync + 'static>`
/// consumers; a future `Error` variant carrying a non-`Send` type would silently
/// break them, so we make the requirement load-bearing here. (Audit #92 C12.)
const _: fn() = || {
    fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    assert_send_sync_static::<Error>();
};
```

Plus one doc line prepended to the existing `Error` enum doc:

```rust
/// Crate-wide error type. Implements [`std::error::Error`] via `thiserror`.
```

### B — Internal data shape: HannBank (C7)

`src/demod.rs:26-50` rewrite:

```rust
/// Bank of seven Hann windows, indexed by SNR-derived window selector.
/// Construct once per decoder; the inner `Box<[f32]>`s have lengths matching
/// [`HANN_LENS`].
pub(crate) struct HannBank {
    windows: [Box<[f32]>; 7],
}

impl HannBank {
    pub fn new() -> Self {
        Self {
            windows: std::array::from_fn(|idx| {
                crate::dsp::build_hann(HANN_LENS[idx]).into_boxed_slice()
            }),
        }
    }

    /// Borrow window `idx`. Length is `HANN_LENS[idx]`.
    ///
    /// # Panics
    ///
    /// Panics if `idx >= 7`. The [`window_idx_for_snr`] selector returns
    /// `0..=6` by construction (it's a 7-branch `if`-`else` chain over SNR
    /// thresholds), so this should never fire from inside the decoder.
    /// Out-of-range calls would be a programmer error.
    #[must_use]
    pub fn get(&self, idx: usize) -> &[f32] {
        &self.windows[idx]
    }
}

impl Default for HannBank {
    fn default() -> Self {
        Self::new()
    }
}
```

### C — Internal micro-opts (C8 + D8 + D9)

**C8 — scratch over-allocation.** Four sites:

| File | Line | Before | After |
|------|------|--------|-------|
| `src/snr.rs` | 64 | `scratch_len.max(FFT_LEN)` | `scratch_len` |
| `src/sync.rs` | 128 | `scratch_len.max(SYNC_FFT_LEN)` | `scratch_len` |
| `src/demod.rs` | 269 | `scratch_len.max(FFT_LEN)` | `scratch_len` |
| `src/vis.rs` | 98 | `scratch_len.max(FFT_LEN)` | `scratch_len` |

Each site gains a one-line comment: `// rustfft scratch_len=0 for power-of-two sizes; .max() was dead-allocating (audit #92 C8).`

**D8 — `fft_buf.fill`.** One site in `src/demod.rs` (the for-loop-then-overwrite pattern from the audit's `mode_pd.rs:137-149` reference, now under `src/demod.rs` post the #85 module extraction). Locate via `grep -n "fft_buf\[i\]" src/demod.rs`; replace the manual zero-then-write loop with `self.fft_buf.fill(Complex { re: 0.0, im: 0.0 });` immediately before the windowing loop. Plan task lists the exact line range after a confirming grep.

**D9 — `with_capacity` in `Resampler::process`.** One hot-path site:

```rust
// Before (resample.rs:155 area):
let mut out = Vec::new();
// After:
// Output length is approximately `buf.len() / self.stride` ± 1 (phase
// carry-over). Pre-sizing saves a reallocation per process() call on
// every audio chunk. (Audit #92 D9.)
#[allow(clippy::cast_precision_loss, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
let expected_out = (buf.len() as f64 / self.stride).ceil() as usize;
let mut out = Vec::with_capacity(expected_out);
```

`Resampler::process` already carries `#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap, clippy::needless_range_loop)]` (added during #87). Those allows cover the three casts the new `expected_out` line needs, so **no new `#[allow]` is needed** — the existing function-level block already silences them.

### D — D7 deferred

No code change. CHANGELOG bullet documents the deferral with the audit's "low priority" framing.

---

## File touch list

| File | Status | Role |
|------|--------|------|
| `src/lib.rs` | modify | Add `MAX_INPUT_SAMPLE_RATE_HZ` to the `pub use crate::resample::{...}` re-export (C9). |
| `src/error.rs` | modify | Dynamic `#[error]` format string + `Error: Send + Sync + 'static` static-assert + doc note + update existing test (C9, C12). |
| `src/resample.rs` | modify | `#[must_use]` on `new`/`process`; `Debug` derive on `Resampler`; `with_capacity` in `process` (C3, C4, D9). |
| `src/snr.rs` | modify | `#[must_use]` on `new`/`estimate`; drop `.max(FFT_LEN)` in scratch (C3, C8). |
| `src/demod.rs` | modify | `#[must_use]` on `ChannelDemod::pixel_freq`; `HannBank` to `[Box<[f32]>; 7]`; drop `.max(FFT_LEN)` in scratch; `fft_buf.fill` (C3, C7, C8, D8). |
| `src/vis.rs` | modify | `#[must_use]` on `VisDetector::new`; drop `.max(FFT_LEN)` in scratch (C3, C8). |
| `src/sync.rs` | modify | `#[must_use]` on `SyncTracker::new`/`find_sync`; `PartialEq` derive on `SyncResult`; drop `.max(SYNC_FFT_LEN)` in scratch (C3, C4, C8). |
| `src/decoder.rs` | modify | `PartialEq` derive on `SstvEvent`; hand-written `Debug` for `SstvDecoder` (C4). |
| `src/image.rs` | modify | `PartialEq` derive on `SstvImage` (C4). |
| `CHANGELOG.md` | modify | One bullet under `[Unreleased] ### Internal`. |

---

## Out of scope

- **D7** — shared FFT plan between `SnrEstimator` and `ChannelDemod`. Deferred per the audit's "low priority" framing.
- (Adding `Eq` to `SstvImage` was originally out-of-scope but moved in-scope during T1 to satisfy clippy's `derive_partial_eq_without_eq` — the fields all support `Eq`. See the C4 section above.)
- Renaming `lookup_vis` (re-exported as in `src/lib.rs:72`) — that's a different audit item, not in #92.
- Anything in `tests/` files — this is a public-surface sweep, not a test-coverage expansion.

---

## Success criteria

- All 8 in-scope audit findings addressed (C3, C4, C7, C8, C9, C12, D8, D9). D7 deferred with explicit CHANGELOG note.
- Full CI gate green: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features --locked --release`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`.
- Lib test count: **136 → 136** (no new tests; the `invalid_sample_rate_renders_with_value` update is in-place).
- Existing per-mode tests + `tests/roundtrip.rs` 11/11 unchanged.
- The compile-time `Error: Send + Sync + 'static` assertion is in place — verifiable by introducing a non-`Send` variant locally and confirming it fails to compile.
- `MAX_INPUT_SAMPLE_RATE_HZ` reachable as `slowrx::MAX_INPUT_SAMPLE_RATE_HZ` (verifiable via doc-test or a one-liner check).
- `Error::InvalidSampleRate { got: 0 }.to_string()` continues to produce `"invalid sample rate: 0 (must be > 0 and ≤ 192000)"` — verified by the updated test (which now derives the cap dynamically rather than hardcoding it).
