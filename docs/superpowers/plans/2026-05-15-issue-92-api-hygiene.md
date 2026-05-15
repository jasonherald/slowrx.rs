# Issue #92 — API hygiene sweep — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land 8 small public-surface polishes from audit bundle 8 of 12 — `#[must_use]` annotations, `PartialEq` + `Debug` derives, re-export `MAX_INPUT_SAMPLE_RATE_HZ` with a dynamic error message, compile-time `Error: Send + Sync + 'static` assertion, `HannBank` to `[Box<[f32]>; 7]`, drop the dead `.max(FFT_LEN)` scratch over-allocation (~26 KiB saved per decoder), and two micro-opts (`fft_buf.fill`, resampler `with_capacity`).

**Architecture:** Six sequential tasks. T1 lands the must_use + derive annotations across 7 files. T2 covers both error.rs changes (dynamic format string + static-assert) and the lib.rs re-export. T3 reshapes HannBank. T4 drops the scratch over-allocation at 4 sites. T5 lands the two micro-opts. T6 closes with the CHANGELOG bullet + final gate. No new tests; one existing test (`invalid_sample_rate_renders_with_value`) updates in place.

**Tech Stack:** Rust 2021, MSRV 1.85. CI gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features --locked --release`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`. No GPG signing.

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-15-issue-92-api-hygiene-design.md`
- Audit: `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs C3, C4, C7, C8, C9, C12, D8, D9; D7 deferred)

---

## File Structure

| File | Status | Task |
|------|--------|------|
| `src/lib.rs` | modify | T2 (re-export `MAX_INPUT_SAMPLE_RATE_HZ`) |
| `src/error.rs` | modify | T2 (dynamic `#[error]` + static-assert + doc + test update) |
| `src/resample.rs` | modify | T1 (`#[must_use]`, `Debug` derive) + T5 (`with_capacity`) |
| `src/snr.rs` | modify | T1 (`#[must_use]`) + T4 (drop `.max(FFT_LEN)`) |
| `src/demod.rs` | modify | T1 (`#[must_use]`) + T3 (`HannBank`) + T4 (drop `.max(FFT_LEN)`) + T5 (`fft_buf.fill`) |
| `src/vis.rs` | modify | T1 (`#[must_use]`) + T4 (drop `.max(FFT_LEN)`) |
| `src/sync.rs` | modify | T1 (`#[must_use]`, `PartialEq` on `SyncResult`) + T4 (drop `.max(SYNC_FFT_LEN)`) |
| `src/decoder.rs` | modify | T1 (`PartialEq` on `SstvEvent`, hand-written `Debug` for `SstvDecoder`) |
| `src/image.rs` | modify | T1 (`PartialEq` on `SstvImage`) |
| `CHANGELOG.md` | modify | T6 (one bullet under `[Unreleased] ### Internal`) |

Task order: **T1** (must_use + derives) → **T2** (C9 + C12) → **T3** (HannBank) → **T4** (scratch over-alloc) → **T5** (D8 + D9) → **T6** (CHANGELOG + final gate).

**Verification after each task** (the rule for this PR):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

Lib test counts at each checkpoint:
- **Pre-#92 baseline:** 136 lib tests.
- **Post-T1:** 136 (additive annotations).
- **Post-T2:** 136 (one existing test updated in place).
- **Post-T3:** 136 (pure refactor).
- **Post-T4:** 136 (pure capacity tweak; FFT calls unchanged).
- **Post-T5:** 136 (micro-opts).
- **Post-T6:** 136.

---

## Task 1: C3 + C4 — `#[must_use]` annotations + `PartialEq` derives + `Debug` on `Resampler` + hand-written `Debug` for `SstvDecoder`

Additive annotations across 7 files. No behavior changes.

**Files:**
- Modify: `src/resample.rs`, `src/snr.rs`, `src/demod.rs`, `src/vis.rs`, `src/sync.rs`, `src/decoder.rs`, `src/image.rs`

- [ ] **Step 1: Add `#[must_use]` on `Resampler::new` and `Resampler::process` in `src/resample.rs`**

Locate `pub fn new(input_rate: u32) -> Result<Self>` (around line 118). If the line immediately above is not already `#[must_use]`, add it:

```rust
    #[must_use = "Resampler::new returns a Result; dropping it silently bypasses rate validation"]
    pub fn new(input_rate: u32) -> Result<Self> {
```

(Note: `#[must_use]` on `Result<T, E>` is already implicit via the `Result` type's own `#[must_use]`; the explicit annotation makes the intent visible at the call site. If the existing source already has `#[must_use]` on `new`, this step is a no-op.)

Locate `pub fn process(&mut self, input: &[f32]) -> Vec<f32>` (around line 157). Add `#[must_use]` immediately before its `pub fn` line if absent:

```rust
    #[must_use = "the resampled audio Vec must be consumed; dropping it discards the decoder input"]
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
```

- [ ] **Step 2: Add `Debug` derive on `Resampler` struct in `src/resample.rs`**

Locate `pub struct Resampler {` (around line 50). Find the `#[derive(...)]` attribute above it (if any). If the struct currently has no derive, add one. If it has `#[derive(...)]` already, add `Debug` to the existing list:

Current state likely shows the struct with no derive at all. Add:

```rust
#[derive(Debug)]
pub struct Resampler {
```

immediately before the existing `pub struct Resampler {` line. All `Resampler` fields (`input_rate: u32`, `stride: f64`, `phase: f64`, `tail: Vec<f32>`, `taps: Box<[[f32; FIR_TAPS]; NUM_PHASES]>`) implement `Debug`, so the derive compiles cleanly.

- [ ] **Step 3: Add `#[must_use]` on `SnrEstimator::new` and `SnrEstimator::estimate` in `src/snr.rs`**

Locate `pub fn new() -> Self` (around line 56). Add `#[must_use]` immediately before its `pub fn` line:

```rust
    #[must_use]
    pub fn new() -> Self {
```

Locate `pub fn estimate(&mut self, audio: &[f32], center_sample: i64, hedr_shift_hz: f64) -> f64` (around line 79). Add `#[must_use]` immediately before its `pub fn` line:

```rust
    #[must_use = "the SNR estimate must be consumed; dropping it makes the estimator a no-op"]
    pub fn estimate(&mut self, audio: &[f32], center_sample: i64, hedr_shift_hz: f64) -> f64 {
```

- [ ] **Step 4: Add `#[must_use]` on `ChannelDemod::pixel_freq` in `src/demod.rs`**

Locate `pub fn pixel_freq(` (around line 289). Add `#[must_use]` immediately before its `pub fn` line:

```rust
    #[must_use = "the demodulated frequency must be consumed; dropping it discards the per-pixel demod result"]
    pub fn pixel_freq(
```

(Do NOT add `#[must_use]` to `ChannelDemod::new` — the audit's C3 list is explicit about which constructors get annotated, and `ChannelDemod::new` isn't on it. Stay scope-faithful.)

- [ ] **Step 5: Add `#[must_use]` on `VisDetector::new` in `src/vis.rs`**

Locate `pub fn new(is_known_vis: fn(u8) -> bool) -> Self` (around line 90). Add `#[must_use]` immediately before its `pub fn` line:

```rust
    #[must_use]
    pub fn new(is_known_vis: fn(u8) -> bool) -> Self {
```

- [ ] **Step 6: Add `#[must_use]` on `SyncTracker::new` and `find_sync` in `src/sync.rs`**

Locate `pub fn new(hedr_shift_hz: f64) -> Self` inside `impl SyncTracker` (around line 113). Add `#[must_use]` immediately before its `pub fn` line:

```rust
    #[must_use]
    pub fn new(hedr_shift_hz: f64) -> Self {
```

Locate `pub(crate) fn find_sync(has_sync: &[bool], initial_rate_hz: f64, spec: ModeSpec) -> SyncResult` (around line 225). Add `#[must_use]` immediately before the existing `#[allow(...)]` block (or directly before `pub(crate) fn find_sync` if no allow block immediately precedes it):

```rust
#[must_use = "the SyncResult must be consumed; dropping it discards the slant + skip correction"]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
pub(crate) fn find_sync(has_sync: &[bool], initial_rate_hz: f64, spec: ModeSpec) -> SyncResult {
```

- [ ] **Step 7: Add `PartialEq` derive on `SyncResult` in `src/sync.rs`**

Locate `pub(crate) struct SyncResult` (around line 195). Its current derive is `#[derive(Clone, Copy, Debug)]`. Update it to add `PartialEq`:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SyncResult {
```

(Note: `Eq` is NOT added — `SyncResult` has an `f64` field, which doesn't implement `Eq`.)

- [ ] **Step 8: Add `PartialEq` derive on `SstvEvent` in `src/decoder.rs`**

Locate `pub enum SstvEvent` (around line 19). Its current derive is `#[derive(Clone, Debug)]`. Update it to add `PartialEq`:

```rust
#[derive(Clone, Debug, PartialEq)]
pub enum SstvEvent {
```

`SstvEvent` variants carry `String`, `u8`, `u64`, `f64`, and `Box<SstvImage>` — all `PartialEq`-able (after T1 Step 10 adds `PartialEq` to `SstvImage`). `Eq` is blocked by the `f64` payload, so don't add it.

- [ ] **Step 9: Add a hand-written `Debug` impl for `SstvDecoder` in `src/decoder.rs`**

Locate the closing `}` of the `impl SstvDecoder` block (after `pub fn process` and any other methods — search for the last `}` of the impl). Immediately after that closing `}`, add the new `impl Debug` block:

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

This skips `vis`, `channel_demod`, `snr_est` (all contain `Arc<dyn rustfft::Fft<f32>>` which doesn't implement `Debug`). `.finish_non_exhaustive()` is the canonical signal for "more fields exist but aren't shown."

If `State` (the variant type for `self.state`) doesn't currently implement `Debug`, add a `#[derive(Debug)]` to its definition (likely near the top of `src/decoder.rs`). Check via grep: `grep -nE "enum State|struct State" src/decoder.rs`. If `Debug` is missing, add it to the existing derive list or create one.

- [ ] **Step 10: Add `PartialEq` derive on `SstvImage` in `src/image.rs`**

Locate `pub struct SstvImage` (around line 15). Its current derive is `#[derive(Clone, Debug)]`. Update it to add `PartialEq`:

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct SstvImage {
```

`SstvImage` fields are `u32` and `Vec<[u8; 3]>` — both `PartialEq` (and `Eq`-able, but we stick to `PartialEq` only to match `SstvEvent`'s shape per the spec).

- [ ] **Step 11: Run the full gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136** (additive annotations; no test changes).

If `derive(Debug)` on `Resampler` fires `clippy::missing_fields_in_debug` or similar, that's the derive working as intended — no action needed. If clippy fires `clippy::derive_partial_eq_without_eq` on `SstvImage`, the existing field types support `Eq`; either add `Eq` to the derive (acceptable extension) OR add `#[allow(clippy::derive_partial_eq_without_eq)]` on the struct. Prefer adding `Eq` since the fields support it; this is a free polish (the spec hedges on it; adding `Eq` is strictly more permissive than not).

If the hand-written `Debug` for `SstvDecoder` fires `clippy::missing_debug_implementations` for the dropped sub-types, that's fine — the impl deliberately omits them and `.finish_non_exhaustive()` signals the omission. No action needed.

- [ ] **Step 12: Commit**

```bash
git add src/resample.rs src/snr.rs src/demod.rs src/vis.rs src/sync.rs src/decoder.rs src/image.rs
git commit -m "feat(api): C3 + C4 — #[must_use] + PartialEq/Debug derives (#92)

C3: #[must_use] on Resampler::{new,process}, SnrEstimator::{new,estimate},
ChannelDemod::{new,pixel_freq}, VisDetector::new, SyncTracker::new,
find_sync.

C4: PartialEq derive on SstvEvent, SstvImage, SyncResult; Debug derive
on Resampler; hand-written Debug impl for SstvDecoder (rate/state/
samples_processed/working_samples_emitted, finish_non_exhaustive to
omit FFT-plan-bearing sub-fields).

Pure additive — no behavior change, no new tests.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: C9 + C12 — `MAX_INPUT_SAMPLE_RATE_HZ` re-export + dynamic error message + `Error: Send + Sync + 'static` static-assert

Both changes live in `src/lib.rs` + `src/error.rs`. One existing test updates in place.

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/error.rs`

- [ ] **Step 1: Re-export `MAX_INPUT_SAMPLE_RATE_HZ` at crate root**

In `src/lib.rs`, locate the `pub use crate::resample::{...}` line (around line 74). Currently:

```rust
pub use crate::resample::{Resampler, WORKING_SAMPLE_RATE_HZ};
```

Change to (alphabetized):

```rust
pub use crate::resample::{MAX_INPUT_SAMPLE_RATE_HZ, Resampler, WORKING_SAMPLE_RATE_HZ};
```

- [ ] **Step 2: Make `Error::InvalidSampleRate`'s message dynamic**

In `src/error.rs`, locate the `InvalidSampleRate` variant (around lines 10-14). Currently:

```rust
    /// Caller-supplied sample rate is outside the supported range.
    #[error("invalid sample rate: {got} (must be > 0 and ≤ 192000)")]
    InvalidSampleRate {
        /// The rate the caller passed.
        got: u32,
    },
```

Change to:

```rust
    /// Caller-supplied sample rate is outside the supported range.
    #[error(
        "invalid sample rate: {got} (must be > 0 and ≤ {max})",
        max = crate::resample::MAX_INPUT_SAMPLE_RATE_HZ
    )]
    InvalidSampleRate {
        /// The rate the caller passed.
        got: u32,
    },
```

(thiserror supports the `name = expr` syntax in `#[error("...", name = expr)]` for binding extra format-arg names. The expression is evaluated at `Display::fmt` time.)

- [ ] **Step 3: Update the `Error` enum top doc to mention `std::error::Error`**

In `src/error.rs`, locate the `/// Crate-wide error type.` doc comment immediately above `pub enum Error` (around line 6). Replace it with:

```rust
/// Crate-wide error type. Implements [`std::error::Error`] via `thiserror`.
```

(The doc-link to `std::error::Error` resolves under rustdoc since `std::error::Error` is in the standard library prelude path.)

- [ ] **Step 4: Add the `Error: Send + Sync + 'static` static-assert**

In `src/error.rs`, append (just before the `#[cfg(test)] mod tests` block):

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

- [ ] **Step 5: Update the existing `invalid_sample_rate_renders_with_value` test**

In `src/error.rs`, locate the test function:

```rust
    #[test]
    fn invalid_sample_rate_renders_with_value() {
        let e = Error::InvalidSampleRate { got: 0 };
        assert_eq!(
            e.to_string(),
            "invalid sample rate: 0 (must be > 0 and ≤ 192000)"
        );
    }
```

Replace it with:

```rust
    #[test]
    fn invalid_sample_rate_renders_with_value() {
        let e = Error::InvalidSampleRate { got: 0 };
        assert_eq!(
            e.to_string(),
            format!(
                "invalid sample rate: 0 (must be > 0 and ≤ {})",
                crate::resample::MAX_INPUT_SAMPLE_RATE_HZ
            )
        );
    }
```

This asserts the formatting end-to-end while sourcing the cap dynamically — future cap changes flow through without touching the test.

- [ ] **Step 6: Run the full gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136** (one test updated in place).

**Critical:** the `invalid_sample_rate_renders_with_value` test must pass after the update. The dynamic message should produce the same string today (since the cap is still 192000), so the assertion is satisfied.

If `RUSTDOCFLAGS="-D warnings"` fires `rustdoc::broken_intra_doc_links` on the new `[`std::error::Error`]` link, change it to `[\`std::error::Error\`]` (just code-span without the link brackets). The std prelude path should resolve under rustdoc, but if not, the code-span fallback is harmless.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/error.rs
git commit -m "feat(error): C9 + C12 — re-export MAX_INPUT_SAMPLE_RATE_HZ + Send/Sync assert (#92)

C9: lib.rs re-exports MAX_INPUT_SAMPLE_RATE_HZ alongside the existing
WORKING_SAMPLE_RATE_HZ (no more asymmetry). Error::InvalidSampleRate
message is now derived from the const at Display time instead of a
hardcoded 192000 literal — future cap changes flow through automatically.

C12: const _: fn() static-assert that Error: Send + Sync + 'static.
Required by anyhow / Box<dyn Error + Send + Sync + 'static> consumers;
a future variant carrying a non-Send type would silently break them.
Plus one doc-line noting Error implements std::error::Error via thiserror.

Existing invalid_sample_rate_renders_with_value test updated in place
to source the cap dynamically.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: C7 — `HannBank` to `[Box<[f32]>; 7]` via `array::from_fn` + documented panic precondition

Pure data-shape refactor inside `src/demod.rs`. No behavior change.

**Files:**
- Modify: `src/demod.rs`

- [ ] **Step 1: Rewrite the `HannBank` struct and `impl HannBank`**

In `src/demod.rs`, locate `pub(crate) struct HannBank` (around line 26). Replace from the struct through the closing `}` of `impl Default for HannBank` (around line 56) with:

```rust
/// Bank of seven Hann windows, indexed by SNR-derived window selector.
/// Construct once per decoder; the inner `Box<[f32]>`s have lengths
/// matching [`HANN_LENS`]. Built once per `HannBank::new()` via
/// `std::array::from_fn` over the 7 entries; the bank is immutable
/// post-construction. (Audit #92 C7.)
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

Notes:
- `Box<[f32]>` is fixed-size after construction; `Vec<f32>` carries a redundant `capacity` field.
- `.into_boxed_slice()` is the idiomatic conversion (shrinks to fit, no reallocation if `len == capacity`).
- `std::array::from_fn` requires Rust 1.63+; the project's MSRV is 1.85 so this is well within scope.
- `HannBank::new` does NOT get `#[must_use]` — the audit's C3 list doesn't include it, and `HannBank` is `pub(crate)` (no external surface).

- [ ] **Step 2: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136**.

**Critical:** any existing call site that calls `HannBank::get(idx)` returns a `&[f32]` — the same return type before and after (since `Vec<f32>` and `Box<[f32]>` both deref to `&[f32]`). Call sites are unchanged. `tests/roundtrip.rs` must stay 11/11 — it exercises the full Hann window dispatch.

If clippy fires `clippy::needless_pass_by_value` or similar on the `std::array::from_fn` closure, the closure form above is idiomatic — no action expected.

- [ ] **Step 3: Commit**

```bash
git add src/demod.rs
git commit -m "refactor(demod): C7 — HannBank to [Box<[f32]>; 7] via array::from_fn (#92)

Replaces the hand-listed 7-element [Vec<f32>; 7] with a Box<[f32]>; 7]
built by std::array::from_fn over HANN_LENS. Box<[T]> drops the
redundant Vec capacity field for an immutable-post-construction table.
HannBank::get gains a documented panic precondition (idx >= 7 is a
programmer error; window_idx_for_snr returns 0..=6 by construction).

Call sites unchanged — both Vec<f32> and Box<[f32]> deref to &[f32].
Pure refactor; tests/roundtrip.rs unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: C8 — drop the `.max(FFT_LEN)` scratch over-allocation at 4 sites

Pure capacity tweak across 4 files. ~26 KiB saved per `SstvDecoder` construction.

**Files:**
- Modify: `src/snr.rs`, `src/sync.rs`, `src/demod.rs`, `src/vis.rs`

- [ ] **Step 1: Drop `.max(FFT_LEN)` in `src/snr.rs` scratch allocation**

Locate (around line 64):

```rust
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len.max(FFT_LEN)],
```

Replace with:

```rust
            // rustfft returns scratch_len = 0 for power-of-two sizes (radix-2/4
            // paths use in-place buffers only). The prior .max(FFT_LEN) was
            // dead-allocating ~8 KiB. (Audit #92 C8.)
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len],
```

- [ ] **Step 2: Drop `.max(SYNC_FFT_LEN)` in `src/sync.rs` scratch allocation**

Locate (around line 128):

```rust
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len.max(SYNC_FFT_LEN)],
```

Replace with:

```rust
            // rustfft returns scratch_len = 0 for power-of-two sizes
            // (SYNC_FFT_LEN=256 is radix-2). The prior .max(SYNC_FFT_LEN) was
            // dead-allocating ~2 KiB. (Audit #92 C8.)
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len],
```

- [ ] **Step 3: Drop `.max(FFT_LEN)` in `src/demod.rs` scratch allocation**

Locate (around line 269):

```rust
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len.max(FFT_LEN)],
```

Replace with:

```rust
            // rustfft returns scratch_len = 0 for power-of-two sizes
            // (FFT_LEN=1024 is radix-2). The prior .max(FFT_LEN) was
            // dead-allocating ~8 KiB. (Audit #92 C8.)
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len],
```

- [ ] **Step 4: Drop `.max(FFT_LEN)` in `src/vis.rs` scratch allocation**

Locate (around line 98):

```rust
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len.max(FFT_LEN)],
```

Replace with:

```rust
            // rustfft returns scratch_len = 0 for power-of-two sizes
            // (FFT_LEN=1024 is radix-2). The prior .max(FFT_LEN) was
            // dead-allocating ~8 KiB. (Audit #92 C8.)
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len],
```

- [ ] **Step 5: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136**.

**Critical:** the FFT calls (`process_with_scratch(&mut buf, &mut scratch)`) must continue to work. rustfft's `process_with_scratch` reads `scratch_len` from the plan; for power-of-two sizes that's 0, so the scratch slice is unused. If any FFT call regresses (panics with "scratch buffer too small"), revert that specific site and report — likely cause is that the size isn't actually a power-of-two on this version of rustfft, but 1024 and 256 are both `2^n` so this should not fire.

`tests/roundtrip.rs` 11/11 is the load-bearing check — it exercises all 4 FFT-using estimators end-to-end.

- [ ] **Step 6: Commit**

```bash
git add src/snr.rs src/sync.rs src/demod.rs src/vis.rs
git commit -m "perf(fft): C8 — drop dead .max(FFT_LEN) scratch over-alloc (#92)

rustfft's get_inplace_scratch_len() returns 0 for power-of-two sizes
(radix-2/4 paths use in-place buffers only). The .max(FFT_LEN) at four
sites was dead-allocating ~8 KiB per site at FFT_LEN=1024 (snr, demod,
vis) and ~2 KiB at SYNC_FFT_LEN=256 (sync) — ~26 KiB total per
SstvDecoder construction. FFT calls unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: D8 + D9 — `fft_buf.fill` + `Resampler::process` `with_capacity`

Two small micro-opts. No behavior change.

**Files:**
- Modify: `src/demod.rs`
- Modify: `src/resample.rs`

- [ ] **Step 1: Locate the D8 fft_buf manual zero loop in `src/demod.rs`**

Run:

```bash
grep -n "fft_buf\[" /data/source/slowrx.rs/src/demod.rs
```

Expected: the line range is in the `pixel_freq` method body (around lines 300-320 post-T3). The pattern is a `for i in 0..FFT_LEN { ... self.fft_buf[i].re = ...; self.fft_buf[i].im = 0.0; ... }` loop OR a paired zero-then-overwrite. Identify the loop that zeros `self.fft_buf` before the windowed audio fills it.

If the loop currently has the form:

```rust
        for i in 0..FFT_LEN {
            self.fft_buf[i].re = 0.0;
            self.fft_buf[i].im = 0.0;
        }
        // ... then a second loop that writes the windowed audio ...
```

— replace the entire first (zero-fill) loop with:

```rust
        self.fft_buf.fill(Complex { re: 0.0, im: 0.0 });
```

If the loop instead has the pattern of a single loop that conditionally writes (which already implicitly zeros), there is no separate zero-fill loop and D8 is a no-op. In that case, document the search result in the commit message: "no separate zero-fill loop existed; D8 marked closed."

- [ ] **Step 2: Apply D9 — `with_capacity` in `Resampler::process`**

In `src/resample.rs`, locate the body of `pub fn process(&mut self, input: &[f32]) -> Vec<f32>` (around line 157). Find the line:

```rust
        let mut out = Vec::new();
```

Replace with:

```rust
        // Output length is approximately buf.len() / stride ± 1 (phase
        // carry-over). Pre-sizing saves a reallocation per process()
        // call on every audio chunk. (Audit #92 D9.)
        let expected_out = (buf.len() as f64 / self.stride).ceil() as usize;
        let mut out = Vec::with_capacity(expected_out);
```

The function already carries `#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap, clippy::needless_range_loop)]` from #87 — those allows cover the three casts the new arithmetic needs. No new annotation required.

- [ ] **Step 3: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136**.

If `clippy::cast_possible_truncation` or `clippy::cast_precision_loss` fires *despite* the function-level allow (e.g., the existing allow uses fewer lints), add the missing ones to the existing block. The minimal addition is `clippy::cast_precision_loss` if it's not in there.

- [ ] **Step 4: Commit**

```bash
git add src/demod.rs src/resample.rs
git commit -m "perf: D8 + D9 — fft_buf.fill + Resampler::process with_capacity (#92)

D8: replace the manual fft_buf zero loop in ChannelDemod::pixel_freq
with .fill(Complex::ZERO) — idiomatic Rust, identical release codegen.

D9: pre-size the Resampler::process output Vec with the computed
expected length (buf.len() / stride, ceil). The resampler runs on
every audio chunk; one reallocation saved per call.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: CHANGELOG + final gate

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add the `CHANGELOG.md` `[Unreleased]` entry**

Open `CHANGELOG.md`. Under `## [Unreleased]` `### Internal`, prepend (so the newest change is first) a new bullet:

```markdown
### Internal

- **API hygiene sweep** — eight small public-surface polishes (audit
  bundle 8 of 12). `#[must_use]` on `Resampler::{new, process}`,
  `SnrEstimator::{new, estimate}`, `ChannelDemod::pixel_freq`,
  `VisDetector::new`, `SyncTracker::new`, `find_sync` (audit C3).
  `PartialEq` derive on `SstvEvent`, `SstvImage`, `SyncResult`; `Debug`
  derive on `Resampler`; hand-written `Debug` impl for `SstvDecoder`
  that prints rate / state / sample counters with
  `.finish_non_exhaustive()` (audit C4). `MAX_INPUT_SAMPLE_RATE_HZ`
  re-exported at the crate root alongside `WORKING_SAMPLE_RATE_HZ`;
  `Error::InvalidSampleRate`'s message is now derived from the const
  at `Display` time (audit C9). Compile-time `Error: Send + Sync +
  'static` static-assert in `error.rs` — protects `anyhow` /
  `Box<dyn Error + Send + Sync>` consumers from a future non-`Send`
  variant (audit C12). `HannBank` reshaped from `[Vec<f32>; 7]` to
  `[Box<[f32]>; 7]` via `std::array::from_fn`, with a documented panic
  precondition on `HannBank::get` (audit C7). Dropped the dead
  `.max(FFT_LEN)` scratch over-allocation at four FFT-using sites
  (`SnrEstimator`, `ChannelDemod`, `VisDetector`, `SyncTracker`) —
  ~26 KiB saved per `SstvDecoder` construction (audit C8). Two
  micro-opts: `fft_buf.fill(Complex::ZERO)` replaces a manual zero
  loop in `ChannelDemod::pixel_freq` (audit D8); `Resampler::process`
  pre-sizes its output `Vec` (audit D9). D7 (shared FFT plan) deferred
  per the audit's "low priority" framing — `rustfft` plan construction
  is microseconds-fast one-shot work; sharing buys plumbing complexity
  without measurable runtime benefit. (#92; audit C3/C4/C7/C8/C9/C12/D8/D9.)

- **`ModeSpec` as single source of truth** — ... [existing #91 bullet stays as-is below]
```

(The existing CHANGELOG continues with the #91, #88, #87, #86, #85 bullets — leave those untouched.)

- [ ] **Step 2: Run the full CI gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected test counts:
- lib: **136** (unchanged from baseline).
- `tests/roundtrip.rs`: 11/11 (unchanged).
- All other integration tests (cli, multi_image, no_vis, unknown_vis): unchanged.
- Doc clean.

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(refactor): CHANGELOG for the API hygiene sweep (#92)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-review notes (for the implementer / reviewers)

- **Spec coverage:**
  - C3 (`#[must_use]`) → T1 Steps 1, 3-6.
  - C4 (`PartialEq` + `Debug` + hand-written `Debug` for SstvDecoder) → T1 Steps 2, 7-10.
  - C7 (`HannBank` shape) → T3.
  - C8 (scratch over-alloc) → T4.
  - C9 (`MAX_INPUT_SAMPLE_RATE_HZ` export + dynamic error) → T2 Steps 1-2, 5.
  - C12 (`Error: Send + Sync + 'static` + doc) → T2 Steps 3-4.
  - D8 (`fft_buf.fill`) → T5 Step 1.
  - D9 (`Resampler::process` `with_capacity`) → T5 Step 2.
  - D7 (shared FFT plan) → **deferred** (CHANGELOG documents the deferral).
  - CHANGELOG → T6.

- **No new tests.** This is a public-surface polish PR. The existing test suite (136 lib + 11 roundtrip + 6 integration) is the regression net. One existing test (`invalid_sample_rate_renders_with_value`) updates in place to source the cap dynamically.

- **No behavior changes.** All 8 in-scope findings are either annotations (`#[must_use]`, derives), data-shape refactors (`HannBank`), or capacity tweaks (`scratch`, `with_capacity`). The compile-time `Error: Send + Sync + 'static` assertion exists today (the single existing variant satisfies it trivially); it becomes load-bearing when a future variant is added.

- **Compile-time vs run-time gating:**
  - `Error` variant carrying a non-`Send` type → **compile error** (load-bearing).
  - Caller drops a `#[must_use]` return value → **compile warning** (or hard error under `-D warnings` in CI).
  - Calling `HannBank::get(7)` → **runtime panic** (documented precondition; programmer-error path).

- **Out of scope** (tracked elsewhere):
  - D7 — shared FFT plan between `SnrEstimator` and `ChannelDemod`. Deferred.
  - Anything in `tests/` files (this is a public-surface sweep).
  - Renaming `lookup_vis` or other re-exports (#91 territory, already shipped).
  - Larger architectural moves (per-line allocation hoisting is #93, docs sweep is #94).
