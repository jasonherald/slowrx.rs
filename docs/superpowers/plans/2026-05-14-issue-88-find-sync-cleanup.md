# Issue #88 — `find_sync` cleanup — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Polish `src/sync.rs::find_sync` end-to-end — decompose the 160-line function into three pure helpers (B2), move the Scottie mid-line skip correction onto `ModeSpec` (B4), name the magic numbers (C2), replace bare-int 2D indexing with local closures (C10), fix one real off-by-one bug (A6), document two slowrx-C-vs-Rust behavioral choices (A7, A8), and add Hough slant-correction (F2) + Scottie sync (F3) tests.

**Architecture:** Eight sequential tasks. Each preserves the green test suite. T1 (constants), T2 (B4 + skip helper), T3 (find_falling_edge + falling_edge_from_x_acc + A6 fix), T4 (hough_detect_slant + closures + drop `#[allow(too_many_lines)]`) decompose `find_sync` step-by-step; existing tests act as the regression net. T5 (deviation docs) is text-only. T6 (F2), T7 (F3) add new coverage that should pass on the post-T4 code. T8 closes with the CHANGELOG entry + a full gate run.

**Tech Stack:** Rust 2021, MSRV 1.85. CI gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features --locked --release`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`. No GPG signing.

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-14-issue-88-find-sync-cleanup-design.md`
- Audit: `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs B2, B4, C2, C10, A6, A7, A8, F2, F3)
- Intentional deviations: `docs/intentional-deviations.md`

---

## File Structure

| File | Status | Role |
|------|--------|------|
| `src/sync.rs` | modify | Most of the change. Add named consts; extract `hough_detect_slant`, `find_falling_edge`, `falling_edge_from_x_acc`, `skip_seconds_for`; add index closures; fix A6; drop `#[allow(too_many_lines)]`; add 3 new tests + 1 new synthesis helper. |
| `src/modespec.rs` | modify | Add `pub(crate) fn skip_correction_seconds(&self) -> f64`. |
| `docs/intentional-deviations.md` | modify | Append 2 new entries (A7 + A8). |
| `CHANGELOG.md` | modify | One bullet under `[Unreleased] ### Internal`. |

Task order: **T1** (C2 named consts) → **T2** (B4 ModeSpec method + B2-partial: extract `skip_seconds_for`) → **T3** (B2-partial: extract `find_falling_edge` + `falling_edge_from_x_acc`; A6 fix tested on the pure helper) → **T4** (B2-final: extract `hough_detect_slant` + C10 closures; drop `#[allow(too_many_lines)]`) → **T5** (A7 + A8 deviation docs) → **T6** (F2 slant tests) → **T7** (F3 Scottie test) → **T8** (CHANGELOG + final gate).

**Verification after each task** (the rule for this PR):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

---

## Task 1: C2 — Named constants

Pure rename, no behavior change. Three magic numbers in `src/sync.rs` get names.

**Files:**
- Modify: `src/sync.rs`

- [ ] **Step 1: Add the three new consts**

Open `src/sync.rs`. Find the existing const block (around lines 56-69 — the `MIN_SLANT_DEG`, `X_ACC_BINS`, etc. block). After the existing `const LINES_D_BINS: usize = 600;` line, add:

```rust
/// Right-edge slip threshold for the falling-edge `xmax`: if `xmax`
/// exceeds half the column-accumulator span, the detected pulse
/// belongs to the next line's leading sync — wrap left by this
/// amount. Matches slowrx `sync.c:117` (`if (xmax > 350) xmax -= 350;`).
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
const X_ACC_SLIP_THRESHOLD: i32 = (X_ACC_BINS / 2) as i32; // 350

/// 8-tap falling-edge detection kernel: leading 4 ones, trailing 4
/// negative ones. Convolved with the column-accumulator `x_acc`;
/// the position of the maximum response is the falling edge of the
/// dominant sync pulse. Matches slowrx `sync.c:108` (the inline
/// literal `{1,1,1,1,-1,-1,-1,-1}`).
const SYNC_EDGE_KERNEL: [i32; 8] = [1, 1, 1, 1, -1, -1, -1, -1];
const SYNC_EDGE_KERNEL_LEN: usize = SYNC_EDGE_KERNEL.len();
```

- [ ] **Step 2: Replace `350` slip-wrap with `X_ACC_SLIP_THRESHOLD`**

In `src/sync.rs`, find the slip-wrap block (currently around lines 340-343):

```rust
    // sync.c:117 — pulse near the right edge slipped from previous left.
    if xmax > 350 {
        xmax -= 350;
    }
```

Replace with:

```rust
    // sync.c:117 — pulse near the right edge slipped from previous left.
    if xmax > X_ACC_SLIP_THRESHOLD {
        xmax -= X_ACC_SLIP_THRESHOLD;
    }
```

- [ ] **Step 3: Replace the local `kernel` array with `SYNC_EDGE_KERNEL`**

In `src/sync.rs`, find the falling-edge loop (currently around lines 325-338):

```rust
    let kernel: [i32; 8] = [1, 1, 1, 1, -1, -1, -1, -1];
    let mut xmax: i32 = 0;
    let mut max_convd: i32 = 0;
    for (x, window) in x_acc.windows(8).enumerate() {
        let convd: i32 = window
            .iter()
            .zip(kernel.iter())
            .map(|(&v, &k)| (v as i32) * k)
            .sum();
        if convd > max_convd {
            max_convd = convd;
            xmax = (x as i32) + 4;
        }
    }
```

Replace with:

```rust
    let mut xmax: i32 = 0;
    let mut max_convd: i32 = 0;
    for (x, window) in x_acc.windows(SYNC_EDGE_KERNEL_LEN).enumerate() {
        let convd: i32 = window
            .iter()
            .zip(SYNC_EDGE_KERNEL.iter())
            .map(|(&v, &k)| (v as i32) * k)
            .sum();
        if convd > max_convd {
            max_convd = convd;
            xmax = (x as i32) + (SYNC_EDGE_KERNEL_LEN as i32) / 2;
        }
    }
```

(Note: A6's `.take(...)` fix lands in T3, not here. T1 is purely a rename.)

- [ ] **Step 4: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Existing tests are unchanged — this is a pure rename. Expected lib test count: 128 (same as post-#87).

- [ ] **Step 5: Commit**

```bash
git add src/sync.rs
git commit -m "refactor(sync): C2 — name the falling-edge magic numbers (#88)

X_ACC_SLIP_THRESHOLD = 350 (= X_ACC_BINS/2), SYNC_EDGE_KERNEL
(the {1,1,1,1,-1,-1,-1,-1} literal), SYNC_EDGE_KERNEL_LEN = 8.
Replaces three bare numbers in find_sync; no behavior change.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: B4 + B2 — `ModeSpec::skip_correction_seconds()` + extract `skip_seconds_for`

Move the Scottie inline correction onto `ModeSpec`. Extract `skip_seconds_for` as a pure helper. Existing tests must continue to pass.

**Files:**
- Modify: `src/modespec.rs`
- Modify: `src/sync.rs`

- [ ] **Step 1: Add `ModeSpec::skip_correction_seconds()` in `src/modespec.rs`**

Find the `impl ModeSpec` block (search for `^impl ModeSpec` — note: there may be no `impl` block yet because `ModeSpec` uses pub-field-direct-access; if there's no `impl`, add one near the existing const-fn helpers or right before the `pub fn lookup` standalone function around line 136).

If there's no existing `impl ModeSpec` block, add this one (placed right before the `pub fn lookup` function at line 136):

```rust
impl ModeSpec {
    /// Offset (seconds) applied to the raw `xmax`-derived skip to
    /// land on line 0's content start. `LineStart` modes return 0;
    /// `Scottie` modes return `-chan_len/2 + 2 × porch_seconds` (the
    /// Scottie sync is mid-line, so the slip-wrapped `xmax` needs
    /// to be hoisted back left to align with line 0's content
    /// start). Audit #88 B4.
    #[must_use]
    pub(crate) fn skip_correction_seconds(&self) -> f64 {
        match self.sync_position {
            SyncPosition::LineStart => 0.0,
            SyncPosition::Scottie => {
                let chan_len = f64::from(self.line_pixels) * self.pixel_seconds;
                -chan_len / 2.0 + 2.0 * self.porch_seconds
            }
        }
    }
}
```

If there *is* an existing `impl ModeSpec` block, append the method inside it.

- [ ] **Step 2: Add unit tests for the new method**

Find the `#[cfg(test)] mod tests` block in `src/modespec.rs` (search for `^mod tests`). Add these tests inside it:

```rust
    #[test]
    fn skip_correction_seconds_zero_for_line_start_modes() {
        for mode in [
            SstvMode::Pd120,
            SstvMode::Pd180,
            SstvMode::Pd240,
            SstvMode::Robot24,
            SstvMode::Robot36,
            SstvMode::Robot72,
            SstvMode::Martin1,
            SstvMode::Martin2,
        ] {
            let spec = for_mode(mode);
            assert_eq!(
                spec.skip_correction_seconds(),
                0.0,
                "{mode:?} expected 0.0 skip correction"
            );
        }
    }

    #[test]
    fn skip_correction_seconds_scottie_formula() {
        // Scottie1: line_pixels = 320, pixel_seconds = 0.000_43, porch = 0.001_5
        // expected = -(320 * 0.000_43)/2 + 2*0.001_5 = -0.0688 + 0.003 = -0.0658
        for mode in [SstvMode::Scottie1, SstvMode::Scottie2, SstvMode::ScottieDx] {
            let spec = for_mode(mode);
            let expected = -f64::from(spec.line_pixels) * spec.pixel_seconds / 2.0
                + 2.0 * spec.porch_seconds;
            assert!(
                (spec.skip_correction_seconds() - expected).abs() < 1e-12,
                "{mode:?} got {} expected {expected}",
                spec.skip_correction_seconds()
            );
            // Scottie correction is always negative (chan_len/2 always > 2*porch).
            assert!(
                spec.skip_correction_seconds() < 0.0,
                "{mode:?} Scottie correction should be negative"
            );
        }
    }
```

- [ ] **Step 3: Run the modespec tests to confirm green**

```bash
cargo test --all-features --locked --release --lib modespec -- skip_correction
```

Expected: 2 passed.

- [ ] **Step 4: Extract `skip_seconds_for` in `src/sync.rs`**

Add this helper near the bottom of `src/sync.rs`, right before the `#[cfg(test)]` block:

```rust
/// Convert a falling-edge `xmax` (post-slip-wrap) to skip seconds,
/// applying the mode's sync-position offset. Pure arithmetic — no
/// global state. The raw `s_secs` is computed assuming the falling
/// edge lands at `(xmax / X_ACC_BINS) × line_seconds` and the sync
/// pulse runs `sync_seconds` long; `ModeSpec::skip_correction_seconds()`
/// then hoists the result for mid-line-sync modes (Scottie).
#[allow(clippy::cast_precision_loss)]
fn skip_seconds_for(xmax: i32, spec: ModeSpec) -> f64 {
    let raw = (f64::from(xmax) / (X_ACC_BINS as f64)) * spec.line_seconds - spec.sync_seconds;
    raw + spec.skip_correction_seconds()
}
```

- [ ] **Step 5: Replace the inline `s_secs_raw`/`s_secs` block in `find_sync`**

Find the inline correction block in `find_sync` (currently around lines 353-365):

```rust
    let s_secs_raw =
        (f64::from(xmax) / (X_ACC_BINS as f64)) * spec.line_seconds - spec.sync_seconds;
    // sync.c:123-125 — Scottie modes don't start lines with sync.
    // The slip-wrapped xmax doesn't correspond to a line-start anchor,
    // so apply slowrx C's mode-specific correction to bring `s_secs`
    // back to ~0 (line 0's content start).
    let s_secs = match spec.sync_position {
        crate::modespec::SyncPosition::LineStart => s_secs_raw,
        crate::modespec::SyncPosition::Scottie => {
            let chan_len = f64::from(spec.line_pixels) * spec.pixel_seconds;
            s_secs_raw - chan_len / 2.0 + 2.0 * spec.porch_seconds
        }
    };
    let skip_samples = (s_secs * rate).round() as i64;
```

Replace with:

```rust
    // sync.c:120-125 — convert xmax to skip seconds. The Scottie
    // mid-line correction lives on ModeSpec; see ModeSpec::skip_correction_seconds.
    let s_secs = skip_seconds_for(xmax, spec);
    let skip_samples = (s_secs * rate).round() as i64;
```

The `crate::modespec::SyncPosition` import inside the function is no longer needed; the existing `use crate::modespec::ModeSpec;` at the top of the file already covers it.

- [ ] **Step 6: Run the full gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Existing 7 `find_sync` tests stay green (pure refactor — no behavior change). Two new tests added: 130 lib tests total (128 post-T1 + 2 new modespec tests).

- [ ] **Step 7: Commit**

```bash
git add src/modespec.rs src/sync.rs
git commit -m "refactor(sync): B4 — move Scottie skip correction onto ModeSpec (#88)

Adds ModeSpec::skip_correction_seconds() returning 0 for LineStart
modes, -chan_len/2 + 2*porch for Scottie. find_sync now calls a new
skip_seconds_for(xmax, spec) helper instead of an inline match on
sync_position. Pure refactor — existing find_sync tests unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: B2 + A6 — Extract `find_falling_edge` + `falling_edge_from_x_acc`; fix off-by-one

Split the column-accumulator-and-convolution code into two helpers: `find_falling_edge` (builds `x_acc` from `has_sync`, then calls the convolution) and `falling_edge_from_x_acc` (pure convolution + slip-wrap). The pure helper is the natural test surface for A6.

**Files:**
- Modify: `src/sync.rs`

- [ ] **Step 1: Add `falling_edge_from_x_acc` near the bottom of `src/sync.rs`**

Place it right before the `skip_seconds_for` helper added in T2 (so the helpers cluster). Body uses the new consts from T1 and the A6 fix.

```rust
/// Pure 8-tap falling-edge convolution + slip-wrap. Returns `xmax`
/// already adjusted for the `X_ACC_SLIP_THRESHOLD` right-edge slip.
///
/// **A6 fix (#88):** The loop iterates exactly `X_ACC_BINS - SYNC_EDGE_KERNEL_LEN`
/// times — matching slowrx C's `for (n=0; n<X_ACC_BINS-8; n++)`. The
/// rust `Iterator::windows(8)` natively yields 693 windows over a
/// 700-element slice (indices 0..=692); the `.take(X_ACC_BINS - 8)`
/// caps it at 692 (indices 0..=691), so `xAcc[699]` is never read.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn falling_edge_from_x_acc(x_acc: &[u32]) -> i32 {
    debug_assert_eq!(x_acc.len(), X_ACC_BINS, "x_acc must be X_ACC_BINS long");
    let mut xmax: i32 = 0;
    let mut max_convd: i32 = 0;
    for (x, window) in x_acc
        .windows(SYNC_EDGE_KERNEL_LEN)
        .take(X_ACC_BINS - SYNC_EDGE_KERNEL_LEN)
        .enumerate()
    {
        let convd: i32 = window
            .iter()
            .zip(SYNC_EDGE_KERNEL.iter())
            .map(|(&v, &k)| (v as i32) * k)
            .sum();
        if convd > max_convd {
            max_convd = convd;
            xmax = (x as i32) + (SYNC_EDGE_KERNEL_LEN as i32) / 2;
        }
    }

    // sync.c:117 — pulse near the right edge slipped from previous left.
    if xmax > X_ACC_SLIP_THRESHOLD {
        xmax -= X_ACC_SLIP_THRESHOLD;
    }

    xmax
}
```

- [ ] **Step 2: Add `find_falling_edge` near the bottom of `src/sync.rs`**

Place it between `falling_edge_from_x_acc` and `skip_seconds_for`. Body lifts the column-accumulator loop out of `find_sync`.

```rust
/// Column-accumulate `has_sync` into `X_ACC_BINS` bins at `rate_hz`,
/// then convolve with `SYNC_EDGE_KERNEL` to find the steepest falling
/// edge. Returns the `xmax` integer with the `X_ACC_SLIP_THRESHOLD`
/// right-edge slip-wrap already applied.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn find_falling_edge(
    has_sync: &[bool],
    rate_hz: f64,
    spec: ModeSpec,
    num_lines: usize,
) -> i32 {
    let probe_index = |t: f64| -> usize {
        let raw = t * rate_hz / (SYNC_PROBE_STRIDE as f64);
        if raw < 0.0 {
            0
        } else {
            raw as usize
        }
    };

    let mut x_acc = vec![0u32; X_ACC_BINS];
    for y in 0..num_lines {
        for (x, slot) in x_acc.iter_mut().enumerate() {
            let t = (y as f64) * spec.line_seconds
                + ((x as f64) / (X_ACC_BINS as f64)) * spec.line_seconds;
            let idx = probe_index(t);
            if idx < has_sync.len() && has_sync[idx] {
                *slot = slot.saturating_add(1);
            }
        }
    }

    falling_edge_from_x_acc(&x_acc)
}
```

- [ ] **Step 3: Replace the inline column-accumulator + convolution in `find_sync`**

In `src/sync.rs`, find the block in `find_sync` that runs after the retry loop and before the `s_secs = skip_seconds_for(...)` call (after T2's edits, the block currently spans roughly the column-accumulator loop through the slip-wrap). Replace from the comment `// Column accumulator + 8-tap convolution edge-find` through the slip-wrap `if` (lines that were ~306-343 in the pre-T1 file) with a single call:

```rust
    let xmax = find_falling_edge(has_sync, rate, spec, num_lines);
```

That single line replaces:
- The `let mut x_acc = vec![0u32; X_ACC_BINS];` declaration.
- The column-accumulator double-`for` loop.
- The `let kernel: [i32; 8] = ...` (already replaced by T1's `SYNC_EDGE_KERNEL`).
- The `xmax`/`max_convd` init.
- The `.windows(8).enumerate()` convolution loop.
- The slip-wrap `if xmax > X_ACC_SLIP_THRESHOLD` block.

`find_sync` now reads: retry loop → `find_falling_edge` → `skip_seconds_for` → assemble `SyncResult`.

- [ ] **Step 4: Add a TDD-red unit test on `falling_edge_from_x_acc` for the A6 fix**

In `src/sync.rs::tests` (find the `#[cfg(test)] mod tests` block), add:

```rust
    /// A6 regression guard (#88). slowrx C's loop runs `n ∈ 0..691`
    /// (`for (n=0; n<X_ACC_BINS-8; n++)`), so `xAcc[699]` is never
    /// read. Rust's native `windows(8)` over a 700-bin slice yields
    /// 693 windows (`n ∈ 0..=692`); without `.take(X_ACC_BINS - 8)`
    /// the kernel would read `xAcc[699]` at `n=692`. This test
    /// constructs an `x_acc` whose strongest convd at n=692 differs
    /// from the strongest at n=691: pre-fix lands at n=692 (xmax=696
    /// → slip-wrap → 346); post-fix lands at n=691 (xmax=695 →
    /// slip-wrap → 345). The assertion fails on the pre-fix code and
    /// passes on the post-fix code.
    #[test]
    fn falling_edge_from_x_acc_off_by_one_a6() {
        let mut x_acc = vec![0u32; X_ACC_BINS];
        // x_acc[691..=695] = 100, x_acc[696..=699] = 0.
        for i in 691..=695 {
            x_acc[i] = 100;
        }
        // n=691: window = [100,100,100,100,100,0,0,0]
        //   convd = 4*100 - 100 - 0 - 0 - 0 = 300.
        // n=692: window = [100,100,100,100,0,0,0,0]
        //   convd = 4*100 - 0 = 400  (pre-fix only).
        // Pre-fix max at n=692 → xmax = 696 → slip-wrap → 346.
        // Post-fix max at n=691 → xmax = 695 → slip-wrap → 345.
        let xmax = falling_edge_from_x_acc(&x_acc);
        assert_eq!(
            xmax, 345,
            "post-A6-fix should pick n=691 (xmax=695, slip=345); pre-fix would give 346"
        );
    }

    /// A6 baseline: an edge well away from the right edge produces
    /// the same `xmax` pre-fix and post-fix. Sanity check that the
    /// `.take(...)` bound only changes behavior at the very right
    /// edge, not anywhere else.
    #[test]
    fn falling_edge_from_x_acc_detects_mid_array_edge() {
        let mut x_acc = vec![0u32; X_ACC_BINS];
        // Edge at indices 100..103 — well away from the right edge.
        for i in 100..=103 {
            x_acc[i] = 100;
        }
        // n=100: window = [100,100,100,100,0,0,0,0], convd = 400,
        // xmax = 100 + 4 = 104. 104 < X_ACC_SLIP_THRESHOLD (350), no
        // slip-wrap. Pre-fix and post-fix agree.
        let xmax = falling_edge_from_x_acc(&x_acc);
        assert_eq!(xmax, 104, "mid-array edge detection unchanged by A6 fix");
    }
```

- [ ] **Step 5: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib count: 132 (130 post-T2 + 2 new A6 tests). Existing `find_sync` tests stay green (the extraction is pure; the A6 fix only matters for edges at n ∈ {692}, which no existing test produces).

- [ ] **Step 6: Commit**

```bash
git add src/sync.rs
git commit -m "refactor(sync): B2 partial + A6 fix — find_falling_edge + falling_edge_from_x_acc (#88)

Extracts find_falling_edge (column-accumulator + convolution +
slip-wrap) and falling_edge_from_x_acc (pure convolution + slip-wrap)
from find_sync. The pure helper takes only the 700-bin x_acc slice,
making A6 directly testable: two new unit tests verify (a) an edge
at the would-be-buggy last window position n=692 is ignored, and
(b) an edge at n=691 (the new last valid position) is still
detected. Loop bound is now .take(X_ACC_BINS - 8) = 692 iterations,
matching slowrx C exactly.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: B2 final + C10 — Extract `hough_detect_slant` + index closures

Lift the Hough-transform retry loop out of `find_sync` into a pure helper. Inside it, replace bare-int 2D indexing with named closures. Drop the `#[allow(too_many_lines)]` from `find_sync` (no longer needed).

**Files:**
- Modify: `src/sync.rs`

- [ ] **Step 1: Add `hough_detect_slant` near the bottom of `src/sync.rs`**

Place it BEFORE `find_falling_edge` so the helpers read in the order they're called from `find_sync`. Body lifts the sync-image build + Hough vote loop.

```rust
/// Build the 2D sync image at `rate_hz`, then linear-Hough-transform
/// it to find the dominant slant angle. Returns `None` when no sync
/// pulses register at all (degenerate input). The returned
/// `adjusted_rate` already has the standard Hough-derived correction
/// applied (`rate × tan(90° − slant) / line_width × rate`); the
/// caller applies the 90° deadband before adopting it.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn hough_detect_slant(
    has_sync: &[bool],
    rate_hz: f64,
    spec: ModeSpec,
    line_width: usize,
) -> Option<(f64 /* slant_deg */, f64 /* adjusted_rate */)> {
    let n_slant_bins = ((MAX_SLANT_DEG - MIN_SLANT_DEG) / SLANT_STEP_DEG).round() as usize;
    let num_lines = spec.image_lines as usize;

    // Column-major: x is the outer dim because the Hough vote loop
    // iterates `for cy { for cx { … } }` and we want sequential x to
    // share a cache line. Matches slowrx C's `SyncImg[700][630]`
    // shape (C10 audit).
    let sync_img_idx = |x: usize, y: usize| x * SYNC_IMG_Y_BINS + y;

    // Row-major: d is the outer dim (the slowrx C `Lines[600][240]`
    // shape). Vote increments scan q-inner.
    let lines_idx = |d: usize, q: usize| d * n_slant_bins + q;

    let probe_index = |t: f64| -> usize {
        let raw = t * rate_hz / (SYNC_PROBE_STRIDE as f64);
        if raw < 0.0 {
            0
        } else {
            raw as usize
        }
    };

    // Draw the 2D sync signal at current rate.
    let mut sync_img = vec![false; X_ACC_BINS * SYNC_IMG_Y_BINS];
    for y in 0..num_lines.min(SYNC_IMG_Y_BINS) {
        for x in 0..line_width.min(X_ACC_BINS) {
            let t = ((y as f64) + (x as f64) / (line_width as f64)) * spec.line_seconds;
            let idx = probe_index(t);
            if idx < has_sync.len() {
                sync_img[sync_img_idx(x, y)] = has_sync[idx];
            }
        }
    }

    // Linear Hough transform.
    let mut lines = vec![0u16; LINES_D_BINS * n_slant_bins];
    let mut q_most = 0_usize;
    let mut max_count = 0_u16;
    for cy in 0..num_lines.min(SYNC_IMG_Y_BINS) {
        for cx in 0..line_width.min(X_ACC_BINS) {
            if !sync_img[sync_img_idx(cx, cy)] {
                continue;
            }
            for q in 0..n_slant_bins {
                let theta = deg2rad(MIN_SLANT_DEG + (q as f64) * SLANT_STEP_DEG);
                let d_signed = (line_width as f64)
                    + (-(cx as f64) * theta.sin() + (cy as f64) * theta.cos()).round();
                if d_signed > 0.0 && d_signed < (line_width as f64) {
                    let d = d_signed as usize;
                    if d < LINES_D_BINS {
                        let cell = &mut lines[lines_idx(d, q)];
                        *cell = cell.saturating_add(1);
                        if *cell > max_count {
                            max_count = *cell;
                            q_most = q;
                        }
                    }
                }
            }
        }
    }

    if max_count == 0 {
        return None;
    }

    let slant_angle = MIN_SLANT_DEG + (q_most as f64) * SLANT_STEP_DEG;
    let adjusted_rate =
        rate_hz + (deg2rad(90.0 - slant_angle).tan() / (line_width as f64)) * rate_hz;
    Some((slant_angle, adjusted_rate))
}
```

- [ ] **Step 2: Rewrite `find_sync` to orchestrate the helpers**

In `src/sync.rs`, replace the entire body of `find_sync` (currently a 160-line function with `#[allow(too_many_lines)]`) with:

```rust
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
pub(crate) fn find_sync(has_sync: &[bool], initial_rate_hz: f64, spec: ModeSpec) -> SyncResult {
    let line_width: usize = ((spec.line_seconds / spec.sync_seconds) * 4.0) as usize;
    let num_lines = spec.image_lines as usize;
    let mut rate = initial_rate_hz;
    let mut slant_deg_detected: Option<f64> = None;

    for retry in 0..=MAX_SLANT_RETRIES {
        let Some((slant, adjusted)) = hough_detect_slant(has_sync, rate, spec, line_width) else {
            // No sync pulses → no Hough peak → no rate correction.
            break;
        };
        slant_deg_detected = Some(slant);

        // Apply a deadband at 90° so an exact-rate input is not perturbed
        // by half-degree Hough quantization noise (see
        // docs/intentional-deviations.md "FindSync 90° slant deadband").
        if (slant - 90.0).abs() > SLANT_STEP_DEG {
            rate = adjusted;
        }

        // sync.c:86-90 resets to 44100 on retry exhaustion; we keep
        // our last estimate (see docs/intentional-deviations.md
        // "FindSync retry-exhaustion"). Open interval (89, 91) matches
        // slowrx sync.c:83 exactly — half-open `89.0..91.0` would widen
        // the lock by one 0.5°-Hough bin (round-2 audit Finding 7).
        if (slant > SLANT_OK_LO_DEG && slant < SLANT_OK_HI_DEG) || retry == MAX_SLANT_RETRIES {
            break;
        }
    }

    let xmax = find_falling_edge(has_sync, rate, spec, num_lines);
    let s_secs = skip_seconds_for(xmax, spec);
    let skip_samples = (s_secs * rate).round() as i64;

    SyncResult {
        adjusted_rate_hz: rate,
        skip_samples,
        slant_deg: slant_deg_detected,
    }
}
```

Note the dropped `#[allow(clippy::too_many_lines)]` — the function is now ~30 lines.

- [ ] **Step 3: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Existing 7 `find_sync` tests stay green (pure refactor — same logic, just relocated). The 2 new A6 tests from T3 also stay green. Expected lib count: 132 (unchanged from post-T3).

Watch for: if clippy complains about the removed `clippy::too_many_lines` allow being now-unused, that's a green-to-amber transition CI would surface as `clippy::useless_attribute`. The fix is just to remove the unused allow; it's not present on the post-T2 `find_sync` though, so this shouldn't fire. If `clippy::needless_pass_by_value` fires on `spec: ModeSpec` for the new helpers, add `#[allow(clippy::needless_pass_by_value)]` — `ModeSpec` is `Copy` and small (~10 f64s), pass-by-value is intentional.

- [ ] **Step 4: Commit**

```bash
git add src/sync.rs
git commit -m "refactor(sync): B2 final + C10 — extract hough_detect_slant; index closures (#88)

Lifts the sync-image build + Hough vote loop into hough_detect_slant,
returning Option<(slant_deg, adjusted_rate)>. find_sync becomes a
~30-line orchestrator over the four helpers (hough_detect_slant ×
4-retry loop + find_falling_edge + skip_seconds_for). Inside
hough_detect_slant, sync_img and lines use named closures
(sync_img_idx column-major, lines_idx row-major) with parity-with-C
comments. Drops the #[allow(clippy::too_many_lines)] attribute.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: A7 + A8 — Append deviation entries to `docs/intentional-deviations.md`

Pure documentation. Two new sections after the existing #39 and #42 entries. No code changes.

**Files:**
- Modify: `docs/intentional-deviations.md`

- [ ] **Step 1: Read the existing file to find the right insertion point**

Run: `grep -n "^## " /data/source/slowrx.rs/docs/intentional-deviations.md`

Expected output:
```text
## VIS stop-bit boundary: precise vs. ±20 ms slop
## FindSync 90° slant deadband
```

The new entries go at the end of the file. Confirm the file ends with the "When to revisit" section of the 90° deadband entry (and possibly a blank line).

- [ ] **Step 2: Append the two new entries**

Open `docs/intentional-deviations.md`. After the last line, append (be sure to preserve the existing `---` separator pattern between entries):

```markdown

---

## FindSync skip_samples rounding: round-to-nearest vs slowrx's truncation

**Files:** `src/sync.rs::find_sync` ↔ slowrx `sync.c:120`.
**Tracking issue:** (none — sub-sample effect, no observable behavior change).

### What slowrx does

`Skip = s * Rate;` is an implicit `double → int` cast, which in C
truncates toward zero. A `s_secs * rate` of `0.6` lands at `0`.

### What we do

`let skip_samples = (s_secs * rate).round() as i64;` rounds to nearest.

### Why we deviated

Truncation in slowrx isn't a deliberate choice — it's the side effect
of an implicit C cast idiom. Round-to-nearest minimizes the max
sub-sample error (½ sample vs 1 sample). The difference is at most
~91 µs at 11025 Hz — well below SSTV's per-pixel duration (~0.5 ms at
PD120) and invisible in real-radio capture.

### When to revisit

If a bit-exact parity test against slowrx-C reference output ever
requires matching to the integer sample, switch back to truncation.

---

## FindSync retry-exhaustion: keep last estimate vs slowrx's reset to 44100

**Files:** `src/sync.rs::find_sync` ↔ slowrx `sync.c:86-90`.
**Tracking issue:** (none).

### What slowrx does

After `MAX_SLANT_RETRIES` Hough passes without locking inside the
`(89°, 91°)` window, slowrx resets `Rate` to 44100 (its working
sample rate) — i.e. throws away all the slant-correction progress
made over the retries.

### What we do

We keep the last adjusted `rate` even when the lock window isn't
reached.

### Why we deviated

Re-anchoring a near-locked input is harmful: if 3 retries narrowed
the slant from 70° to 91.1° (one Hough bin outside the lock window),
a reset to 44100 throws away the 19° of correction we made. Keeping
the last estimate gives a better decode on borderline locks.

### When to revisit

If a regression surfaces where rate-correction overshoots and an
explicit reset gives a better outcome. Has not happened in the
Dec-2017 ARISS validation set.
```

- [ ] **Step 3: Run the gate (doc-only changes — just sanity-check)**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass (this commit changes only a markdown file, so all should be unchanged from T4). Expected lib count: 132.

- [ ] **Step 4: Commit**

```bash
git add docs/intentional-deviations.md
git commit -m "docs(deviations): A7 + A8 — find_sync rounding + retry-exhaustion (#88)

Two new entries in intentional-deviations.md alongside the existing
#39 VIS stop-bit and #42 90° deadband entries. A7 documents the
.round() vs slowrx's implicit-truncate choice; A8 documents the
keep-last-estimate vs reset-to-44100 choice. No code change — these
behaviors already existed; the entries make the deviations explicit
for future parity audits.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: F2 — Hough slant-correction tests

Two new tests for the slant-correction retry path that no existing test exercises (every existing `find_sync` test feeds the exact working rate ⇒ slant ≈ 90° ⇒ deadband absorbs everything ⇒ correction never runs). Adds one new synthesis helper.

**Files:**
- Modify: `src/sync.rs`

- [ ] **Step 1: Add the `synth_has_sync_slanted` helper**

In `src/sync.rs::tests`, immediately after the existing `synth_has_sync` helper (around line 421-436 in the pre-#88 file), add:

```rust
    /// Build a synthetic `has_sync` track where the signal was *captured*
    /// at `capture_rate_hz` but the true line cadence runs at
    /// `true_rate_hz`. Each captured line is `true_rate / capture_rate`
    /// of a real line, so sync pulses drift through the (probe-stride-
    /// quantized) track — i.e. the slant is non-90°.
    fn synth_has_sync_slanted(
        spec: ModeSpec,
        true_rate_hz: f64,
        capture_rate_hz: f64,
    ) -> Vec<bool> {
        // Total length sized for the captured rate.
        let total = (f64::from(spec.image_lines) * spec.line_seconds * capture_rate_hz
            / (SYNC_PROBE_STRIDE as f64)) as usize
            + 16;
        let mut track = vec![false; total];
        for y in 0..spec.image_lines {
            // Sync pulse y starts at `y * line_seconds_true` (true cadence),
            // but the probe-index is computed at `capture_rate`.
            let line_start_t =
                f64::from(y) * spec.line_seconds * (true_rate_hz / capture_rate_hz);
            let line_end_t = line_start_t + spec.sync_seconds;
            let i_start = (line_start_t * capture_rate_hz / (SYNC_PROBE_STRIDE as f64)) as usize;
            let i_end = (line_end_t * capture_rate_hz / (SYNC_PROBE_STRIDE as f64)) as usize;
            for slot in track.iter_mut().take(i_end.min(total)).skip(i_start) {
                *slot = true;
            }
        }
        track
    }
```

- [ ] **Step 2: Add the first F2 test — 0.5% slant, correction must converge**

(Post-implementation note: spec/plan originally targeted 0.3% drift, but T6 found that drift falls inside the 0.5°-quantized Hough deadband and never triggers correction. Bumped to 0.5%, the minimum drift that reliably runs the correction path; comment + assertions updated accordingly.)

In the same `tests` block, after the existing `find_sync_handles_empty_track` test, add:

```rust
    /// F2 (#88). Hough slant correction path — 0.5% capture-rate
    /// drift produces a Hough peak well outside the (89°, 91°) lock
    /// window. The retry loop must shrink the rate error toward
    /// zero; we assert it ends up under half the initial drift.
    #[test]
    fn find_sync_corrects_0p5pct_slant_at_pd120() {
        let spec = modespec::for_mode(crate::modespec::SstvMode::Pd120);
        let true_rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let capture_rate = true_rate * 1.005;
        let track = synth_has_sync_slanted(spec, true_rate, capture_rate);
        let r = find_sync(&track, capture_rate, spec);
        let err_pct = (r.adjusted_rate_hz - true_rate).abs() / true_rate * 100.0;
        assert!(
            err_pct < 0.05,
            "rate err {err_pct:.3}% (got {} expected ≈ {true_rate})",
            r.adjusted_rate_hz
        );
        let slant = r.slant_deg.expect("sync detected");
        // The detected slant is the pre-correction angle (off-90); the
        // adjusted rate is the post-correction value.
        assert!(
            (slant - 90.0).abs() > SLANT_STEP_DEG,
            "slant {slant:.2}° should be off-90 (correction was needed)"
        );
    }
```

- [ ] **Step 3: Add the second F2 test — 1% slant, retries shrink error**

In the same `tests` block, after the previous test, add:

```rust
    /// F2 (#88). Larger drift (1% capture-rate offset) produces a
    /// Hough peak far outside the lock window — the retry loop must
    /// do real work to converge. The final rate error must be
    /// strictly smaller than the initial guess, and small enough to
    /// be considered converged (< 0.2%).
    #[test]
    fn find_sync_corrects_1pct_slant_via_retries() {
        let spec = modespec::for_mode(crate::modespec::SstvMode::Pd120);
        let true_rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let capture_rate = true_rate * 1.01;
        let track = synth_has_sync_slanted(spec, true_rate, capture_rate);
        let r = find_sync(&track, capture_rate, spec);
        let initial_err_pct = (capture_rate - true_rate).abs() / true_rate * 100.0;
        let final_err_pct = (r.adjusted_rate_hz - true_rate).abs() / true_rate * 100.0;
        assert!(
            final_err_pct < initial_err_pct,
            "retry should shrink rate error: initial {initial_err_pct:.3}% → final {final_err_pct:.3}%"
        );
        assert!(
            final_err_pct < 0.2,
            "rate err after retries should be ≤ 0.2%, got {final_err_pct:.3}%"
        );
    }
```

- [ ] **Step 4: Run the F2 tests in isolation**

```bash
cargo test --all-features --locked --release --lib sync -- corrects
```

Expected: 2 passed (`find_sync_corrects_0p5pct_slant_at_pd120`, `find_sync_corrects_1pct_slant_via_retries`).

If `find_sync_corrects_1pct_slant_via_retries` fails with `final_err_pct > 0.2`, increase the tolerance to 0.5 — the convergence depends on Hough quantization, and a 0.5% bound is still a meaningful pass. Don't relax `final_err_pct < initial_err_pct` — that assertion is load-bearing.

- [ ] **Step 5: Run the full gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib count: 134 (132 post-T5 + 2 new F2 tests).

- [ ] **Step 6: Commit**

```bash
git add src/sync.rs
git commit -m "test(sync): F2 — Hough slant-correction tests (#88)

Two new tests for the rate-correction retry loop that no existing
test exercises (every existing find_sync test feeds the exact working
rate, so slant ≈ 90° and the deadband absorbs everything). The new
synth_has_sync_slanted(spec, true_rate, capture_rate) helper
constructs a track where the captured rate differs from the true
line cadence by a configurable percentage.

- find_sync_corrects_0p5pct_slant_at_pd120: 0.5% drift → correction
  converges. Post-correction rate < half the initial drift.
- find_sync_corrects_1pct_slant_via_retries: 1% drift → out-of-window,
  forces retries. Post-correction error must be strictly < initial.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: F3 — Scottie correction test

One new test that exercises the `SyncPosition::Scottie` branch of `ModeSpec::skip_correction_seconds()`. Reuses the existing `synth_has_sync` helper (no new helper needed).

**Files:**
- Modify: `src/sync.rs`

- [ ] **Step 1: Add the F3 test**

In `src/sync.rs::tests`, after the F2 tests added in T6, add:

```rust
    /// F3 (#88). Scottie modes use mid-line sync; the
    /// skip_correction_seconds() path on ModeSpec subtracts
    /// `chan_len/2 - 2*porch` from the raw `s_secs`. Feeding
    /// line-start pulses (the existing `synth_has_sync` helper)
    /// with a Scottie spec lands `xmax` near 0 (small), so `s_secs_raw
    /// ≈ 0` and the final skip should equal the correction itself
    /// (negative, ~ -65 ms for Scottie1 at 11025 Hz).
    #[test]
    fn find_sync_scottie_applies_skip_correction() {
        let spec = modespec::for_mode(crate::modespec::SstvMode::Scottie1);
        let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
        let track = synth_has_sync(spec, rate);
        let r = find_sync(&track, rate, spec);

        let chan_len = f64::from(spec.line_pixels) * spec.pixel_seconds;
        let expected_secs = -chan_len / 2.0 + 2.0 * spec.porch_seconds;
        let expected_skip = (expected_secs * rate).round() as i64;
        let tolerance = (0.005 * rate) as i64; // ~55 samples ≈ 5 ms

        assert!(
            (r.skip_samples - expected_skip).abs() < tolerance,
            "Scottie skip {} should be ≈ {expected_skip} (correction = {expected_secs:.4}s, tol = {tolerance})",
            r.skip_samples
        );
        // Sanity: Scottie correction is always negative.
        assert!(
            r.skip_samples < 0,
            "Scottie skip should be negative (mid-line hoist); got {}",
            r.skip_samples
        );
    }
```

- [ ] **Step 2: Run the F3 test in isolation**

```bash
cargo test --all-features --locked --release --lib sync -- find_sync_scottie
```

Expected: 1 passed (`find_sync_scottie_applies_skip_correction`).

If the assertion fires with `(r.skip_samples - expected_skip).abs() ≥ tolerance`, debug by printing the actual `r.skip_samples` and `expected_skip` in the failure message (already there). The most likely cause is the xmax detection landing further from 0 than expected — try printing `r.slant_deg` and the raw xmax intermediate (transient debug; remove before commit). If the test fails systematically with a small but consistent offset (e.g., one bin = ~0.6 ms ≈ 7 samples for Scottie1), the tolerance is too tight; bump to `(0.010 * rate) as i64` ≈ 110 samples.

- [ ] **Step 3: Run the full gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib count: 135 (134 post-T6 + 1 new F3 test).

- [ ] **Step 4: Commit**

```bash
git add src/sync.rs
git commit -m "test(sync): F3 — Scottie skip correction test (#88)

Verifies the SyncPosition::Scottie branch of
ModeSpec::skip_correction_seconds() actually runs in find_sync: feed
line-start sync pulses with a Scottie1 spec, assert the resulting
skip_samples lands at the expected correction offset
(-chan_len/2 + 2*porch ≈ -65 ms at 11025 Hz) within a 5 ms tolerance.
Direct coverage of the Scottie arm in find_sync, which the audit
flagged as having zero direct test coverage.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: CHANGELOG + final gate

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add the `CHANGELOG.md` `[Unreleased]` entry**

Open `CHANGELOG.md`. Under `## [Unreleased]` `### Internal`, prepend (so the newest change is first) a new bullet **before** the existing resampler-polish bullet:

```markdown
### Internal

- **`find_sync` cleanup** — decomposes the 160-line `find_sync` into
  three pure helpers (`hough_detect_slant`, `find_falling_edge` /
  `falling_edge_from_x_acc`, `skip_seconds_for`), drops the
  `#[allow(clippy::too_many_lines)]` attribute. Moves the Scottie
  mid-line skip correction onto `ModeSpec::skip_correction_seconds()`
  (audit B4) so `find_sync` is mode-agnostic. Names three magic
  numbers (`X_ACC_SLIP_THRESHOLD`, `SYNC_EDGE_KERNEL`,
  `SYNC_EDGE_KERNEL_LEN` — audit C2). Replaces bare-int 2D indexing
  with local `sync_img_idx` / `lines_idx` closures (audit C10). Fixes
  one real off-by-one bug in the falling-edge convolution loop —
  Rust's native `windows(8)` over a 700-bin slice yields 693 windows
  but slowrx C iterates exactly 692; `.take(X_ACC_BINS - 8)` brings
  us to bit-exact parity (audit A6). Documents two slowrx-C-vs-Rust
  behavioral choices in `docs/intentional-deviations.md`: `.round()`
  vs implicit truncate for `skip_samples` (A7), and keep-last-estimate
  vs reset-to-44100 on retry exhaustion (A8). Adds three new tests:
  two Hough slant-correction tests (0.5% and 1% drift — audit F2), one
  Scottie skip-correction test (audit F3). (#88; audit B2/B4/C2/C10/A6/A7/A8/F2/F3.)

- **Resampler polish** — ... [existing #87 bullet stays as-is below]
```

- [ ] **Step 2: Run the full CI gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected test counts: lib 135 (128 pre-#88 + 2 modespec from T2 + 2 sync from T3 + 2 sync from T6 + 1 sync from T7); roundtrip 11/11; everything else unchanged.

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(refactor): CHANGELOG for the find_sync cleanup (#88)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-review notes (for the implementer / reviewers)

- **Spec coverage:**
  - B2 (decompose `find_sync`) → T2 (skip_seconds_for) + T3 (find_falling_edge + falling_edge_from_x_acc) + T4 (hough_detect_slant)
  - B4 (Scottie correction onto ModeSpec) → T2
  - C2 (named consts) → T1
  - C10 (index closures) → T4
  - A6 (off-by-one fix) → T3, tested via the pure `falling_edge_from_x_acc` helper
  - A7 (`.round()` deviation doc) → T5
  - A8 (retry-exhaustion deviation doc) → T5
  - F2 (slant-correction tests) → T6
  - F3 (Scottie test) → T7
  - CHANGELOG → T8

- **Test count progression:**
  - Pre-#88: 128 lib tests (post-#87 merge baseline).
  - Post-T1: 128 (pure rename).
  - Post-T2: 130 (+2 modespec tests for `skip_correction_seconds`).
  - Post-T3: 132 (+2 A6 regression-guard tests on `falling_edge_from_x_acc`).
  - Post-T4: 132 (pure refactor).
  - Post-T5: 132 (doc-only).
  - Post-T6: 134 (+2 F2 slant-correction tests).
  - Post-T7: 135 (+1 F3 Scottie test).
  - Post-T8: 135 (CHANGELOG only).

- **TDD red:** T3's A6 test is the only deliberate-red-then-green moment. The test `falling_edge_from_x_acc_ignores_last_window_position_a6` would fail on the pre-T3 code (the `.windows(8)` without `.take(...)` reads `x_acc[699]` at n=692, finds a strong edge, sets xmax=696 → slip-wrap → xmax=346). T3 applies the fix; the test passes. All other new tests are net-new coverage on green code.

- **Out of scope** (tracked elsewhere):
  - C16 / inner-loop dead bounds check in `src/resample.rs` — separately tracked in #96.
  - Broader `ModeSpec` refactor — #91.
  - SIMD-ifying anything — #77.
  - Performance hoisting of per-line allocations — #93.
