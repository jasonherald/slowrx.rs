# Issue #86 — Shared test-tone module + encoders off public API — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract `crate::test_tone` for the continuous-phase FM tone generator (B9 — consolidates four pre-#86 copies across `pd_test_encoder` / `robot_test_encoder` / `scottie_test_encoder` / `vis::tests`). Tighten the test-encoder modules to `pub(crate) mod` and switch `__test_support` to thin wrapper fns so `slowrx::pd_test_encoder::*` (etc.) is no longer reachable externally (B10). Plus E2 (scottie double-doc fix), F11 (encoder smoke tests), C17 partial (`unreachable!`/`expect!` cleanup).

**Architecture:** Bottom-up migration. T1 creates `src/test_tone.rs` and migrates every consumer (3 encoders + `vis::tests`) atomically — no duplicate-then-delete intermediate state. T2 flips `pub mod *_test_encoder` → `pub(crate) mod` and replaces the `__test_support` `pub use` re-exports with thin `pub fn` wrappers. T3 cleans up the scottie double-doc and the `unreachable!`/`expect` redundancies. T4 adds encoder smoke tests. T5 lands CHANGELOG + final gate. After each task the crate compiles and the full release test suite passes.

**Tech Stack:** Rust 2021, MSRV 1.85. Clippy config: `clippy::all`/`pedantic` = warn, `unwrap_used`/`panic`/`expect_used` = warn. CI gate: `cargo test --all-features --locked --release`, `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`. No GPG signing.

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-14-issue-86-shared-test-tone-module-design.md`
- Audit: `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs B9, B10, E2, F11, C17 partial)

---

## File Structure

| File | Status | Role |
|------|--------|------|
| `src/test_tone.rs` | **new** (T1) | Generic continuous-phase FM tone generator: `SYNC_HZ` / `PORCH_HZ` / `SEPTR_HZ` / `BLACK_HZ` / `WHITE_HZ` consts, `lum_to_freq`, `ToneWriter` struct (with `new`/`with_pre_silence_samples`/`fill_to`/`fill_secs`/`len`/`into_vec`). Gated `cfg(any(test, feature = "test-support"))`. |
| `src/pd_test_encoder.rs` | modify | Drop local consts + `lum_to_freq` + `fill_to`; import from `test_tone`. Use `ToneWriter`. `pub fn` → `pub(crate) fn` (T2). |
| `src/robot_test_encoder.rs` | modify | Same as PD. |
| `src/scottie_test_encoder.rs` | modify | Same as PD. Plus E2 (delete stale doc paragraph), `pub fn` → `pub(crate) fn` (T2). |
| `src/vis.rs` | modify (tests only) | `synth_vis_with_offset` + `r12bw_rejects_standard_parity` use `ToneWriter::fill_secs`. |
| `src/lib.rs` | modify | Add `pub(crate) mod test_tone;` (T1). Flip the three `pub mod *_test_encoder;` to `pub(crate) mod` (T2). Switch `__test_support::mode_{pd,robot,scottie}` `pub use` re-exports to thin `pub fn` wrappers (T2). |
| `CHANGELOG.md` | modify | `[Unreleased]` `### Internal` bullet (T5). |

Task order: **T1** → **T2** → **T3** → **T4** → **T5**. Each task leaves a working state (crate compiles, full release suite passes).

**Verification after each task** (the rule for this PR):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

---

## Task 1: Create `src/test_tone.rs` and migrate all 4 consumers atomically

Create the new module populated and switch every consumer in one task so there's no duplicate-then-delete intermediate state.

**Files:**
- Create: `src/test_tone.rs`
- Modify: `src/lib.rs`, `src/pd_test_encoder.rs`, `src/robot_test_encoder.rs`, `src/scottie_test_encoder.rs`, `src/vis.rs`

- [ ] **Step 1: Create `src/test_tone.rs`**

```rust
//! Continuous-phase FM tone generator for the synthetic test encoders.
//!
//! Consolidates the four pre-#86 copies in `pd_test_encoder.rs`,
//! `robot_test_encoder.rs`, `scottie_test_encoder.rs`, and
//! `vis.rs::tests`. The SSTV-specific frequency constants (`SYNC_HZ` =
//! 1200 Hz, the 1500 Hz `PORCH_HZ` / `SEPTR_HZ` / `BLACK_HZ`, the 2300 Hz
//! `WHITE_HZ`) live here too, as does the `lum_to_freq(lum) → Hz`
//! mapping.
//!
//! [`ToneWriter`] owns the output `Vec<f32>` and a running `phase`
//! accumulator. Two emission forms share the same `phase`:
//! [`ToneWriter::fill_to`] (cumulative absolute sample target — used by
//! the encoders; prevents per-tone rounding drift across a multi-channel
//! line) and [`ToneWriter::fill_secs`] (per-tone wall-clock duration —
//! used by VIS bursts where each tone has a fixed duration).
//!
//! Gated under `cfg(any(test, feature = "test-support"))` — not part of
//! the published API.

use std::f64::consts::PI;

use crate::resample::WORKING_SAMPLE_RATE_HZ;

pub(crate) const SYNC_HZ: f64 = 1200.0;
pub(crate) const PORCH_HZ: f64 = 1500.0;
/// Same value as `PORCH_HZ` / `BLACK_HZ` (1500 Hz), named for SSTV-spec clarity.
pub(crate) const SEPTR_HZ: f64 = 1500.0;
pub(crate) const BLACK_HZ: f64 = 1500.0;
pub(crate) const WHITE_HZ: f64 = 2300.0;

/// Map an 8-bit luminance value to its FM frequency in Hz.
/// Linear interpolation between [`BLACK_HZ`] (lum=0) and [`WHITE_HZ`] (lum=255).
#[must_use]
pub(crate) fn lum_to_freq(lum: u8) -> f64 {
    BLACK_HZ + (WHITE_HZ - BLACK_HZ) * f64::from(lum) / 255.0
}

/// Continuous-phase FM tone writer. Owns the output `Vec<f32>` and a
/// running `phase` accumulator so consecutive tones produce no audible
/// discontinuity at boundaries.
pub(crate) struct ToneWriter {
    out: Vec<f32>,
    phase: f64,
}

impl ToneWriter {
    pub fn new() -> Self {
        Self {
            out: Vec::new(),
            phase: 0.0,
        }
    }

    /// Construct with `n` zero samples already in `out` (encoder pre-silence
    /// or VIS pre-silence). Phase starts at 0.
    pub fn with_pre_silence_samples(n: usize) -> Self {
        Self {
            out: vec![0.0; n],
            phase: 0.0,
        }
    }

    /// Emit samples up to absolute output index `target_n` (exclusive) at
    /// `freq_hz`. Cumulative-target form — call repeatedly with increasing
    /// `target_n` across a multi-channel line; per-pixel rounding error
    /// never compounds.
    #[allow(clippy::cast_precision_loss)]
    pub fn fill_to(&mut self, freq_hz: f64, target_n: usize) {
        let dphi = 2.0 * PI * freq_hz / f64::from(WORKING_SAMPLE_RATE_HZ);
        while self.out.len() < target_n {
            self.out.push(self.phase.sin() as f32);
            self.phase += dphi;
            if self.phase > 2.0 * PI {
                self.phase -= 2.0 * PI;
            }
        }
    }

    /// Emit `secs` seconds at `freq_hz`. Per-tone-duration form. Used by
    /// VIS bursts where each tone has a fixed wall-clock duration.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn fill_secs(&mut self, freq_hz: f64, secs: f64) {
        let n = (secs * f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize;
        let target = self.out.len() + n;
        self.fill_to(freq_hz, target);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.out.len()
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<f32> {
        self.out
    }
}

impl Default for ToneWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::cast_precision_loss)]
mod tests {
    use super::*;

    #[test]
    fn lum_to_freq_endpoints_match_black_and_white() {
        assert_eq!(lum_to_freq(0), BLACK_HZ);
        assert_eq!(lum_to_freq(255), WHITE_HZ);
        // Midpoint (lum=128) is just under (BLACK+WHITE)/2.
        let mid = lum_to_freq(128);
        let target = (BLACK_HZ + WHITE_HZ) / 2.0;
        assert!((mid - target).abs() < 5.0, "mid={mid} ≉ {target}");
    }

    #[test]
    fn fill_to_advances_to_exact_target() {
        let mut tone = ToneWriter::new();
        tone.fill_to(1200.0, 100);
        assert_eq!(tone.len(), 100);
        tone.fill_to(1500.0, 250);
        assert_eq!(tone.len(), 250);
    }

    #[test]
    fn fill_to_and_fill_secs_are_equivalent_for_matching_durations() {
        // Pick a duration that gives an integer sample count (avoids `.round()` ambiguity).
        let secs = 100.0 / f64::from(WORKING_SAMPLE_RATE_HZ); // 100 samples
        let mut a = ToneWriter::new();
        a.fill_secs(1200.0, secs);
        let mut b = ToneWriter::new();
        b.fill_to(1200.0, 100);
        let av = a.into_vec();
        let bv = b.into_vec();
        assert_eq!(av.len(), bv.len());
        for (i, (&x, &y)) in av.iter().zip(bv.iter()).enumerate() {
            assert!((x - y).abs() < 1e-6, "sample {i}: {x} vs {y}");
        }
    }

    #[test]
    fn phase_is_continuous_across_tone_boundaries() {
        // Fill 100 samples at 1200 Hz then 100 more at 1500 Hz. The
        // sample-to-sample step at the boundary should be roughly the
        // dphi of the SECOND tone, not a huge phase jump.
        let mut tone = ToneWriter::new();
        tone.fill_to(1200.0, 100);
        tone.fill_to(1500.0, 200);
        let v = tone.into_vec();
        // Maximum sample-to-sample absolute delta across the whole signal:
        // for a continuous-phase signal with the highest tone freq we use
        // here (1500 Hz at 11025 Hz SR), dphi ≈ 0.855 rad/sample → max
        // |Δ sin(phase)| ≤ |dphi| ≈ 0.855. Add a margin → < 1.0.
        for w in v.windows(2) {
            let delta = (w[1] - w[0]).abs();
            assert!(delta < 1.0, "sample-to-sample delta {delta} > 1.0 — phase discontinuity");
        }
    }
}
```

- [ ] **Step 2: Wire `test_tone` into `src/lib.rs`**

Open `src/lib.rs`. The module-declaration block (lines ~40-66) currently has the three `pub mod *_test_encoder;` declarations gated on `cfg(any(test, feature = "test-support"))`. Add immediately above the first of them (above `pub mod pd_test_encoder;`):

```rust
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_tone;
```

(Same gating as the encoders. `pub(crate)` — never visible externally; `#[doc(hidden)]` is unnecessary on `pub(crate)`.)

- [ ] **Step 3: Migrate `src/pd_test_encoder.rs` to `ToneWriter`**

Open `src/pd_test_encoder.rs`. Apply these edits:

(a) **At the top of the file**, add (just below the existing `use` block, before `const SYNC_HZ`):

```rust
use crate::test_tone::{lum_to_freq, ToneWriter, PORCH_HZ, SYNC_HZ};
```

(PD doesn't use `SEPTR_HZ` in the body — septr_seconds is 0 for PD modes — but it does use `BLACK_HZ` / `WHITE_HZ` indirectly through `lum_to_freq`. Importing `lum_to_freq` suffices; the encoder doesn't reference `BLACK_HZ` / `WHITE_HZ` by name.)

(b) **Delete** the local definitions (currently around lines 19-43):
- `const SYNC_HZ: f64 = 1200.0;`
- `const PORCH_HZ: f64 = 1500.0;`
- `const BLACK_HZ: f64 = 1500.0;`
- `const WHITE_HZ: f64 = 2300.0;`
- `fn lum_to_freq(lum: u8) -> f64 { ... }`
- `fn fill_to(out: &mut Vec<f32>, freq_hz: f64, target_n: usize, phase: &mut f64) { ... }` (with its preceding doc comment)

(c) In `encode_pd`, the body currently allocates an output Vec and a phase scalar, then calls `fill_to(&mut out, freq, target_n, &mut phase)` many times, returning `out` at the end. Apply this textual substitution throughout the fn:

- `let mut out: Vec<f32> = Vec::new();` → `let mut tone = ToneWriter::new();`
  (or `vec![0.0; n]` → `ToneWriter::with_pre_silence_samples(n)` if the encoder pre-pads with silence.)
- `let mut phase = 0.0_f64;` → delete (it lives on `ToneWriter` now).
- Each `fill_to(&mut out, freq, target_n, &mut phase);` → `tone.fill_to(freq, target_n);`.
- Final `out` (the return expression) → `tone.into_vec()`.

If the function does any direct writes to `out` like `out.push(...)` or `out.extend(...)` outside `fill_to` (it shouldn't — PD encoder only emits via `fill_to`), STOP and report so we can extend `ToneWriter` instead of bypassing it.

- [ ] **Step 4: Migrate `src/robot_test_encoder.rs` to `ToneWriter`**

Same shape as Step 3. Open `src/robot_test_encoder.rs`:

(a) Add at top (below existing `use` block):

```rust
use crate::test_tone::{lum_to_freq, ToneWriter, PORCH_HZ, SEPTR_HZ, SYNC_HZ};
```

(Robot72 uses septr between Y/U/V; Robot36/24 uses septr between Y and chroma. So `SEPTR_HZ` is needed here even though robot_test_encoder didn't have a local `SEPTR_HZ` constant — it likely used `PORCH_HZ` as a stand-in for the septr tone since they share value. Verify when editing: if the code emits a septr tone at `PORCH_HZ`, change to `SEPTR_HZ` for spec-clarity. If it never emits a septr tone, drop `SEPTR_HZ` from the import.)

(b) Delete the local definitions (currently lines 27-48):
- `const SYNC_HZ: f64 = 1200.0;`
- `const PORCH_HZ: f64 = 1500.0;`
- `const BLACK_HZ: f64 = 1500.0;`
- `const WHITE_HZ: f64 = 2300.0;`
- `fn lum_to_freq(...)`.
- `fn fill_to(...)`.

(c) In `encode_robot`, apply the same `out` → `tone` substitution as Step 3(c).

- [ ] **Step 5: Migrate `src/scottie_test_encoder.rs` to `ToneWriter`**

Same shape. Open `src/scottie_test_encoder.rs`:

(a) Add at top:

```rust
use crate::test_tone::{lum_to_freq, ToneWriter, PORCH_HZ, SEPTR_HZ, SYNC_HZ};
```

(b) Delete the local definitions (currently lines 35-65):
- `const SYNC_HZ`, `PORCH_HZ`, `SEPTR_HZ`, `BLACK_HZ`, `WHITE_HZ`.
- `fn lum_to_freq(...)`.
- `fn fill_to(...)`.

(c) In `encode_scottie`, apply the same `out` → `tone` substitution.

- [ ] **Step 6: Migrate `src/vis.rs::tests` to `ToneWriter`**

Open `src/vis.rs`. Two sites use a per-tone phase-advance closure: `synth_vis_with_offset` (around lines 462-501) and `r12bw_rejects_standard_parity` (around lines 638-670).

(a) **`synth_vis_with_offset`** — the current body looks like:

```rust
let mut out: Vec<f32> = vec![0.0; (pre_silence_secs * sr).round() as usize];
let mut phase = 0.0_f64;
let mut emit = |freq: f64, secs: f64, out: &mut Vec<f32>| {
    let dphi = 2.0 * PI * freq / sr;
    for _ in 0..(secs * sr).round() as usize {
        out.push(phase.sin() as f32);
        phase += dphi;
        if phase > 2.0 * PI {
            phase -= 2.0 * PI;
        }
    }
};
// ... VIS frequency math (leader, break_f, bit_freq) ...
emit(leader, 0.300, &mut out);
emit(break_f, 0.030, &mut out);
// ... 7 bit emits + parity + trailing break ...
out
```

Replace with:

```rust
let mut tone = crate::test_tone::ToneWriter::with_pre_silence_samples(
    (pre_silence_secs * f64::from(WORKING_SAMPLE_RATE_HZ)).round() as usize,
);
// ... VIS frequency math (leader, break_f, bit_freq) stays here ...
tone.fill_secs(leader, 0.300);
tone.fill_secs(break_f, 0.030);
// ... 7 bit fill_secs + parity + trailing break ...
tone.into_vec()
```

The VIS frequency math (the `leader = LEADER_HZ + freq_offset_hz` line, `break_f = leader + BREAK_HZ_OFFSET`, the `bit_freq` closure) STAYS — those are VIS-classifier offsets, not encoder consts.

If the body has `let sr = f64::from(WORKING_SAMPLE_RATE_HZ);` as a local binding used only by the `emit` closure: delete it (no longer needed once `emit` is gone, since `tone.fill_secs` reads `WORKING_SAMPLE_RATE_HZ` internally). If it's used elsewhere in the function (e.g. the `pre_silence_secs * sr` line), keep it or inline `f64::from(WORKING_SAMPLE_RATE_HZ)`.

(b) **`r12bw_rejects_standard_parity`** — same closure pattern (with its own inline `let mut out = ...; let mut emit = |...| {...};`). Apply the same substitution: build a `tone = ToneWriter::with_pre_silence_samples(0)` (or `ToneWriter::new()` if there's no pre-silence in this test), then `tone.fill_secs(freq, secs)` instead of `emit(freq, secs, &mut out)`, and `tone.into_vec()` at the end.

**Important:** the per-tone wall-clock durations and VIS-classifier frequency math stay identical — this is a pure mechanism swap, not a behavior change.

- [ ] **Step 7: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. `cargo test --release` is load-bearing — `tests/roundtrip.rs` exercises all three encoders end-to-end on every supported mode; any drift in tone math, phase continuity, or channel ordering surfaces as a roundtrip pixel-diff regression. `vis::tests`'s VIS-detection tests exercise `synth_vis_with_offset` and `r12bw_rejects_standard_parity`.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor(test_tone): extract continuous-phase FM tone generator + ToneWriter (#86 B9)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Flip `pub mod *_test_encoder` to `pub(crate) mod` and switch `__test_support` to wrapper fns

Kill the duplicate stable path. The three `slowrx::pd_test_encoder::*` / `robot_test_encoder::*` / `scottie_test_encoder::*` external paths disappear; `__test_support` becomes the sole consumer-facing entry point.

**Files:**
- Modify: `src/lib.rs`, `src/pd_test_encoder.rs`, `src/robot_test_encoder.rs`, `src/scottie_test_encoder.rs`

- [ ] **Step 1: Flip the three module declarations in `src/lib.rs`**

The three declarations currently read:

```rust
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod pd_test_encoder;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod robot_test_encoder;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod scottie_test_encoder;
```

Replace each with:

```rust
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod pd_test_encoder;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod robot_test_encoder;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod scottie_test_encoder;
```

`#[doc(hidden)]` is no longer needed — `pub(crate)` items don't appear in `cargo doc` at all.

- [ ] **Step 2: Tighten the encoder fn visibilities**

In each encoder source file (`src/pd_test_encoder.rs`, `src/robot_test_encoder.rs`, `src/scottie_test_encoder.rs`):

(a) Change `pub fn encode_<mode>(...)` to `pub(crate) fn encode_<mode>(...)`. The outer module is `pub(crate)` so `pub` is unnecessary.

(b) Drop the `#[doc(hidden)]` attribute on each `encode_*` fn (if present) — redundant with `pub(crate)`. Keep all other attributes (`#[must_use]`, any `#[allow(...)]`).

Run after this step: `grep -n "pub fn encode_pd\|pub fn encode_robot\|pub fn encode_scottie\|#\[doc(hidden)\]" src/pd_test_encoder.rs src/robot_test_encoder.rs src/scottie_test_encoder.rs` — should print nothing for the `pub fn` patterns (all should be `pub(crate) fn` now), and `#[doc(hidden)]` should only appear if attached to *internal* items inside the modules (not the `encode_*` fns themselves).

- [ ] **Step 3: Rewrite `__test_support::mode_pd::encode_pd` as a wrapper fn**

In `src/lib.rs`, find the `pub mod __test_support` block. The `mode_pd` sub-module currently is:

```rust
pub mod mode_pd {
    pub use crate::demod::ycbcr_to_rgb;
    pub use crate::pd_test_encoder::encode_pd;
}
```

Replace with:

```rust
pub mod mode_pd {
    pub use crate::demod::ycbcr_to_rgb;

    /// Thin wrapper around the now-`pub(crate)` `crate::pd_test_encoder::encode_pd`.
    /// `__test_support` is the sole consumer-facing path for the synthetic
    /// PD encoder (#86 B10).
    #[doc(hidden)]
    #[must_use]
    pub fn encode_pd(mode: crate::modespec::SstvMode, ycrcb: &[[u8; 3]]) -> Vec<f32> {
        crate::pd_test_encoder::encode_pd(mode, ycrcb)
    }
}
```

**Verify the wrapper signature matches the underlying fn exactly** — read the current `pub(crate) fn encode_pd(...) -> Vec<f32>` signature from `src/pd_test_encoder.rs` (after T1+Step 2 above) and ensure the wrapper's parameter types match (in particular: `&[[u8; 3]]` is the slice form; if the underlying takes something else, mirror it).

- [ ] **Step 4: Rewrite `__test_support::mode_robot::encode_robot` as a wrapper fn**

In `src/lib.rs`, the `mode_robot` sub-module:

```rust
pub mod mode_robot {
    pub use crate::robot_test_encoder::encode_robot;
}
```

Replace with:

```rust
pub mod mode_robot {
    /// Thin wrapper around the now-`pub(crate)` `crate::robot_test_encoder::encode_robot`.
    /// `__test_support` is the sole consumer-facing path for the synthetic
    /// Robot encoder (#86 B10).
    #[doc(hidden)]
    #[must_use]
    pub fn encode_robot(mode: crate::modespec::SstvMode, ycrcb: &[[u8; 3]]) -> Vec<f32> {
        crate::robot_test_encoder::encode_robot(mode, ycrcb)
    }
}
```

(Again, verify the wrapper signature matches the underlying `encode_robot` exactly.)

- [ ] **Step 5: Rewrite `__test_support::mode_scottie::encode_scottie` as a wrapper fn**

In `src/lib.rs`, the `mode_scottie` sub-module:

```rust
pub mod mode_scottie {
    pub use crate::scottie_test_encoder::encode_scottie;
}
```

Replace with:

```rust
pub mod mode_scottie {
    /// Thin wrapper around the now-`pub(crate)` `crate::scottie_test_encoder::encode_scottie`.
    /// `__test_support` is the sole consumer-facing path for the synthetic
    /// Scottie/Martin encoder (#86 B10).
    #[doc(hidden)]
    #[must_use]
    pub fn encode_scottie(mode: crate::modespec::SstvMode, rgb: &[[u8; 3]]) -> Vec<f32> {
        crate::scottie_test_encoder::encode_scottie(mode, rgb)
    }
}
```

(Verify signature — note Scottie's param name is `rgb` rather than `ycrcb`. Match the underlying fn.)

- [ ] **Step 6: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

`cargo test --release` confirms `tests/roundtrip.rs` etc. still resolve `slowrx::__test_support::mode_*::encode_*` correctly — those consumer paths are stable through the wrappers.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(test-encoders): pub(crate) modules + __test_support wrapper fns (#86 B10)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: E2 (scottie double-doc) + C17 partial (`unreachable!`/`expect!` cleanup)

**Files:**
- Modify: `src/scottie_test_encoder.rs` (E2 + possibly C17), `src/pd_test_encoder.rs` (C17 if present), `src/robot_test_encoder.rs` (C17 if present)

- [ ] **Step 1: Fix the scottie double-doc-comment (E2)**

Open `src/scottie_test_encoder.rs`. The `encode_scottie` fn currently has TWO `///` doc-comment paragraphs straddling `#[must_use]`:

```rust
/// (paragraph 1 — stale Scottie-only: starts with "Encode an image as Scottie 1 / 2 / DX audio.
/// `rgb` is row-major..." and ends with "Per radio line, emits in this order: 1. Septr 1 ...")
#[must_use]
/// (paragraph 2 — accurate Scottie-OR-Martin: starts with "Encode an RGB image as continuous-phase
/// FM audio for either Scottie (S1/S2/DX) or Martin (M1/M2)..." and ends with "Panics if `mode` is
/// not one of the five supported variants or if `rgb.len() != line_pixels * image_lines`.")
#[doc(hidden)]
#[allow(dead_code, clippy::too_many_lines)]
pub(crate) fn encode_scottie(mode: SstvMode, rgb: &[[u8; 3]]) -> Vec<f32> {
```

(After T2 the fn is `pub(crate) fn` and `#[doc(hidden)]` may already be gone from Step 2(b). Verify.)

Replace with:

```rust
/// (paragraph 2 verbatim — the accurate Scottie-OR-Martin doc.)
#[must_use]
#[allow(dead_code, clippy::too_many_lines)]
pub(crate) fn encode_scottie(mode: SstvMode, rgb: &[[u8; 3]]) -> Vec<f32> {
```

Concretely: **delete** paragraph 1 in full (every `///` line from the start of the doc block down to and including the "Per radio line, emits in this order" enumeration); **keep** paragraph 2 verbatim; **move** `#[must_use]` to sit with the other attributes (after the doc comment, before `pub(crate) fn`).

- [ ] **Step 2: C17 — audit each encoder for redundant `assert!(matches!(...))` + later `unreachable!()`/`.expect(...)`**

Run: `grep -nE "assert!\(matches!|unreachable!\(\)|\.expect\(\".*mode" src/pd_test_encoder.rs src/robot_test_encoder.rs src/scottie_test_encoder.rs`

For each hit, look at the surrounding code. Each encoder typically opens with something like:

```rust
assert!(matches!(mode, SstvMode::Pd120 | SstvMode::Pd180 | SstvMode::Pd240),
    "encode_pd: unsupported mode {mode:?}");
```

…and later has a `match mode { ... _ => unreachable!() }` or similar on the same variant set. Two redundant guards.

**Resolution per site:**

- If the `match` doesn't use a wildcard arm and uses only the variants the `assert!` covers, the `assert!` is genuinely redundant — **delete the leading `assert!(matches!(...))`** and let the `match`'s exhaustiveness (over the constrained variants) do the work. Caveat: `SstvMode` is `#[non_exhaustive]`, so an explicit `match` on just `Pd120 | Pd180 | Pd240` requires a wildcard, which lands back in `unreachable!()` territory. In that case…
- …**delete the leading `assert!(matches!(...))`** and **keep the `unreachable!()` arm** with a one-line comment explaining: `// unreachable! — guarded by the modespec dispatch in SstvDecoder; SstvMode is #[non_exhaustive] so the wildcard is structurally required.` (The `unreachable!()` is a runtime guard for the non-exhaustive enum; the leading `assert!` was a redundant doubling.)
- If a site has only an `assert!(matches!(...))` with no later `unreachable!()` / `.expect(...)`: **leave it** (the `assert!` IS the only guard — not redundant).
- If a site has only `unreachable!()` / `.expect(...)` with no leading `assert!(matches!(...))`: **leave it** (likewise — not redundant).

Don't add `#[allow]`s. Don't add `panic!()` calls. The goal is to remove one guard per double-guarded site, not to add or restructure.

- [ ] **Step 3: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. The scottie doc fix is a behavior-preserving cleanup; the C17 cleanup deletes redundant guards without changing the surviving guard's behavior, so `tests/roundtrip.rs` is unaffected.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor(test-encoders): scottie double-doc fix + drop redundant assert!/unreachable! pairs (#86 E2/C17)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: F11 — encoder smoke tests

Add `encode_pd` and `encode_robot` unit tests that catch channel-order / septr-placement regressions with a pointed assertion (vs the fuzzy `mean ≥ 5.0` from `tests/roundtrip.rs`). Update existing `scottie_test_encoder::tests` to use the new `crate::test_tone::*` const paths.

**Files:**
- Modify: `src/pd_test_encoder.rs`, `src/robot_test_encoder.rs`, `src/scottie_test_encoder.rs`

- [ ] **Step 1: Add `pd_test_encoder::tests`**

In `src/pd_test_encoder.rs`, append at the end of the file:

```rust
#[cfg(test)]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]
mod tests {
    use super::*;
    use crate::modespec::{for_mode, SstvMode};

    /// A regression in channel order surfaces here as a pointed failure
    /// instead of a fuzzy roundtrip pixel-diff.
    #[test]
    fn encode_pd120_first_tone_is_sync_hz() {
        let spec = for_mode(SstvMode::Pd120);
        let img = vec![[128_u8, 128, 128]; (spec.line_pixels * spec.image_lines) as usize];
        let audio = encode_pd(SstvMode::Pd120, &img);
        let sync_samples =
            (spec.sync_seconds * f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ)) as usize;
        assert!(audio.len() >= sync_samples, "audio too short");
        let p_sync = crate::dsp::goertzel_power(&audio[..sync_samples], crate::test_tone::SYNC_HZ);
        let p_porch = crate::dsp::goertzel_power(&audio[..sync_samples], crate::test_tone::PORCH_HZ);
        assert!(
            p_sync > 10.0 * p_porch,
            "PD line starts with SYNC tone (p_sync={p_sync}, p_porch={p_porch})"
        );
    }

    /// Catches structural drift — extra/missing septr, wrong channel count —
    /// without round-tripping.
    #[test]
    fn encode_pd120_length_matches_radio_frames() {
        let spec = for_mode(SstvMode::Pd120);
        let img = vec![[0_u8; 3]; (spec.line_pixels * spec.image_lines) as usize];
        let audio = encode_pd(SstvMode::Pd120, &img);
        // PD packs 2 image rows / radio frame.
        let radio_frames = f64::from(spec.image_lines) / 2.0;
        let expected = (radio_frames
            * spec.line_seconds
            * f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ)) as usize;
        let diff = (audio.len() as i64 - expected as i64).abs();
        assert!(
            diff < 64,
            "PD120 audio len {} ≉ {expected} (diff {})",
            audio.len(),
            diff
        );
    }
}
```

- [ ] **Step 2: Add `robot_test_encoder::tests`**

In `src/robot_test_encoder.rs`, append at the end of the file:

```rust
#[cfg(test)]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]
mod tests {
    use super::*;
    use crate::modespec::{for_mode, SstvMode};

    /// A regression in Robot channel order surfaces here as a pointed
    /// failure instead of a fuzzy roundtrip pixel-diff. Tests Robot72
    /// (simplest of the three Robot variants — 3 channels, no chroma
    /// alternation).
    #[test]
    fn encode_robot72_first_tone_is_sync_hz() {
        let spec = for_mode(SstvMode::Robot72);
        let img = vec![[128_u8, 128, 128]; (spec.line_pixels * spec.image_lines) as usize];
        let audio = encode_robot(SstvMode::Robot72, &img);
        let sync_samples =
            (spec.sync_seconds * f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ)) as usize;
        assert!(audio.len() >= sync_samples, "audio too short");
        let p_sync = crate::dsp::goertzel_power(&audio[..sync_samples], crate::test_tone::SYNC_HZ);
        let p_porch = crate::dsp::goertzel_power(&audio[..sync_samples], crate::test_tone::PORCH_HZ);
        assert!(
            p_sync > 10.0 * p_porch,
            "Robot72 line starts with SYNC tone (p_sync={p_sync}, p_porch={p_porch})"
        );
    }

    #[test]
    fn encode_robot72_length_matches_radio_lines() {
        let spec = for_mode(SstvMode::Robot72);
        let img = vec![[0_u8; 3]; (spec.line_pixels * spec.image_lines) as usize];
        let audio = encode_robot(SstvMode::Robot72, &img);
        // Robot: 1 image row per radio line.
        let radio_lines = f64::from(spec.image_lines);
        let expected = (radio_lines
            * spec.line_seconds
            * f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ)) as usize;
        let diff = (audio.len() as i64 - expected as i64).abs();
        assert!(
            diff < 64,
            "Robot72 audio len {} ≉ {expected} (diff {})",
            audio.len(),
            diff
        );
    }
}
```

- [ ] **Step 3: Update existing `scottie_test_encoder::tests` to use `crate::test_tone::*`**

In `src/scottie_test_encoder.rs`'s `#[cfg(test)] mod tests` (the existing 2 tests): if any of them references `SYNC_HZ` / `PORCH_HZ` / `SEPTR_HZ` / `BLACK_HZ` / `WHITE_HZ` / `lum_to_freq` — those names no longer resolve locally (T1 deleted them). Change references to fully-qualified `crate::test_tone::SYNC_HZ` etc., or add a `use crate::test_tone::*;` line at the top of the test module.

(After T1 / T2 cleanly the existing tests probably compile already — they may not reference those names. Verify with `grep -nE "SYNC_HZ|PORCH_HZ|SEPTR_HZ|BLACK_HZ|WHITE_HZ|lum_to_freq" src/scottie_test_encoder.rs`. Any hits inside `mod tests` need the import; hits outside `mod tests` should have already been fixed by T1.)

- [ ] **Step 4: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

The new F11 tests should be in the `cargo test --release` output's lib test count (was 116 + 4 from `test_tone::tests` in T1 = 120 → now 124 after the 4 new F11 tests).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(encoders): F11 — pointed smoke tests for encode_pd and encode_robot (#86)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: CHANGELOG + sanity grep + final gate

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add the `CHANGELOG.md` `[Unreleased]` entry**

In `CHANGELOG.md`, under the `## [Unreleased]` header, add (or merge into an existing `### Internal` subsection — `[Unreleased]` is currently empty after #85's merge into 0.5.x or the latest release; either way, add as `### Internal`):

```markdown
### Internal

- **Extracted `crate::test_tone`** — the continuous-phase FM tone generator
  (`SYNC_HZ` / `PORCH_HZ` / `SEPTR_HZ` / `BLACK_HZ` / `WHITE_HZ` consts,
  `lum_to_freq`, the `ToneWriter` struct with cumulative-target `fill_to` and
  per-tone-duration `fill_secs` methods) — shared by `pd_test_encoder` /
  `robot_test_encoder` / `scottie_test_encoder` / `vis::tests` (four prior
  copies; audit B9). Tightened the three test-encoder modules to `pub(crate)
  mod` so `slowrx::pd_test_encoder::*` / `robot_test_encoder::*` /
  `scottie_test_encoder::*` are no longer reachable externally; `__test_support`
  switched from `pub use` re-exports to thin `pub fn` wrappers (the sole
  consumer-facing path for the synthetic encoders — audit B10). Plus the
  scottie double-doc-comment fix (E2), smoke tests for `encode_pd` /
  `encode_robot` (F11), and `unreachable!`/`expect!` cleanup in the test
  encoders (C17 partial). Pure refactor: identical behavior; the
  `slowrx::__test_support::mode_*::encode_*` paths are stable for existing
  consumers (`tests/roundtrip.rs` unchanged). (#86; audit B9/B10/E2/F11/C17.)
```

- [ ] **Step 2: Sanity grep**

Run:

```bash
grep -rn "fn lum_to_freq\|fn fill_to\|const SYNC_HZ\|const PORCH_HZ\|const BLACK_HZ\|const WHITE_HZ\|const SEPTR_HZ\|^pub mod pd_test_encoder\|^pub mod robot_test_encoder\|^pub mod scottie_test_encoder\|pub use crate::pd_test_encoder\|pub use crate::robot_test_encoder\|pub use crate::scottie_test_encoder" src/
```

Expected: hits **only** in `src/test_tone.rs` (the canonical `fn lum_to_freq` definition + the 5 `const *_HZ` definitions). Every other file should have zero hits.

(The `^` anchors on `pub mod *_test_encoder` are deliberate — we want to catch a `pub mod` line at column 0 if one survived; the `pub(crate) mod` declarations are fine and won't match.)

- [ ] **Step 3: Run the full CI gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. The `cargo doc` step verifies that no doc-link references `slowrx::pd_test_encoder::*` etc. (those paths are now `pub(crate)`, so external doc-links to them would warn).

- [ ] **Step 4: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(refactor): CHANGELOG for the test_tone extraction + B10 path tightening (#86)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

(If Step 2's sanity grep found unexpected hits and you fixed them, fold those fixes into this commit.)

---

## Self-review notes (for the implementer / reviewers)

- **Spec coverage:** B9 (extract `test_tone` + `ToneWriter`) → T1; B10 (`pub(crate) mod` + wrapper fns) → T2; E2 (scottie double-doc) → T3 Step 1; C17 partial (`unreachable!`/`expect` cleanup) → T3 Step 2; F11 (encoder smoke tests) → T4; CHANGELOG → T5 Step 1.

- **Why the order matters:** T1 must be atomic (create + migrate) so the crate compiles after each commit — `test_tone.rs` items would be `dead_code` if T1 stopped after Step 2. T2 depends on T1 (the encoder fns must already use `crate::test_tone` consts before they become `pub(crate)`, otherwise the consts are unreachable from `__test_support` wrappers in T2 Step 3). T3 and T4 are independent of each other but both depend on T2 (they touch the encoder files; T2's `pub fn` → `pub(crate) fn` and `#[doc(hidden)]` drops must land first). T5 depends on everything.

- **`pub(crate) → pub` "private item in public interface" risk:** the wrapper fns in T2 Steps 3-5 are `pub fn` (not `pub use`), so they don't re-export a `pub(crate)` item at a `pub` path — they're plain public fns that internally call `pub(crate)` ones. Legal under all rustc visibility rules.

- **F11 SYNC-first thresholds:** `p_sync > 10.0 * p_porch` — at PD's `sync_seconds` (~9 ms ≈ 99 samples at 11025 Hz), Goertzel power at 1200 Hz vs 1500 Hz on a pure 1200 Hz tone gives a ratio of many orders of magnitude. 10× is wildly conservative; any encoder regression that emits PORCH first instead of SYNC inverts the ratio and the test fails clearly.

- **No new tests in T1's `test_tone::tests` exercise full-encoder behavior** — those tests target `ToneWriter` itself (phase continuity, `fill_to`/`fill_secs` equivalence, `lum_to_freq` endpoints). The encoders are exercised end-to-end by `tests/roundtrip.rs` (existing) and the new F11 smoke tests (T4).

- **Out of scope** (deliberate — tracked separately under epic #97):
  - Moving encoders entirely out of `src/` to `tests/common/` (B10 "better" option) — rejected during brainstorming.
  - The remaining audit cleanups not in #86's bundle (#87 resampler, #88 find_sync, #91 ModeSpec, #92 API hygiene, #93 perf, #94 docs, #95 CI, #96 standalone).
  - `vis::tests::synth_vis` / `synth_tone` / `synth_tone_n` — the bare-tone helpers that don't carry phase. Staying in `vis::tests`.
