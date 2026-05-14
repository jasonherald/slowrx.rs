# Issue #85 — Shared DSP / channel-demod modules — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract two crate-private modules from material scattered across the codebase — `src/dsp.rs` (generic primitives: Hann/power/get_bin/Goertzel) and `src/demod.rs` (SSTV per-channel demod: `ChannelDemod`, `decode_one_channel_into`, etc.) — making the module graph honest (Robot/Scottie no longer depend on `mode_pd` for shared machinery) and ending three concrete cleanups: dead `chan_bounds_abs` parameter + the 3 caller arrays that fed it, the `time_offset_seconds` → `radio_frame_offset_seconds` rename, the C20 cast-safety comment block.

**Architecture:** Bottom-up migration. T1 creates `dsp.rs` and migrates all 4 primitives + their tests in one atomic step (no duplicate-then-delete dance). T2 creates `demod.rs` populated only with `HannBank`/`HANN_LENS`/`window_idx_for_snr*` moved from `snr.rs`. T3 renames `PdDemod` → `ChannelDemod` and moves it + the four supporting fns from `mode_pd.rs` to `demod.rs`, leaving `decode_one_channel_into` in `mode_pd.rs` for now (just with its `ChannelDemod` param type). T4 moves `decode_one_channel_into` to `demod.rs` and reshapes its signature (`ChannelDecodeCtx`/`DemodState` structs, drop `chan_bounds_abs`, rename `time_offset_seconds`, add the C20 comment) — all in one atomic step so call sites only churn once. T5 lands the CHANGELOG entry, the module-level docs, and the full CI gate. After each task the crate compiles and the full test suite passes.

**Tech Stack:** Rust 2021, MSRV 1.85. Crate clippy config: `clippy::all`/`pedantic` = warn, `unwrap_used`/`panic`/`expect_used` = warn, no panics in lib code. CI gate: `cargo test --all-features --locked --release`, `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`. No GPG signing (`git commit` plain).

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-12-issue-85-shared-demod-dsp-modules-design.md`
- Audit: `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs B1, B8, B3, B5, B16, C6, C20)

---

## File Structure

| File | Status | Role |
|------|--------|------|
| `src/dsp.rs` | **new** (T1) | Generic DSP primitives: `build_hann`, `power`, `get_bin`, `goertzel_power`. |
| `src/demod.rs` | **new** (T2 creates; T3/T4 populate) | SSTV per-channel demod: `ChannelDemod`, `decode_one_channel_into`, `pixel_freq` (method on `ChannelDemod`), `freq_to_luminance`, `ycbcr_to_rgb`, `FFT_LEN`, `SNR_REESTIMATE_STRIDE`, `HannBank`, `HANN_LENS`, `window_idx_for_snr`, `window_idx_for_snr_with_hysteresis`, `ChannelDecodeCtx`, `DemodState`. |
| `src/lib.rs` | modify | `pub(crate) mod dsp;` + `pub(crate) mod demod;` added. `get_bin` removed (moved to `dsp`). `__test_support::mode_pd::ycbcr_to_rgb` re-export points at `crate::demod`. |
| `src/snr.rs` | slim | `SnrEstimator` + its 1024-sample SNR-analysis Hann stay. `HannBank` / `HANN_LENS` / `window_idx_for_snr*` / local `build_hann` / local `power` closure → gone (T1+T2). |
| `src/vis.rs` | slim | Local `build_hann_window`, `goertzel_power`, local `power` closure → gone (T1). |
| `src/sync.rs` | slim | `build_sync_hann` collapses to a call to `crate::dsp::build_hann(SYNC_FFT_WINDOW_SAMPLES)`; local `power` closure → gone (T1). |
| `src/mode_pd.rs` | slim | Keeps `decode_pd_line_pair` + PD-specific scaffolding. `PdDemod`, `FFT_LEN`, `pixel_freq`, `freq_to_luminance`, `ycbcr_to_rgb`, `SNR_REESTIMATE_STRIDE` → moved out (T3). `decode_one_channel_into` → moved out (T4). The 4 caller arrays for `chan_bounds_abs` → gone (T4). |
| `src/mode_robot.rs`, `src/mode_scottie.rs` | modify | Call-site paths flip `crate::mode_pd::*` → `crate::demod::*` (T3). The 3 caller arrays for `chan_bounds_abs` → gone (T4). |
| `src/decoder.rs` | modify | Field `pd_demod: PdDemod` → `channel_demod: ChannelDemod`; ~4 usages updated (T3). |
| `src/resample.rs` | modify (tests only) | `crate::vis::goertzel_power` → `crate::dsp::goertzel_power` in the 8 test call sites (T1). |
| `CHANGELOG.md` | modify | `[Unreleased]` gets an `### Internal` bullet (T5). |

**Task order:** T1 → T2 → T3 → T4 → T5. Each task leaves a working state (crate compiles, full `cargo test --release` passes).

**Verification after every commit** (the rule for this PR — wide blast radius):
```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

---

## Task 1: Create `src/dsp.rs` and migrate the 4 primitives

Create the new `dsp` module populated with the 4 canonical primitives, then migrate every consumer in one atomic task so there are no duplicate-then-delete intermediate states.

**Files:**
- Create: `src/dsp.rs`
- Modify: `src/lib.rs`, `src/vis.rs`, `src/sync.rs`, `src/snr.rs`, `src/mode_pd.rs`, `src/resample.rs`

- [ ] **Step 1: Create `src/dsp.rs` with the 4 primitives**

```rust
//! Generic DSP primitives used across the decoder.
//!
//! - [`build_hann`] — the canonical zero/one-safe Hann-window builder
//!   (the four pre-#85 copies in `vis.rs` / `sync.rs` / `snr.rs` /
//!   `mode_pd.rs` collapse here, with the safe `n <= 1` handling).
//! - [`power`] — `|c|²` as `f64`, the replacement for three inline-closure
//!   copies.
//! - [`get_bin`] — frequency → FFT bin index with slowrx C-truncation
//!   semantics (moved from `lib.rs`).
//! - [`goertzel_power`] — single-bin DFT magnitude² (moved from `vis.rs`).
//!
//! Nothing here is SSTV-specific; per-channel demod machinery lives in
//! [`crate::demod`].

use rustfft::num_complex::Complex;

/// Build a Hann window of length `len`. Used for both the per-pixel demod's
/// [`crate::demod::HannBank`] entries (lengths from
/// [`crate::demod::HANN_LENS`]) and the [`crate::snr::SnrEstimator`]'s
/// `FFT_LEN`-sample `hann_long`.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn build_hann(len: usize) -> Vec<f32> {
    if len == 0 {
        return Vec::new();
    }
    if len == 1 {
        return vec![0.0_f32];
    }
    (0..len)
        .map(|i| {
            let m = (len - 1) as f32;
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * (i as f32) / m).cos())
        })
        .collect()
}

/// `|c|²` as `f64`. Used wherever an FFT bin's power (= magnitude²) is needed.
#[inline]
pub(crate) fn power(c: Complex<f32>) -> f64 {
    let r = f64::from(c.re);
    let i = f64::from(c.im);
    r * r + i * i
}

/// Translate a frequency in Hz to the nearest FFT bin index using slowrx's
/// C-truncation semantics.
///
/// slowrx's `GetBin` (`common.c:39-41`) is:
/// ```c
/// guint GetBin(double Freq, guint FFTLen) {
///     return (Freq / 44100 * FFTLen);  // implicit double→uint = truncation toward zero
/// }
/// ```
///
/// The implicit `double → guint` cast truncates toward zero. We replicate
/// this with an `as usize` cast (well-defined for positive doubles: truncates
/// toward zero), which gives the same result as C for all frequencies used
/// in slowrx. **Do NOT change this to `.round()`** — that would deviate from
/// slowrx's bin assignments at 5 of the 8 production frequencies (800, 1200,
/// 1500, 2700, 3400 Hz), shifting SNR-estimator bandwidth divisors and the
/// sync tracker's `Praw`/`Psync` range.
///
/// # Numerical verification (both at slowrx-native 1024/44100 and our 256/11025
/// — same Hz/bin ratio, so bins are identical)
///
/// | Frequency | Expected bin |
/// |-----------|-------------|
/// | 400 Hz    | 9           |
/// | 800 Hz    | 18          |
/// | 1200 Hz   | 27          |
/// | 1500 Hz   | 34          |
/// | 1900 Hz   | 44          |
/// | 2300 Hz   | 53          |
/// | 2700 Hz   | 62          |
/// | 3400 Hz   | 78          |
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
#[inline]
pub(crate) fn get_bin(hz: f64, fft_len: usize, sample_rate_hz: u32) -> usize {
    (hz * fft_len as f64 / f64::from(sample_rate_hz)) as usize
}

/// Goertzel power on `samples` at `target_hz` (bin power, ~amplitude²).
/// Used by `decoder::estimate_freq` and the resample-quality tests.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn goertzel_power(samples: &[f32], target_hz: f64) -> f64 {
    let n = samples.len() as f64;
    if n == 0.0 {
        return 0.0;
    }
    let k = (0.5 + n * target_hz / f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ)).floor();
    let coeff = 2.0 * (2.0 * std::f64::consts::PI * k / n).cos();
    let mut s_prev = 0.0_f64;
    let mut s_prev2 = 0.0_f64;
    for &sample in samples {
        let s = f64::from(sample) + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }
    s_prev2.mul_add(s_prev2, s_prev.mul_add(s_prev, -coeff * s_prev * s_prev2))
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    /// Verify `get_bin` truncation matches slowrx's C `guint GetBin(double, guint)`.
    #[test]
    fn get_bin_matches_slowrx_truncation() {
        let cases: &[(f64, usize)] = &[
            (400.0, 9),
            (800.0, 18),
            (1190.0, 27),
            (1200.0, 27),
            (1500.0, 34),
            (1900.0, 44),
            (2300.0, 53),
            (2700.0, 62),
            (3400.0, 78),
        ];
        for &(hz, expected) in cases {
            let bin_ours = get_bin(hz, 256, 11025);
            let bin_slowrx = get_bin(hz, 1024, 44100);
            assert_eq!(bin_ours, expected, "get_bin({hz}, 256, 11025) = {bin_ours}, expected {expected}");
            assert_eq!(bin_slowrx, expected, "get_bin({hz}, 1024, 44100) = {bin_slowrx}, expected {expected}");
        }
    }

    #[test]
    fn build_hann_zero_and_one_length_safe() {
        assert!(build_hann(0).is_empty());
        let one = build_hann(1);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0], 0.0);
    }

    #[test]
    fn build_hann_endpoints_are_zero_and_middle_is_one() {
        let h = build_hann(256);
        assert!(h[0].abs() < 1e-6);
        assert!(h[h.len() - 1].abs() < 1e-6);
        let mid = h.len() / 2;
        assert!((h[mid] - 1.0).abs() < 1e-2, "middle ≈ 1, got {}", h[mid]);
    }

    #[test]
    fn goertzel_empty_input_returns_zero_power() {
        assert_eq!(goertzel_power(&[], 1900.0), 0.0);
    }

    #[test]
    fn goertzel_handcomputed_quarter_cycle() {
        let samples = [1.0_f32, 0.0, -1.0, 0.0];
        let target = f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ) / 4.0;
        let p = goertzel_power(&samples, target);
        assert!((p - 4.0).abs() < 1e-9, "expected 4.0, got {p}");
    }
}
```

- [ ] **Step 2: Wire `dsp` into `src/lib.rs` + remove the inline `get_bin`**

In `src/lib.rs`, find the `pub mod` / `pub(crate) mod` declarations block. Add immediately above the existing `pub mod decoder;` line:

```rust
pub(crate) mod dsp;
```

Then locate the inline `pub(crate) fn get_bin(...)` definition (with its big bin-table doc-comment) and the `mod tests_common { ... }` block that contains `get_bin_matches_slowrx_truncation`. **Delete both** — the fn moved to `dsp.rs` in Step 1, and the test moved with it. Search for any remaining `pub(crate) fn get_bin` in `lib.rs` to confirm it's gone.

- [ ] **Step 3: Update callers of `crate::get_bin` → `crate::dsp::get_bin`**

Run: `grep -rn "crate::get_bin\b" src/`
Expected hits (update each to `crate::dsp::get_bin`):

```text
src/mode_pd.rs
src/vis.rs
src/snr.rs
src/sync.rs
```

(Use a single search-and-replace within each file. The semantics are identical — same fn, new path.)

- [ ] **Step 4: Migrate `src/vis.rs` — drop its `build_hann_window`, `goertzel_power`, and `power` closure**

In `src/vis.rs`:

(a) **Replace** the call to `build_hann_window(WINDOW_SAMPLES)` in `VisDetector::new` (look for `hann: build_hann_window(WINDOW_SAMPLES)`) with `hann: crate::dsp::build_hann(WINDOW_SAMPLES)`.

(b) **Delete** the `fn build_hann_window(n: usize) -> Vec<f32> { ... }` block (currently around lines 270-282). Both the fn and its `#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]` attribute go.

(c) In `estimate_peak_freq` (around line 290-330), **delete** the `let power = |c: Complex<f32>| -> f64 { ... };` closure declaration and **add** `use crate::dsp::power;` at the top of the file's `use` block — OR change the four `power(...)` call sites to `crate::dsp::power(...)`. (Pick the `use` form for readability; one import.) Same change for any `power` closure in `vis.rs`'s `process_hop` or elsewhere.

(d) **Delete** the `pub(crate) fn goertzel_power(samples: &[f32], target_hz: f64) -> f64 { ... }` block (currently around lines 442-475) — moved to `dsp.rs` in Step 1.

(e) In `vis.rs::tests`, **delete** the `empty_input_returns_zero_power` and `goertzel_handcomputed_quarter_cycle` tests (moved to `dsp::tests` in Step 1). **Also delete** `hann_window_endpoints_are_zero` if present (the one testing `build_hann_window` directly) — its replacement is `dsp::tests::build_hann_endpoints_are_zero_and_middle_is_one`.

- [ ] **Step 5: Migrate `src/sync.rs` — collapse `build_sync_hann` and drop its `power` closure**

In `src/sync.rs`:

(a) **Replace** the body of `fn build_sync_hann() -> Vec<f32>` (currently lines ~179-189) so the function just calls `crate::dsp::build_hann`:

```rust
fn build_sync_hann() -> Vec<f32> {
    crate::dsp::build_hann(SYNC_FFT_WINDOW_SAMPLES)
}
```

Drop the `#[allow(clippy::cast_precision_loss)]` attribute on it — it's just a wrapper now.

(b) In `has_sync_at` (around line 146), **delete** the `let power = |c: Complex<f32>| -> f64 { ... };` closure and change `power(self.fft_buf[k])` calls to `crate::dsp::power(self.fft_buf[k])` (or add `use crate::dsp::power;` near the top of the file).

- [ ] **Step 6: Migrate `src/snr.rs` — drop its local `build_hann` (and `power` closure if any)**

In `src/snr.rs`:

(a) **Delete** the `fn build_hann(len: usize) -> Vec<f32> { ... }` definition (currently around lines 47-66 — the version with the `len == 0` / `len == 1` guards). Its `#[allow]` attribute goes too.

(b) In `HannBank::new` (currently calls `build_hann(HANN_LENS[i])`), change each call to `crate::dsp::build_hann(HANN_LENS[i])`. (HannBank still lives in `snr.rs` for now — T2 moves it.)

(c) In `SnrEstimator::new` (or wherever the 1024-sample `hann_long` is built), change `build_hann(FFT_LEN)` to `crate::dsp::build_hann(FFT_LEN)`.

(d) If `snr.rs` has any local `power` closures, replace with `crate::dsp::power` (search: `grep -n "|c: Complex" src/snr.rs`).

(e) In `snr.rs::tests`, **delete** `build_hann_zero_and_one_length_safe` (moved to `dsp::tests` in Step 1).

- [ ] **Step 7: Migrate `src/mode_pd.rs` — drop its `power` closure**

In `src/mode_pd.rs::PdDemod::pixel_freq` (around lines 170-180), **delete** the `let power = |c: Complex<f32>| -> f64 { ... };` closure (if it exists as a separate binding — it may already be inline) and replace `power(c)` / `power(self.fft_buf[lo])` calls with `crate::dsp::power(c)` / `crate::dsp::power(self.fft_buf[lo])`. If there are several call sites, add `use crate::dsp::power;` near the top of the file (just below the existing `use` lines).

(The PD-specific FFT struct still owns its `hann_bank` and computes its own bin search — only the `power` closure goes.)

- [ ] **Step 8: Update `src/resample.rs` test references**

In `src/resample.rs::tests`, change every `crate::vis::goertzel_power` to `crate::dsp::goertzel_power`. Run: `grep -n "goertzel_power" src/resample.rs` — expect 8 hits, all in the test module.

- [ ] **Step 9: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. The `cargo test` step is the load-bearing one (it exercises every consumer of every primitive — `vis`, `sync`, `snr`, `mode_pd` via `roundtrip.rs`).

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "refactor(dsp): extract build_hann / power / get_bin / goertzel_power to crate::dsp (#85 B8)"
```

---

## Task 2: Create `src/demod.rs` and move `HannBank` / `HANN_LENS` / `window_idx_for_snr*` from `snr.rs`

Pure mechanical move. `demod.rs` is created for the first time, populated only with the moved items. The single consumer (`mode_pd::PdDemod`, which references `crate::snr::HannBank` and `crate::snr::HANN_LENS`) is updated to use the new paths.

**Files:**
- Create: `src/demod.rs`
- Modify: `src/lib.rs`, `src/snr.rs`, `src/mode_pd.rs`

- [ ] **Step 1: Create `src/demod.rs` with the moved items**

Write to `src/demod.rs`:

```rust
//! SSTV per-channel demod machinery.
//!
//! The per-pixel FFT, adaptive Hann-window selection, and YCbCr→RGB
//! conversion that turn the working-rate audio of one image-line channel
//! into pixel bytes. Consumed by `mode_pd` (PD-family), `mode_robot`
//! (Robot-family), and `mode_scottie` (Scottie/Martin RGB-sequential).
//!
//! `HannBank` / `HANN_LENS` / `window_idx_for_snr{_with_hysteresis}` moved
//! here from `crate::snr` (#85 B1): they're per-pixel-demod machinery, not
//! SNR-estimation logic. The `SnrEstimator` itself stays in `crate::snr` and
//! carries its own separate 1024-sample SNR-analysis Hann.

/// Adaptive Hann-window lengths used by the per-pixel demod. Index 0 is the
/// shortest window (best at high SNR — preserves edges); index 6 is the
/// longest (best at low SNR — averages out noise). Translated from slowrx's
/// `video.c:54`.
pub(crate) const HANN_LENS: [usize; 7] = [12, 16, 24, 32, 64, 128, 256];

/// Bank of seven Hann windows, indexed by SNR-derived window selector.
/// Construct once per decoder; the inner `Vec<f32>`s have lengths matching
/// [`HANN_LENS`].
pub(crate) struct HannBank {
    windows: [Vec<f32>; 7],
}

impl HannBank {
    pub fn new() -> Self {
        Self {
            windows: [
                crate::dsp::build_hann(HANN_LENS[0]),
                crate::dsp::build_hann(HANN_LENS[1]),
                crate::dsp::build_hann(HANN_LENS[2]),
                crate::dsp::build_hann(HANN_LENS[3]),
                crate::dsp::build_hann(HANN_LENS[4]),
                crate::dsp::build_hann(HANN_LENS[5]),
                crate::dsp::build_hann(HANN_LENS[6]),
            ],
        }
    }

    /// Borrow window `idx` (0..=6). Length is `HANN_LENS[idx]`.
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

/// SNR → Hann-window-index mapping (bare, no hysteresis). slowrx
/// `video.c:354-364`'s seven-threshold ladder, translated literally.
#[must_use]
pub(crate) fn window_idx_for_snr(snr_db: f64) -> usize {
    if snr_db >= 20.0 {
        0
    } else if snr_db >= 10.0 {
        1
    } else if snr_db >= 9.0 {
        2
    } else if snr_db >= 3.0 {
        3
    } else if snr_db >= -5.0 {
        4
    } else if snr_db >= -10.0 {
        5
    } else {
        6
    }
}

/// Hysteresis variant of [`window_idx_for_snr`]. Takes a `prev_idx` (the
/// window index used by the previous FFT in this channel decode) and applies
/// a 1 dB hysteresis band at every threshold to prevent flip-flop on
/// real-radio SNR fluctuations near boundary values. See
/// `docs/intentional-deviations.md` for the rationale and the deliberate
/// divergence from slowrx C's pure-threshold logic.
#[must_use]
pub(crate) fn window_idx_for_snr_with_hysteresis(snr_db: f64, prev_idx: usize) -> usize {
    const HYSTERESIS_DB_HALF: f64 = 0.5;

    let baseline = window_idx_for_snr(snr_db);
    if baseline == prev_idx {
        return prev_idx;
    }

    let target_idx = if baseline > prev_idx {
        prev_idx + 1
    } else {
        prev_idx - 1
    };

    let shifted_snr = if target_idx < prev_idx {
        snr_db - HYSTERESIS_DB_HALF
    } else {
        snr_db + HYSTERESIS_DB_HALF
    };
    let shifted_idx = window_idx_for_snr(shifted_snr);

    let robust = if target_idx < prev_idx {
        shifted_idx <= target_idx
    } else {
        shifted_idx >= target_idx
    };

    if robust {
        target_idx
    } else {
        prev_idx
    }
}
```

(The bodies of `window_idx_for_snr` and `window_idx_for_snr_with_hysteresis` are copied **verbatim** from `src/snr.rs` — preserve the existing rich doc comments, just elide them here for brevity. The implementer should copy the full doc-comment blocks from the current `snr.rs` versions.)

- [ ] **Step 2: Wire `demod` into `src/lib.rs`**

In `src/lib.rs`, add immediately below the `pub(crate) mod dsp;` line (added in T1 Step 2):

```rust
pub(crate) mod demod;
```

- [ ] **Step 3: Delete the moved items from `src/snr.rs`**

In `src/snr.rs`, **delete** (the items now live in `crate::demod`):
- The `pub(crate) const HANN_LENS: [usize; 7] = [...];` definition.
- The `pub(crate) struct HannBank { ... }` definition and its `impl HannBank { new, get }` + `impl Default for HannBank`.
- The `pub(crate) fn window_idx_for_snr(...)` definition (with its doc comment).
- The `pub(crate) fn window_idx_for_snr_with_hysteresis(...)` definition (with its doc comment).
- The module-level `//! - **Per-pixel demod**: HANN_LENS stays at slowrx's lengths divided / ...` doc paragraph that documents `HANN_LENS`/`HannBank` — rewrite the module-doc block to refer to `crate::demod` instead. (Keep the part describing `SnrEstimator`'s own 1024-sample Hann.)

In `snr::tests`, **delete** any tests that target the moved items (`HannBank` construction, `window_idx_for_snr*` cases). They move to `demod::tests` in Step 4.

- [ ] **Step 4: Move the relevant tests from `snr::tests` to `demod::tests`**

Add a `#[cfg(test)] mod tests` block at the end of `src/demod.rs` containing the tests that were in `snr::tests` for `HannBank` / `window_idx_for_snr` / `window_idx_for_snr_with_hysteresis`. Copy them verbatim (with `use super::*;`) — adjust paths only if a test referenced something that's also moved.

Run: `grep -n "fn.*HannBank\|fn.*window_idx_for_snr" src/snr.rs` — expect no hits.

- [ ] **Step 5: Update `src/mode_pd.rs` import paths**

`grep -rn "crate::snr::HannBank\|crate::snr::HANN_LENS\|crate::snr::window_idx_for_snr" src/` — expect hits in `src/mode_pd.rs` only. Update each:
- `crate::snr::HannBank` → `crate::demod::HannBank`
- `crate::snr::HANN_LENS` → `crate::demod::HANN_LENS`
- `crate::snr::window_idx_for_snr_with_hysteresis` → `crate::demod::window_idx_for_snr_with_hysteresis`
- `crate::snr::window_idx_for_snr` → `crate::demod::window_idx_for_snr` (if used)

Also any `[doc-link]`-style `[`crate::snr::HannBank`]` references in `mode_pd.rs`'s doc comments — update to `[`crate::demod::HannBank`]`. `cargo doc -D warnings` is the gate that catches broken intra-doc links.

- [ ] **Step 6: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(demod): move HannBank / HANN_LENS / window_idx_for_snr* from snr to crate::demod (#85 B1)"
```

---

## Task 3: Rename `PdDemod` → `ChannelDemod` and move it (+ `FFT_LEN`, `pixel_freq`, `freq_to_luminance`, `ycbcr_to_rgb`, `SNR_REESTIMATE_STRIDE`) to `src/demod.rs`

The big rename. `decode_one_channel_into` stays in `mode_pd.rs` for this task (param type just flips from `&mut PdDemod` to `&mut ChannelDemod`) — T4 moves it.

**Files:**
- Modify: `src/demod.rs`, `src/mode_pd.rs`, `src/decoder.rs`, `src/mode_robot.rs`, `src/mode_scottie.rs`, `src/lib.rs`

- [ ] **Step 1: Append `ChannelDemod` + supporting items to `src/demod.rs`**

Add to `src/demod.rs` (below the items added in T2):

```rust
use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::sync::Arc;

/// FFT length used by the per-pixel demod (and by `SnrEstimator`, which
/// re-exports this constant). 1024 at the 11025 Hz working rate gives
/// ~10.8 Hz/bin — 4× slowrx C's resolution (which uses 1024 at 44100 Hz,
/// ~43 Hz/bin). See `docs/intentional-deviations.md` "FFT frequency
/// resolution exceeds slowrx C by 4×".
pub(crate) const FFT_LEN: usize = 1024;

/// SNR re-estimation stride in working-rate samples. The per-pixel demod
/// loop re-runs the SNR estimator every `SNR_REESTIMATE_STRIDE` samples to
/// keep the adaptive Hann-window selector responsive to changing channel
/// conditions inside a single image line. Translated from slowrx
/// `video.c:312`-ish (slowrx re-estimates on a similar cadence).
pub(crate) const SNR_REESTIMATE_STRIDE: i64 = 64;

/// Per-pixel demod context: holds an FFT plan + reusable buffers + the
/// adaptive Hann-window bank. Construct once per decoder; reuse for many
/// [`ChannelDemod::pixel_freq`] calls.
pub(crate) struct ChannelDemod {
    fft: Arc<dyn Fft<f32>>,
    hann_bank: HannBank,
    fft_buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl ChannelDemod {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_LEN);
        let scratch_len = fft.get_inplace_scratch_len();
        Self {
            fft,
            hann_bank: HannBank::new(),
            fft_buf: vec![Complex { re: 0.0, im: 0.0 }; FFT_LEN],
            scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len.max(FFT_LEN)],
        }
    }

    /// Estimate the dominant tone frequency in a Hann-windowed FFT centered
    /// near `center_sample`. Body is the existing `mode_pd::PdDemod::pixel_freq`
    /// verbatim — copy from `src/mode_pd.rs` lines ~120-230. Replace any
    /// `crate::snr::HANN_LENS` → `HANN_LENS`, any `crate::get_bin` → `crate::dsp::get_bin`,
    /// any local `power` closure → `crate::dsp::power`.
    #[allow(/* same #[allow]s as the existing PdDemod::pixel_freq */)]
    pub fn pixel_freq(
        &mut self,
        audio: &[f32],
        center_sample: i64,
        win_idx: usize,
        hedr_shift_hz: f64,
    ) -> f64 {
        // … body copied verbatim from mode_pd::PdDemod::pixel_freq …
        todo!("copy body from mode_pd.rs")
    }
}

impl Default for ChannelDemod {
    fn default() -> Self {
        Self::new()
    }
}

/// Slowrx `freq_to_luminance` (`video.c:69-72`). Maps a per-pixel frequency
/// in Hz to a 0..=255 luminance byte, clamping the [1500, 2300] Hz video
/// band to the full dynamic range. `hedr_shift_hz` shifts the band for
/// radio-mistuning correction.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub(crate) fn freq_to_luminance(freq_hz: f64, hedr_shift_hz: f64) -> u8 {
    // … body copied verbatim from mode_pd::freq_to_luminance …
    todo!("copy body from mode_pd.rs")
}

/// YCbCr → RGB (BT.601 / JPEG convention) — slowrx `common.c:118-138`,
/// rounded with the same `clip` semantics.
///
/// **Not part of the stable public API** — surfaced only via
/// `__test_support::mode_pd::ycbcr_to_rgb` for the synthetic round-trip
/// integration tests (`tests/roundtrip.rs`). The audit (C6) asked for
/// `pub(crate)`, but the existing `pub use` re-export inside the `pub mod
/// __test_support` requires the source to be `pub` (rustc rejects
/// re-exporting a `pub(crate)` item at a `pub` path — "private item in
/// public interface"). `#[doc(hidden)]` achieves the same intent — kept off
/// the documented API surface — without breaking the re-export.
#[doc(hidden)]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn ycbcr_to_rgb(y: u8, cr: u8, cb: u8) -> [u8; 3] {
    // … body copied verbatim from mode_pd::ycbcr_to_rgb …
    todo!("copy body from mode_pd.rs")
}
```

**Step 1a — actually fill in the bodies:** open `src/mode_pd.rs` and copy:
- `pixel_freq` body (the entire method body of `PdDemod::pixel_freq`) → replace the `todo!()` in `ChannelDemod::pixel_freq`. Keep the exact `#[allow(...)]` attributes from the original.
- `freq_to_luminance` body → replace its `todo!()`.
- `ycbcr_to_rgb` body → replace its `todo!()`.

Then **delete** those four items (`PdDemod` struct + `impl PdDemod`, the standalone `freq_to_luminance`, the standalone `ycbcr_to_rgb`, the `pub(crate) const FFT_LEN`, the `pub(crate) const SNR_REESTIMATE_STRIDE`) from `src/mode_pd.rs`.

- [ ] **Step 2: Move the relevant unit tests from `mode_pd::tests` to `demod::tests`**

In `src/mode_pd.rs::tests`, identify the tests that target the moved items — they're the ones that construct a `PdDemod::new()` (you saw 5 hits in the grep) plus any tests for `freq_to_luminance` or `ycbcr_to_rgb`. Copy those tests verbatim into `src/demod.rs::tests` (the test module added in T2 Step 4). Update:
- `PdDemod::new()` → `ChannelDemod::new()`
- `crate::mode_pd::pixel_freq` → just `pixel_freq` (it's a method on the local `ChannelDemod`)
- `crate::mode_pd::freq_to_luminance` → just `freq_to_luminance`
- `crate::mode_pd::ycbcr_to_rgb` → just `ycbcr_to_rgb`

Then **delete** those tests from `mode_pd::tests`. Tests for `decode_pd_line_pair` (the PD-line-pair integration tests) **stay** in `mode_pd::tests`.

- [ ] **Step 3: Update `src/mode_pd.rs` to use the new paths**

In `src/mode_pd.rs`:

(a) Remove any `pub(crate) use crate::snr::FFT_LEN as ...` alias (the `FFT_LEN` line at ~line 80 — `pub(crate) const FFT_LEN: usize = crate::snr::FFT_LEN;`). The constant now lives canonically in `crate::demod::FFT_LEN`. If `mode_pd.rs` needs `FFT_LEN`, import it: `use crate::demod::FFT_LEN;` at the top.

(b) `decode_pd_line_pair`'s signature: `demod: &mut crate::mode_pd::PdDemod` → `demod: &mut crate::demod::ChannelDemod`.

(c) `decode_one_channel_into`'s signature (still in `mode_pd.rs` for now): `demod: &mut PdDemod` → `demod: &mut crate::demod::ChannelDemod`.

(d) Any `crate::mode_pd::FFT_LEN` / `crate::mode_pd::SNR_REESTIMATE_STRIDE` references → drop `mode_pd::` (now imported via `use`).

(e) Update intra-doc-link references — search for `[`crate::mode_pd::ycbcr_to_rgb`]` / `[`crate::mode_pd::PdDemod`]` / `[`crate::mode_pd::FFT_LEN`]` / `[`crate::mode_pd::pixel_freq`]` / `[`crate::mode_pd::freq_to_luminance`]` in `mode_pd.rs` doc comments and rewrite to `crate::demod::*` equivalents.

- [ ] **Step 4: Update `src/decoder.rs` — field rename + usage updates**

In `src/decoder.rs`:

(a) `SstvDecoder` field: `pd_demod: crate::mode_pd::PdDemod,` → `channel_demod: crate::demod::ChannelDemod,`. Update the doc-comment above it (currently mentions `PdDemod`).

(b) `SstvDecoder::new`: `pd_demod: crate::mode_pd::PdDemod::new(),` → `channel_demod: crate::demod::ChannelDemod::new(),`.

(c) `SstvDecoder::reset` (if it resets the demod): `self.pd_demod = crate::mode_pd::PdDemod::new();` → `self.channel_demod = crate::demod::ChannelDemod::new();`.

(d) `run_findsync_and_decode` and `process`: every `&mut self.pd_demod` → `&mut self.channel_demod`. Expect ~3-4 usages.

(e) Search: `grep -n "pd_demod\|PdDemod" src/decoder.rs` — expect zero hits after this step.

- [ ] **Step 5: Update `src/mode_robot.rs` and `src/mode_scottie.rs` call sites**

In `src/mode_robot.rs` and `src/mode_scottie.rs`:

(a) Every `&mut crate::mode_pd::PdDemod` (param type in fn signatures) → `&mut crate::demod::ChannelDemod`.

(b) Every `crate::mode_pd::decode_one_channel_into` (call sites — `decode_one_channel_into` is still in `mode_pd` for this task, so the path stays) — leave unchanged for now; T4 flips it to `crate::demod::decode_one_channel_into`.

(c) Search: `grep -rn "PdDemod" src/` — expect zero hits.

- [ ] **Step 6: Update `src/lib.rs` `__test_support` re-export**

In `src/lib.rs`, find the `pub mod mode_pd { ... }` block inside `pub mod __test_support`. Change `pub use crate::mode_pd::ycbcr_to_rgb;` → `pub use crate::demod::ycbcr_to_rgb;`. The export path `slowrx::__test_support::mode_pd::ycbcr_to_rgb` is unchanged for callers (this is the audit's C6 stability promise — `tests/roundtrip.rs` keeps working without any test-file change).

- [ ] **Step 7: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

`cargo test --release` is load-bearing: `tests/roundtrip.rs` exercises `decode_one_channel_into` (still in `mode_pd.rs`) which now takes `&mut crate::demod::ChannelDemod`. Any rename miss surfaces here.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor(demod): move ChannelDemod (renamed from PdDemod) + FFT_LEN / pixel_freq / freq_to_luminance / ycbcr_to_rgb to crate::demod (#85 B1, C6)"
```

---

## Task 4: Move `decode_one_channel_into` to `crate::demod`, introduce `ChannelDecodeCtx`/`DemodState`, drop `chan_bounds_abs`, rename `time_offset_seconds` → `radio_frame_offset_seconds`, add C20 cast-safety comment

The substantive change. All call sites churn once (the rename in T3 left them pointing at the old fn; this task moves and reshapes the fn AND updates every call site in one atomic commit).

**Files:**
- Modify: `src/demod.rs` (target — receives the moved fn + the new structs)
- Modify: `src/mode_pd.rs` (source — `decode_one_channel_into` leaves; `decode_pd_line_pair` call sites update)
- Modify: `src/mode_robot.rs` (3 call sites update)
- Modify: `src/mode_scottie.rs` (call site updates)

- [ ] **Step 1: Add `ChannelDecodeCtx` + `DemodState` to `src/demod.rs`**

Append to `src/demod.rs` (above where `decode_one_channel_into` will land):

```rust
/// Per-channel-decode-call invariants — these don't change between channels
/// of the same image. Bundled to cut `decode_one_channel_into`'s signature
/// from 11 args down to 5 (#85 B3).
pub(crate) struct ChannelDecodeCtx<'a> {
    pub audio: &'a [f32],
    pub skip_samples: i64,
    pub rate_hz: f64,
    pub hedr_shift_hz: f64,
    pub spec: crate::modespec::ModeSpec,
}

/// Per-call mutable state: the channel demod's FFT + Hann bank, plus the SNR
/// estimator. Lifetime-only borrow; neither field is owned here.
pub(crate) struct DemodState<'a> {
    pub demod: &'a mut ChannelDemod,
    pub snr: &'a mut crate::snr::SnrEstimator,
}
```

- [ ] **Step 2: Add the new `decode_one_channel_into` to `src/demod.rs`**

Append to `src/demod.rs`:

```rust
/// Decode one image-line *channel* worth of audio into `out` (one luminance
/// byte per output pixel).
///
/// Body translated from slowrx `video.c:124-200` — copy verbatim from the
/// existing `crate::mode_pd::decode_one_channel_into` minus the dead
/// `chan_bounds_abs` parameter (#45 / #85 B5) and with `time_offset_seconds`
/// renamed to `radio_frame_offset_seconds` (#85 B16). Param accesses through
/// `ctx` and `state` (#85 B3).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
pub(crate) fn decode_one_channel_into(
    out: &mut [u8],
    chan_start_sec: f64,
    radio_frame_offset_seconds: f64,
    ctx: &ChannelDecodeCtx<'_>,
    state: &mut DemodState<'_>,
) {
    // SAFETY of the f64→i64 / f64→usize casts below: every `.round() as i64`
    // / `as usize` in this fn computes a sample-buffer index. Out-of-range
    // values either saturate to i64::MAX/MIN (turning into out-of-bounds
    // reads that the `audio.get(...)` / explicit `.max(0).min(audio.len())`
    // logic resolves to 0.0 = silence) or are clamped before indexing.
    // Nothing panics on an unexpected f64; the worst case is a black pixel.
    // (#85 C20.)

    // … body copied verbatim from src/mode_pd.rs::decode_one_channel_into,
    // with these textual substitutions throughout:
    //   audio                 → ctx.audio
    //   skip_samples          → ctx.skip_samples
    //   rate_hz               → ctx.rate_hz
    //   hedr_shift_hz         → ctx.hedr_shift_hz
    //   spec                  → ctx.spec
    //   demod                 → state.demod
    //   snr_est               → state.snr
    //   time_offset_seconds   → radio_frame_offset_seconds
    //
    // Delete the line `let _ = chan_bounds_abs;` (the dead-discard of the
    // dropped parameter) and the stale "used to zero-pad outside the active
    // channel" inner comment that contradicts the `#32-lifted` note above.
}
```

**Step 2a — actually fill in the body:** open `src/mode_pd.rs::decode_one_channel_into` (currently at ~line 398), copy its body (everything inside the outer `{ }`), paste into the new `crate::demod::decode_one_channel_into`, then apply the textual substitutions listed in the comment above. Verify the body builds standalone (paths like `crate::dsp::*`, `crate::demod::HANN_LENS`, etc. resolve from `demod.rs`).

- [ ] **Step 3: Delete `decode_one_channel_into` from `src/mode_pd.rs`**

In `src/mode_pd.rs`, delete the entire `pub(crate) fn decode_one_channel_into(...) { ... }` block (and its `#[allow(...)]` attribute and doc comment). Confirm: `grep -n "fn decode_one_channel_into" src/mode_pd.rs` → no hits.

- [ ] **Step 4: Update call sites in `src/mode_pd.rs::decode_pd_line_pair`**

In `decode_pd_line_pair` (currently around lines 292-380):

(a) **Delete** the `let chan_bounds_abs: [(i64, i64); 4] = std::array::from_fn(|i| { ... });` block (around lines 330-348, with its ~8 `.round()` calls).

(b) **Delete** the doc-comment paragraph that explained `chan_bounds_abs`'s zero-pad rationale (the one that contradicts the `#32-lifted` note — audit E5).

(c) Before the `for chan_idx in 0..4` (or equivalent channel loop), construct `ctx` and prepare `state`:

```rust
let ctx = crate::demod::ChannelDecodeCtx {
    audio,
    skip_samples,
    rate_hz,
    hedr_shift_hz,
    spec,
};
```

(d) Change each `decode_one_channel_into(...)` call inside the loop from the 11-arg form:

```rust
crate::mode_pd::decode_one_channel_into(
    out,
    chan_start_sec,
    chan_bounds_abs[chan_idx],
    spec,
    audio,
    skip_samples,
    pair_seconds,
    rate_hz,
    demod,
    snr_est,
    hedr_shift_hz,
);
```

to the 5-arg form:

```rust
crate::demod::decode_one_channel_into(
    out,
    chan_start_sec,
    pair_seconds,  // the radio-frame offset
    &ctx,
    &mut crate::demod::DemodState { demod, snr: snr_est },
);
```

(`DemodState` is constructed inline at each call to keep `demod` and `snr_est`'s borrows fresh for each iteration; the struct itself is zero-cost — just a pair of `&mut` references.)

(e) Rename the local binding `pair_seconds` to `radio_frame_offset_seconds` (and update any inner uses) — OR keep `pair_seconds` as the call-site name since it's mode-specific and reads clearer locally. (Pick whichever is least disruptive; the param at the callee is `radio_frame_offset_seconds`, the local name doesn't have to match.)

- [ ] **Step 5: Update call sites in `src/mode_robot.rs`**

`mode_robot.rs` has the same pattern in `decode_r36_r24_line` (lines ~138-160) and `decode_r72_line` (lines ~260-285):

(a) In `decode_r36_r24_line`: **delete** `let chan_bounds_abs: [(i64, i64); 3] = std::array::from_fn(...);`. Before the channel loop, build `ctx`:

```rust
let ctx = crate::demod::ChannelDecodeCtx {
    audio,
    skip_samples,
    rate_hz,
    hedr_shift_hz,
    spec,
};
```

Change the `crate::mode_pd::decode_one_channel_into(out, chan_start_sec, chan_bounds_abs[chan_idx], spec, audio, skip_samples, line_seconds_offset, rate_hz, demod, snr_est, hedr_shift_hz)` call to:

```rust
crate::demod::decode_one_channel_into(
    out,
    chan_start_sec,
    line_seconds_offset,
    &ctx,
    &mut crate::demod::DemodState { demod, snr: snr_est },
);
```

(b) In `decode_r72_line` (3 channel calls, with whatever locals it uses for offsets — the spec said no `chan_bounds_abs` array there but check; if there is one, delete it the same way): build `ctx` once before the calls, then each call drops to the 5-arg form. Same `crate::mode_pd::decode_one_channel_into` → `crate::demod::decode_one_channel_into` flip.

- [ ] **Step 6: Update call sites in `src/mode_scottie.rs`**

In `src/mode_scottie.rs::decode_line` (around lines 110-135):

(a) **Delete** the `let chan_bounds_abs: [(i64, i64); 3] = std::array::from_fn(...);` array.

(b) Before the channel loop, build `ctx` (same shape as Step 5(a)).

(c) Change each `crate::mode_pd::decode_one_channel_into(...)` call to `crate::demod::decode_one_channel_into(out, chan_start_sec, line_seconds_offset, &ctx, &mut crate::demod::DemodState { demod, snr: snr_est })`.

- [ ] **Step 7: Sanity grep**

Run:
```bash
grep -rn "chan_bounds_abs\|time_offset_seconds\|crate::mode_pd::decode_one_channel_into\|crate::mode_pd::PdDemod\|crate::mode_pd::ycbcr_to_rgb\|crate::mode_pd::FFT_LEN\|crate::mode_pd::pixel_freq\|crate::mode_pd::freq_to_luminance\|crate::snr::HannBank\|crate::snr::HANN_LENS\|crate::snr::window_idx_for_snr\|fn build_hann_window\|fn build_sync_hann\b" src/ tests/ examples/
```

Expected: **nothing** (after T4 plus the previous tasks). If `fn build_sync_hann` still appears with the body that's just `crate::dsp::build_hann(SYNC_FFT_WINDOW_SAMPLES)`, that's OK (the body changed but the fn name remained per T1 Step 5).

- [ ] **Step 8: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

The `cargo test --release` step is the load-bearing verification: `tests/roundtrip.rs` decodes one image per supported mode (PD120/180/240, R24/36/72, Scottie 1/2/DX, Martin 1/2), exercising every reshaped call site. Any mistake in the body-copy of `decode_one_channel_into`, in the call-site struct constructions, or in the param-rename surfaces as a roundtrip pixel-diff regression. `tests/unknown_vis.rs`, `tests/multi_image.rs`, `tests/no_vis.rs` also exercise the decoder path.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor(demod): move decode_one_channel_into + drop dead chan_bounds_abs + ChannelDecodeCtx/DemodState (#85 B3/B5/B16/C20)"
```

---

## Task 5: CHANGELOG, module-level docs polish, full gate

The wrap-up. Verify nothing was missed; ship docs + changelog.

**Files:**
- Modify: `CHANGELOG.md`, `src/dsp.rs` (doc polish if needed), `src/demod.rs` (doc polish if needed)

- [ ] **Step 1: Add the `CHANGELOG.md` `[Unreleased]` entry**

In `CHANGELOG.md`, under the `## [Unreleased]` header, add (or merge into an existing `### Internal` subsection):

```markdown
### Internal

- **Extracted `crate::demod` and `crate::dsp` from `mode_pd.rs` / `snr.rs` /
  `vis.rs`.** `crate::demod` now owns the per-channel demod machinery
  (`ChannelDemod` — renamed from `PdDemod`, `decode_one_channel_into`,
  `pixel_freq`, `freq_to_luminance`, `ycbcr_to_rgb` [`#[doc(hidden)]`],
  `FFT_LEN`, `SNR_REESTIMATE_STRIDE`, `HannBank`, `HANN_LENS`,
  `window_idx_for_snr{_with_hysteresis}`); `crate::dsp` consolidates the
  generic Hann-window builder (4 prior copies), `power(Complex<f32>)→f64`,
  `get_bin`, and `goertzel_power`. `decode_one_channel_into`'s 11-arg
  signature collapses to 5 via new `ChannelDecodeCtx`/`DemodState` structs;
  the dead `chan_bounds_abs` parameter (and the three caller
  `array::from_fn` blocks that fed it) is gone; `time_offset_seconds`
  renamed to `radio_frame_offset_seconds`. Pure refactor: identical
  behavior, no public-API change. (#85; audit B1/B3/B5/B8/B16/C6/C20.)
```

- [ ] **Step 2: Verify the module-level docs read well**

Re-read `src/dsp.rs` and `src/demod.rs` module-level docs (the `//!` blocks at the top of each file). Confirm:
- `dsp.rs` opens with one paragraph describing what it owns (Hann/power/get_bin/Goertzel) and notes nothing here is SSTV-specific.
- `demod.rs` opens with one paragraph describing what it owns (per-channel demod) and notes that `HannBank`/`HANN_LENS`/`window_idx_for_snr*` moved here from `crate::snr` because they're per-pixel-demod machinery, not SNR-estimation logic. Notes that `SnrEstimator` itself stays in `crate::snr` with its own 1024-sample SNR-analysis Hann.

If either doc is missing, sparse, or stale (e.g. mentions `todo!()`-placeholder text from earlier in the refactor), revise.

- [ ] **Step 3: Final sanity grep (post-T4 cleanup verification)**

Run:
```bash
grep -rn "PdDemod\|mode_pd::ycbcr_to_rgb\|mode_pd::FFT_LEN\|mode_pd::decode_one_channel_into\|mode_pd::pixel_freq\|mode_pd::freq_to_luminance\|crate::snr::HannBank\|crate::snr::HANN_LENS\|crate::snr::window_idx_for_snr\|chan_bounds_abs\|time_offset_seconds\|fn build_hann_window\|fn goertzel_power\b" src/ tests/ examples/
```

Expected: **completely empty output**. Any hit is a missed rename and must be fixed before the PR.

- [ ] **Step 4: Run the full CI gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. If `cargo doc -D warnings` fires on a broken `[crate::mode_pd::...]` intra-doc link anywhere, fix it (the new home is `crate::demod` or `crate::dsp`).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "docs(refactor): module-level docs + CHANGELOG for the demod/dsp extraction (#85)"
```

If Step 2 or Step 3 produced no doc changes (everything was clean from T1–T4), this commit may only contain the CHANGELOG bump — that's fine. If Step 3 found a missed rename, fix it and combine with the CHANGELOG bump in this commit.

---

## Self-review notes (for the implementer / reviewers)

- **Spec coverage:** B1 (extract demod/dsp) → T1 + T2 + T3; B8 (consolidate DSP primitives) → T1; B3 (`ChannelDecodeCtx`/`DemodState` structs) → T4 Step 1; B5 (drop `chan_bounds_abs`) → T4 Steps 2-6; B16 (`time_offset_seconds` rename) → T4; C6 (`ycbcr_to_rgb` visibility — adapted to `#[doc(hidden)]`, see Step 3 of T3 Step 1) → T3; C20 (cast-safety comment block) → T4 Step 2. CHANGELOG → T5 Step 1.

- **No new tests** — pure refactor. Tests follow their code (T1 Step 1 brings `dsp` tests; T2 Step 4 brings `HannBank` / `window_idx_for_snr*` tests to `demod::tests`; T3 Step 2 brings `pixel_freq` / `freq_to_luminance` / `ycbcr_to_rgb` tests to `demod::tests`). All existing integration tests pass byte-for-byte.

- **Why the order matters:** T1 (dsp.rs) is dependency-free. T2 (HannBank move) depends on T1 because `HannBank::new` uses `crate::dsp::build_hann`. T3 (ChannelDemod move) depends on T2 because `ChannelDemod` holds a `HannBank` from `crate::demod`. T4 (decode_one_channel_into refactor) depends on T3 because it uses `ChannelDemod`. T5 (docs + gate) depends on everything.

- **Why the call-site flattening sits in T4 (not earlier):** T4 is the only task that touches `decode_one_channel_into`'s signature. Doing the `chan_bounds_abs` drop / `time_offset_seconds` rename / `ChannelDecodeCtx` introduction in a single atomic step means call sites churn exactly once.

- **`ycbcr_to_rgb` visibility — `#[doc(hidden)]` rather than `pub(crate)`:** the audit (C6) called for `pub(crate)`, but `pub use crate::demod::ycbcr_to_rgb;` inside the `pub mod __test_support` requires the source to be `pub` (rustc rejects re-exporting a `pub(crate)` item at a `pub` path). `#[doc(hidden)]` achieves the same intent — kept off the documented public surface — without breaking the existing `__test_support::mode_pd::ycbcr_to_rgb` re-export path that `tests/roundtrip.rs` uses. The spec elaborates.

- **Out of scope:** `trait ModeDecoder` (B12 → #96); test encoders off the public API (B10 → #86); shared test-tone module (B9 → #86); per-channel allocation hoisting (D3/D5/D6 → #93); `process` decomposition (B14 → #96). Each is tracked under epic #97.
