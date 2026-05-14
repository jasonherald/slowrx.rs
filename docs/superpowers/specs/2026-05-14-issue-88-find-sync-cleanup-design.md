# Issue #88 — `find_sync` cleanup — Design

**Issue:** [#88](https://github.com/jasonherald/slowrx.rs/issues/88) (audit bundle 4 of 12 — IDs B2, B4, C2, C10, A6, A7, A8, F2, F3).

**Scope:** end-to-end polish of `src/sync.rs::find_sync` — decompose the 160-line function into three pure helpers (B2), move the Scottie mid-line correction onto `ModeSpec` (B4), name the magic numbers (C2), replace bare-int 2D indexing with local closures (C10), fix one real off-by-one bug in the falling-edge convolution loop (A6), document two slowrx-C-vs-Rust behavioral choices in `intentional-deviations.md` (A7, A8), and add two new test categories — Hough slant correction (F2) and Scottie mid-line sync (F3). All in `src/sync.rs` + a small addition to `src/modespec.rs` + a new entry pair in `docs/intentional-deviations.md`.

---

## Background — audit findings

- **B2 — decompose `find_sync`.** It's a `#[allow(too_many_lines)]` function combining four responsibilities (build sync image / Hough-detect slant / 4-retry adjust / column-accumulate + edge-find). Extract three pure helpers: `hough_detect_slant`, `find_falling_edge`, `skip_seconds_for`.
- **B4 — Scottie mid-line correction lives in `find_sync`.** The inline `match spec.sync_position` should be a method on `ModeSpec` so `find_sync` is mode-agnostic.
- **C2 — magic numbers.** Three live without names: `350` (the slip-wrap threshold = `X_ACC_BINS/2`), the literal `8`/`+4` for the edge-detection kernel length and center offset, and the `[1,1,1,1,-1,-1,-1,-1]` array literal. Name them: `X_ACC_SLIP_THRESHOLD`, `SYNC_EDGE_KERNEL`, `SYNC_EDGE_KERNEL_LEN`.
- **C10 — inconsistent 2D layouts.** `sync_img` is column-major (`x * SYNC_IMG_Y_BINS + y`); `lines` is row-major (`d * n_slant_bins + q`). The bare-int indexing scatters the layout choice across 6+ sites. **Resolution: local index closures** (one per grid, defined at the top of `find_sync` / `hough_detect_slant` with a parity-with-C comment), not a typed `Grid2D<T>` (borderline YAGNI for two call sites).
- **A6 — real off-by-one bug.** `x_acc.windows(8).enumerate()` yields 693 windows on a 700-bin `x_acc`; slowrx C's `for (n=0; n<X_ACC_BINS-8; n++)` yields 692 (the loop runs for `n ∈ {0,…,691}`, reading `xAcc[n..n+8)`; `xAcc[699]` is never read). The extra right-edge position can shift `xmax` if a stronger falling edge happens to land at `x = 692` (very unlikely on real radio). Fix: `.take(X_ACC_BINS - 8)` ⇒ 692 iterations, bit-exact parity with C.
- **A7 — `skip_samples` rounding deviation.** Rust uses `.round() as i64`; slowrx C truncates via implicit `double→int` cast. **Resolution: keep `.round()`** (semantically correct, sub-sample effect) **and document in `intentional-deviations.md`.**
- **A8 — retry-exhaustion behavior deviation.** Slowrx C resets `Rate` to 44100 on retry exhaustion; we keep the last adjusted rate. **Resolution: keep current behavior** (already commented in code as a deliberate improvement) **and document in `intentional-deviations.md`.**
- **F2 — Hough slant-correction path is untested.** Every existing `find_sync` test feeds the exact working rate ⇒ slant ≈ 90° ⇒ deadband absorbs everything ⇒ correction path never runs. Add `synth_has_sync_slanted(spec, true_rate, capture_rate)` and assert the post-correction rate converges back toward `true_rate`.
- **F3 — Scottie mid-line sync untested.** Add a Scottie-specific test that the `skip_correction_seconds()` path hoists `xmax` back to `s_secs ≈ 0`.

---

## Architecture

### Decomposition (B2)

`find_sync`'s body becomes the orchestration shell:

```rust
pub(crate) fn find_sync(has_sync: &[bool], initial_rate_hz: f64, spec: ModeSpec) -> SyncResult {
    let line_width = ((spec.line_seconds / spec.sync_seconds) * 4.0) as usize;
    let num_lines = spec.image_lines as usize;
    let mut rate = initial_rate_hz;
    let mut slant_deg_detected: Option<f64> = None;

    for retry in 0..=MAX_SLANT_RETRIES {
        let Some((slant, adjusted)) = hough_detect_slant(has_sync, rate, spec, line_width)
        else {
            break;  // No sync detected at all → leave rate untouched.
        };
        slant_deg_detected = Some(slant);
        // 90° deadband: skip the tiny correction when already aligned
        // (see docs/intentional-deviations.md "FindSync 90° slant deadband").
        if (slant - 90.0).abs() > SLANT_STEP_DEG {
            rate = adjusted;
        }
        if (slant > SLANT_OK_LO_DEG && slant < SLANT_OK_HI_DEG) || retry == MAX_SLANT_RETRIES {
            break;
        }
    }

    let xmax = find_falling_edge(has_sync, rate, spec, num_lines);
    let s_secs = skip_seconds_for(xmax, spec);
    let skip_samples = (s_secs * rate).round() as i64;

    SyncResult { adjusted_rate_hz: rate, skip_samples, slant_deg: slant_deg_detected }
}
```

The three helpers:

```rust
/// Build the sync image at `rate_hz`, then linear-Hough-transform it
/// to find the dominant slant angle. Returns `None` when no sync
/// pulses register at all (degenerate input). The returned
/// `adjusted_rate` is `rate_hz × tan(90° − slant) / line_width × rate_hz`
/// applied (the standard Hough-derived rate correction); caller
/// applies the 90° deadband before adopting it.
fn hough_detect_slant(
    has_sync: &[bool],
    rate_hz: f64,
    spec: ModeSpec,
    line_width: usize,
) -> Option<(f64 /* slant_deg */, f64 /* adjusted_rate */)>;

/// Column-accumulate `has_sync` into `X_ACC_BINS` bins at `rate_hz`,
/// then convolve with `SYNC_EDGE_KERNEL` to find the steepest
/// falling edge. Returns the `xmax` integer with the
/// `X_ACC_SLIP_THRESHOLD` right-edge slip-wrap already applied. Loop
/// bound matches slowrx C's `for (n=0; n<X_ACC_BINS-8; n++)` — 692
/// iterations (audit A6).
fn find_falling_edge(
    has_sync: &[bool],
    rate_hz: f64,
    spec: ModeSpec,
    num_lines: usize,
) -> i32;

/// Convert a falling-edge `xmax` to skip seconds, applying the mode's
/// sync-position offset. Pure arithmetic — no global state.
fn skip_seconds_for(xmax: i32, spec: ModeSpec) -> f64 {
    let raw = (f64::from(xmax) / (X_ACC_BINS as f64)) * spec.line_seconds - spec.sync_seconds;
    raw + spec.skip_correction_seconds()
}
```

### `ModeSpec::skip_correction_seconds()` (B4)

`src/modespec.rs` gains a pub(crate) method:

```rust
impl ModeSpec {
    /// Offset (seconds) applied to the raw `xmax`-derived skip to land
    /// on line 0's content start. `LineStart` modes return 0;
    /// `Scottie` modes return `-chan_len/2 + 2 × porch_seconds` (the
    /// Scottie sync is mid-line, so the slip-wrapped `xmax` needs to
    /// be hoisted back left).
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

`find_sync` calls it exactly once, indirectly via `skip_seconds_for`. The inline `match spec.sync_position` block in `find_sync` (sync.rs:359-365) is deleted.

### Named constants (C2)

Add to the existing const block near the top of `src/sync.rs`:

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

In `find_falling_edge`:
- `xmax = (x as i32) + 4` becomes `xmax = (x as i32) + (SYNC_EDGE_KERNEL_LEN as i32) / 2;`
- `if xmax > 350 { xmax -= 350; }` becomes `if xmax > X_ACC_SLIP_THRESHOLD { xmax -= X_ACC_SLIP_THRESHOLD; }`
- The array literal `let kernel: [i32; 8] = [1, 1, 1, 1, -1, -1, -1, -1];` is gone — `SYNC_EDGE_KERNEL` is used directly.

### Index closures (C10)

At the top of `find_sync` (or rather inside `hough_detect_slant` where they're used):

```rust
// Column-major: x is the outer dim because the Hough vote loop
// iterates `for cy { for cx { … } }` and we want sequential x to
// share a cache line. Matches slowrx C's `SyncImg[700][630]`
// indexing order.
let sync_img_idx = |x: usize, y: usize| x * SYNC_IMG_Y_BINS + y;

// Row-major: d is the outer dim (the slowrx C `Lines[600][240]`
// shape). Vote increments scan q-inner.
let lines_idx = |d: usize, q: usize| d * n_slant_bins + q;
```

Access sites become `sync_img[sync_img_idx(cx, cy)]` and `lines[lines_idx(d, q)]`. The layout-choice rationale is now visible exactly twice (the closure definitions), not scattered.

### A6 off-by-one bug fix

In `find_falling_edge`, change:
```rust
for (x, window) in x_acc.windows(8).enumerate() { … }
```
to:
```rust
for (x, window) in x_acc
    .windows(SYNC_EDGE_KERNEL_LEN)
    .take(X_ACC_BINS - SYNC_EDGE_KERNEL_LEN)
    .enumerate()
{
    …
}
```
That's `X_ACC_BINS - SYNC_EDGE_KERNEL_LEN = 700 - 8 = 692` iterations, matching slowrx C's `for (n=0; n<X_ACC_BINS-8; n++)`. The current code does 693 iterations (`windows(8)` over 700 elements ⇒ 693 windows), so this drops the last window position (the one whose kernel right edge would read `x_acc[699]`).

---

## Documented deviations (A7 + A8)

Two new entries appended to `docs/intentional-deviations.md`, following the existing template structure (used by #39 VIS stop-bit and #42 90° deadband):

### Entry 1 — A7

```markdown
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
```

### Entry 2 — A8

```markdown
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

The inline `// sync.c:86-90 resets to 44100 on retry exhaustion; we keep our last estimate (re-anchoring a near-locked input is harmful)` comment in `find_sync` is retained but adds a `(see docs/intentional-deviations.md)` reference.

---

## Tests (F2 + F3)

Three new tests in `src/sync.rs::tests`:

### F2.1 — `find_sync_corrects_0p5pct_slant_at_pd120`

(Post-implementation note: original drift target was 0.3%, but T6 found that drift falls within the 0.5°-quantized Hough deadband. Bumped to 0.5% — the minimum drift that reliably runs the correction path; comment + assertion semantics updated accordingly.)

```rust
/// Build a sync track that was *captured* at `capture_rate_hz` but
/// the true line cadence is at `true_rate_hz` — i.e. each
/// captured-line is `true_rate / capture_rate` of a real line, so
/// sync pulses drift through the (probe-stride-quantized) track.
fn synth_has_sync_slanted(spec: ModeSpec, true_rate_hz: f64, capture_rate_hz: f64) -> Vec<bool> {
    // Total length sized for the captured rate.
    let total = (f64::from(spec.image_lines) * spec.line_seconds * capture_rate_hz
        / (SYNC_PROBE_STRIDE as f64)) as usize + 16;
    let mut track = vec![false; total];
    for y in 0..spec.image_lines {
        // Sync pulse y starts at `y * line_seconds_true` (true cadence),
        // but the probe-index is computed at `capture_rate`.
        let line_start_t = f64::from(y) * spec.line_seconds * (true_rate_hz / capture_rate_hz);
        let line_end_t = line_start_t + spec.sync_seconds;
        let i_start = (line_start_t * capture_rate_hz / (SYNC_PROBE_STRIDE as f64)) as usize;
        let i_end = (line_end_t * capture_rate_hz / (SYNC_PROBE_STRIDE as f64)) as usize;
        for slot in track.iter_mut().take(i_end.min(total)).skip(i_start) {
            *slot = true;
        }
    }
    track
}

#[test]
fn find_sync_corrects_0p5pct_slant_at_pd120() {
    let spec = modespec::for_mode(SstvMode::Pd120);
    let true_rate = f64::from(WORKING_SAMPLE_RATE_HZ);
    // 0.5% drift produces a Hough peak well outside the (89°, 91°) lock
    // window; the retry loop must shrink the rate error toward zero.
    let capture_rate = true_rate * 1.005;
    let track = synth_has_sync_slanted(spec, true_rate, capture_rate);
    let r = find_sync(&track, capture_rate, spec);
    let err_pct = (r.adjusted_rate_hz - true_rate).abs() / true_rate * 100.0;
    let initial_err_pct = (capture_rate - true_rate).abs() / true_rate * 100.0;
    assert!(r.slant_deg.is_some(), "expected sync to be detected");
    assert!(
        r.adjusted_rate_hz < capture_rate,
        "correction should move rate toward true_rate"
    );
    assert!(
        err_pct < initial_err_pct / 2.0,
        "rate err {err_pct:.3}% should be < half of initial {initial_err_pct:.3}%"
    );
}
```

### F2.2 — `find_sync_corrects_1pct_slant_via_retries`

```rust
/// 1% capture-rate drift produces a Hough peak far outside the lock
/// window, forcing multiple retries. Verifies the retry loop
/// progresses (final rate closer to true than initial guess).
#[test]
fn find_sync_corrects_1pct_slant_via_retries() {
    let spec = modespec::for_mode(SstvMode::Pd120);
    let true_rate = f64::from(WORKING_SAMPLE_RATE_HZ);
    let capture_rate = true_rate * 1.01;  // 1% drift
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

### F3 — `find_sync_scottie_applies_skip_correction`

The audit's F3 asks for "a `find_sync` test for `SyncPosition::Scottie` (mid-line sync) — zero direct coverage today." The cleanest way to cover that path is to feed *line-start* pulses (the existing `synth_has_sync` helper) but with a Scottie spec — the test then asserts that `skip_correction_seconds()` is actually called by checking that the resulting `skip_samples` lands at the *known* correction offset, not at zero.

```rust
#[test]
fn find_sync_scottie_applies_skip_correction() {
    let spec = modespec::for_mode(SstvMode::Scottie1);
    let rate = f64::from(WORKING_SAMPLE_RATE_HZ);
    // Line-start pulses → raw `xmax`-derived s_secs ≈ 0 (small);
    // Scottie's skip_correction_seconds() subtracts `chan_len/2 -
    // 2*porch`, so the final skip should be that negative offset
    // (in samples), within a few-bin tolerance for xmax detection.
    let track = synth_has_sync(spec, rate);
    let r = find_sync(&track, rate, spec);

    let chan_len = f64::from(spec.line_pixels) * spec.pixel_seconds;
    let expected_secs = -chan_len / 2.0 + 2.0 * spec.porch_seconds;
    let expected_skip = (expected_secs * rate).round() as i64;
    let tolerance = (0.005 * rate) as i64;  // ~55 samples = ~5 ms

    assert!(
        (r.skip_samples - expected_skip).abs() < tolerance,
        "Scottie skip {} should be ≈ {expected_skip} (correction = {expected_secs:.4}s)",
        r.skip_samples
    );
}
```

Why this approach over constructing a "real" Scottie-positioned track:
- The slip-wrap (`xmax > X_ACC_SLIP_THRESHOLD`) interacts with where the Scottie sync pulse actually lands in the column-accumulator; getting the exact pulse-position math right would amount to re-deriving slowrx's Scottie layout, which is gold-plating for a test that just needs to *verify the correction path runs*.
- The end-to-end Scottie decode is already exercised by `tests/roundtrip.rs` (via `scottie_test_encoder`); the audit asked specifically for direct `find_sync` coverage of the `SyncPosition::Scottie` arm, which this test provides cheaply.
- If a future change accidentally breaks the Scottie branch (e.g., flips the sign of the correction), this test fails with a clear diagnostic; an end-to-end roundtrip would also fail but with a more diffuse cause.

---

## Out of scope (deferred to other epic #97 issues or follow-ups)

- C16 — inner-loop dead bounds check in `process()` in `src/resample.rs` (closed by #87's reviewer pass but explicitly left out of the #87 PR; covered separately in #96).
- B12 — broader `ModeSpec` refactoring (covered in #91).
- B14, C17-C19 — unrelated cleanup (covered in #96).
- The Hough quantization (0.5° step) — accepted as-is; not flagged in #88.

---

## File touch list

| File | Status | Role |
|------|--------|------|
| `src/sync.rs` | modify | Most of the change. Extract 3 helpers, name 3 consts, add 2 index closures, fix A6 off-by-one. Add 3 new tests + 2 helpers. |
| `src/modespec.rs` | modify | Add `pub(crate) fn skip_correction_seconds()`. |
| `docs/intentional-deviations.md` | modify | Append 2 new entries (A7 + A8). |
| `CHANGELOG.md` | modify | One bullet under `[Unreleased] ### Internal`. |

---

## Success criteria

- All 9 audit findings addressed (B2, B4, C2, C10, A6, A7, A8, F2, F3).
- Full CI gate green: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features --locked --release`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`.
- `find_sync` no longer needs `#[allow(too_many_lines)]`.
- Existing 7 tests in `src/sync.rs::tests` still pass.
- 3 new tests pass: `find_sync_corrects_0p5pct_slant_at_pd120`, `find_sync_corrects_1pct_slant_via_retries`, `find_sync_scottie_applies_skip_correction`.
- `tests/roundtrip.rs` 11/11 still passes (no regression in any synthetic round-trip).
- A6 fix produces an unchanged `xmax` for all existing tests (its semantic effect — dropping 2 right-edge window positions — only matters when a stronger falling edge sits at x ∈ {692, 693}, which no current test produces).
- A7 + A8 entries added to `docs/intentional-deviations.md`.
