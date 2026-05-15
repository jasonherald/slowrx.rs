# Issue #93 — Perf: hoist per-channel/per-line allocations into reusable scratch — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate per-decode allocation churn at three sites — D3 per-channel scratch on `ChannelDemod`, D5 `LineDecoded.pixels` drop + `current_image()` exposure (breaking API), D6 `DecodingState` capacity + `find_sync` scratch hoist + throwaway-black-image fix.

**Architecture:** Four sequential tasks. T1 lands `FindSyncScratch` and threads it through `find_sync` + helpers + `SstvDecoder` (D6.3). T2 hoists per-channel scratch onto `ChannelDemod` (D3). T3 is the API-breaking task: drops `LineDecoded.pixels`, adds `SstvDecoder::current_image()`, refactors `run_findsync_and_decode` to take `DecodingState` by value (D5 + D6.1 + D6.2), and migrates all consumers (CLI + 5 test files). T4 closes with CHANGELOG bullets (`### Changed` for D5 breaking, `### Internal` for the perf hoists) and the final gate.

**Tech Stack:** Rust 2021, MSRV 1.85. CI gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features --locked --release`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`. No GPG signing.

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-15-issue-93-perf-scratch-design.md`
- Audit: `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs D3, D5, D6)

**Release implication:** D5 is a breaking change. After this PR merges, the next release is **0.6.0** (the release PR is a separate `chore/release-0.6.0` per the established pattern). This PR's CHANGELOG entry goes under `[Unreleased]`.

---

## File Structure

| File | Status | Task |
|------|--------|------|
| `src/sync.rs` | modify | T1 (`FindSyncScratch` struct + `find_sync`/`hough_detect_slant`/`find_falling_edge` signature changes + tests update) |
| `src/decoder.rs` | modify | T1 (`SstvDecoder.find_sync_scratch` field) + T3 (`LineDecoded` shape, `current_image()`, `run_findsync_and_decode` by-value refactor, `DecodingState::with_capacity`, `out.reserve`) |
| `src/demod.rs` | modify | T2 (`ChannelDemod` scratch fields + `decode_one_channel_into` rewrites) |
| `src/mode_pd.rs` | modify (if PD scratches live here) | T2 (4× PD line-pair `vec![0u8; width]` hoists, location TBD-by-grep) |
| `src/bin/slowrx_cli.rs` | modify | T3 (consumer migration: drop `pixels` field from match arms; if used, call `decoder.current_image()`) |
| `tests/roundtrip.rs` | modify | T3 (consumer migration) |
| `tests/cli.rs` | modify (if uses `LineDecoded.pixels`) | T3 |
| `tests/multi_image.rs` | modify (if uses `LineDecoded.pixels`) | T3 |
| `tests/no_vis.rs` | modify (if uses `LineDecoded.pixels`) | T3 |
| `tests/unknown_vis.rs` | modify (if uses `LineDecoded.pixels`) | T3 |
| `CHANGELOG.md` | modify | T4 (`### Changed` for D5; `### Internal` for D3/D6) |

Task order: **T1** (`FindSyncScratch` plumbing) → **T2** (`ChannelDemod` scratch) → **T3** (D5 API change + `run_findsync_and_decode` refactor + `DecodingState::with_capacity` + consumer migration) → **T4** (CHANGELOG + final gate).

**Verification after each task:**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

Lib test counts at each checkpoint:
- **Pre-#93 baseline:** 136 lib tests.
- **Post-T1:** 136 (existing `find_sync_*` tests updated in place to construct local `FindSyncScratch`).
- **Post-T2:** 136 (pure refactor).
- **Post-T3:** 136 (API rename in match arms; no test count change).
- **Post-T4:** 136.

---

## Task 1: D6.3 — `FindSyncScratch` struct + plumbing

Hoist `find_sync`'s ~730 KB of per-call buffers (`sync_img`, `lines`, `x_acc`) onto a new `FindSyncScratch` struct owned by `SstvDecoder`. Thread `&mut scratch` through `find_sync` + helpers. Update the ~5 existing `find_sync_*` tests to construct a local scratch.

**Files:**
- Modify: `src/sync.rs` (new struct + signature changes + tests)
- Modify: `src/decoder.rs` (new field on `SstvDecoder` + `find_sync` call site)

- [ ] **Step 1: Add the `FindSyncScratch` struct in `src/sync.rs`**

In `src/sync.rs`, locate the end of the const block (around line 84, after `SYNC_EDGE_KERNEL_LEN`). Append:

```rust

/// Scratch buffers for [`find_sync`] and its helpers. Hoisted onto
/// [`crate::decoder::SstvDecoder`] so they're reused across decode
/// passes — the largest two (`sync_img`, `x_acc`) are sized at
/// construction and never resized; `lines` resizes per-call because
/// `n_slant_bins × LINES_D_BINS` depends on the mode's `line_width`.
/// (Audit #93 D6.)
pub(crate) struct FindSyncScratch {
    /// `[X_ACC_BINS × SYNC_IMG_Y_BINS]` flat buffer for the 2D sync image.
    /// Sized once; `find_sync` calls `.fill(false)` each invocation.
    pub(crate) sync_img: Vec<bool>,
    /// `[LINES_D_BINS × n_slant_bins]` flat buffer for the Hough
    /// accumulator. Resized per-call (`.clear() + .resize(...)`).
    pub(crate) lines: Vec<u16>,
    /// `[X_ACC_BINS]` column accumulator. Sized once; `.fill(0)` per call.
    pub(crate) x_acc: Vec<u32>,
}

impl FindSyncScratch {
    pub(crate) fn new() -> Self {
        Self {
            sync_img: vec![false; X_ACC_BINS * SYNC_IMG_Y_BINS],
            lines: Vec::new(),
            x_acc: vec![0; X_ACC_BINS],
        }
    }
}

impl Default for FindSyncScratch {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Update `find_sync` signature and body in `src/sync.rs`**

Locate `pub(crate) fn find_sync(has_sync: &[bool], initial_rate_hz: f64, spec: ModeSpec) -> SyncResult` (around line 225). Update the signature to add the scratch parameter:

```rust
#[must_use = "the SyncResult must be consumed; dropping it discards the slant + skip correction"]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
pub(crate) fn find_sync(
    has_sync: &[bool],
    initial_rate_hz: f64,
    spec: ModeSpec,
    scratch: &mut FindSyncScratch,
) -> SyncResult {
```

Inside the function body, find the calls to `hough_detect_slant` and `find_falling_edge`. Pass `scratch` through:

```rust
// Existing call (approximate; locate via grep on `hough_detect_slant`):
let Some((slant, adjusted)) = hough_detect_slant(has_sync, rate, spec, line_width) else {
    break;
};

// Updated:
let Some((slant, adjusted)) = hough_detect_slant(has_sync, rate, spec, line_width, scratch) else {
    break;
};

// Existing call (approximate; locate via grep on `find_falling_edge`):
let xmax = find_falling_edge(has_sync, rate, spec, num_lines);

// Updated:
let xmax = find_falling_edge(has_sync, rate, spec, num_lines, scratch);
```

- [ ] **Step 3: Update `hough_detect_slant` signature and body to use scratch**

In `src/sync.rs`, locate `fn hough_detect_slant(has_sync: &[bool], rate_hz: f64, spec: ModeSpec, line_width: usize) -> Option<(f64, f64)>`. Add the scratch parameter and use it for `sync_img` and `lines`:

```rust
fn hough_detect_slant(
    has_sync: &[bool],
    rate_hz: f64,
    spec: ModeSpec,
    line_width: usize,
    scratch: &mut FindSyncScratch,
) -> Option<(f64, f64)> {
    let n_slant_bins = ((MAX_SLANT_DEG - MIN_SLANT_DEG) / SLANT_STEP_DEG).round() as usize;
    let num_lines = spec.image_lines as usize;

    let sync_img_idx = |x: usize, y: usize| x * SYNC_IMG_Y_BINS + y;
    let lines_idx = |d: usize, q: usize| d * n_slant_bins + q;

    let probe_index = |t: f64| -> usize {
        let raw = t * rate_hz / (SYNC_PROBE_STRIDE as f64);
        if raw < 0.0 {
            0
        } else {
            raw as usize
        }
    };

    // Reset the hoisted scratch buffers (was: fresh Vec allocations).
    scratch.sync_img.fill(false);
    scratch.lines.clear();
    scratch.lines.resize(LINES_D_BINS * n_slant_bins, 0);

    // Draw the 2D sync signal at current rate.
    for y in 0..num_lines.min(SYNC_IMG_Y_BINS) {
        for x in 0..line_width.min(X_ACC_BINS) {
            let t = ((y as f64) + (x as f64) / (line_width as f64)) * spec.line_seconds;
            let idx = probe_index(t);
            if idx < has_sync.len() {
                scratch.sync_img[sync_img_idx(x, y)] = has_sync[idx];
            }
        }
    }

    // Linear Hough transform.
    let mut q_most = 0_usize;
    let mut max_count = 0_u16;
    for cy in 0..num_lines.min(SYNC_IMG_Y_BINS) {
        for cx in 0..line_width.min(X_ACC_BINS) {
            if !scratch.sync_img[sync_img_idx(cx, cy)] {
                continue;
            }
            for q in 0..n_slant_bins {
                let theta = deg2rad(MIN_SLANT_DEG + (q as f64) * SLANT_STEP_DEG);
                let d_signed = (line_width as f64)
                    + (-(cx as f64) * theta.sin() + (cy as f64) * theta.cos()).round();
                if d_signed > 0.0 && d_signed < (line_width as f64) {
                    let d = d_signed as usize;
                    if d < LINES_D_BINS {
                        let cell = &mut scratch.lines[lines_idx(d, q)];
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

Key changes from the existing body:
- The two local `let mut sync_img = vec![...]` / `let mut lines = vec![...]` are gone.
- `scratch.sync_img.fill(false)` + `scratch.lines.clear() + .resize(...)` at the top.
- All `sync_img[...]` references become `scratch.sync_img[...]`; all `lines[...]` become `scratch.lines[...]`.
- The function-level `#[allow(...)]` block (cast lints) stays.

- [ ] **Step 4: Update `find_falling_edge` signature and body to use scratch**

In `src/sync.rs`, locate `fn find_falling_edge(has_sync: &[bool], rate_hz: f64, spec: ModeSpec, num_lines: usize) -> i32`. Add the scratch parameter and use it for `x_acc`:

```rust
fn find_falling_edge(
    has_sync: &[bool],
    rate_hz: f64,
    spec: ModeSpec,
    num_lines: usize,
    scratch: &mut FindSyncScratch,
) -> i32 {
    let probe_index = |t: f64| -> usize {
        let raw = t * rate_hz / (SYNC_PROBE_STRIDE as f64);
        if raw < 0.0 {
            0
        } else {
            raw as usize
        }
    };

    scratch.x_acc.fill(0);
    for y in 0..num_lines {
        for (x, slot) in scratch.x_acc.iter_mut().enumerate() {
            let t = (y as f64) * spec.line_seconds
                + ((x as f64) / (X_ACC_BINS as f64)) * spec.line_seconds;
            let idx = probe_index(t);
            if idx < has_sync.len() && has_sync[idx] {
                *slot = slot.saturating_add(1);
            }
        }
    }

    falling_edge_from_x_acc(&scratch.x_acc)
}
```

Key changes:
- `let mut x_acc = vec![0u32; X_ACC_BINS]` is gone; `scratch.x_acc.fill(0)` replaces it.
- All `x_acc.iter_mut()` / `&x_acc` references become `scratch.x_acc.iter_mut()` / `&scratch.x_acc`.
- The function-level `#[allow(...)]` block stays.

`falling_edge_from_x_acc` (the pure helper that takes `&[u32]`) stays UNCHANGED — it's already a pure function over a borrowed slice.

- [ ] **Step 5: Add `find_sync_scratch` field to `SstvDecoder` in `src/decoder.rs`**

In `src/decoder.rs`, locate `pub struct SstvDecoder` (around line 160 — the field list). Add a new field:

```rust
pub struct SstvDecoder {
    resampler: Resampler,
    vis: crate::vis::VisDetector,
    channel_demod: crate::demod::ChannelDemod,
    snr_est: crate::snr::SnrEstimator,
    /// Scratch buffers for `find_sync` (sync_img / lines / x_acc).
    /// Hoisted here so they're reused across decode passes instead of
    /// being allocated fresh per call. (Audit #93 D6.)
    find_sync_scratch: crate::sync::FindSyncScratch,
    state: State,
    samples_processed: u64,
    working_samples_emitted: u64,
}
```

(The existing fields ordering may differ slightly — insert `find_sync_scratch` between `snr_est` and `state` so it groups with the other internal-state fields.)

Then update `SstvDecoder::new` (around line 247) to initialize it:

```rust
pub fn new(input_sample_rate_hz: u32) -> Result<Self> {
    Ok(Self {
        resampler: Resampler::new(input_sample_rate_hz)?,
        vis: crate::vis::VisDetector::new(IS_KNOWN_VIS),
        channel_demod: crate::demod::ChannelDemod::new(),
        snr_est: crate::snr::SnrEstimator::new(),
        find_sync_scratch: crate::sync::FindSyncScratch::new(),  // NEW
        state: State::AwaitingVis,
        samples_processed: 0,
        working_samples_emitted: 0,
    })
}
```

The existing `reset()` method (around line 600) does NOT need to recreate `find_sync_scratch` — the buffers are state-free (cleared at every `find_sync` call). Leave `reset()` untouched.

The hand-written `Debug for SstvDecoder` impl (added in #92) does NOT need to surface `find_sync_scratch` — it's pure scratch with no diagnostic value. The existing `.finish_non_exhaustive()` already signals "more fields exist."

- [ ] **Step 6: Update the `find_sync` call site in `src/decoder.rs::run_findsync_and_decode`**

In `src/decoder.rs::run_findsync_and_decode` (around line 478), update the function signature to accept a scratch parameter and pass it to `find_sync`:

```rust
fn run_findsync_and_decode(
    d: &mut DecodingState,
    channel_demod: &mut crate::demod::ChannelDemod,
    snr_est: &mut crate::snr::SnrEstimator,
    find_sync_scratch: &mut crate::sync::FindSyncScratch,  // NEW
    out: &mut Vec<SstvEvent>,
) {
    let work_rate = f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ);
    let result = find_sync(&d.has_sync, work_rate, d.spec, find_sync_scratch);  // pass scratch
    // ... rest of function unchanged ...
```

(T3 will further refactor this function's signature to take `d: DecodingState` by value; T1 keeps it as `&mut DecodingState` for now.)

Then update the `run_findsync_and_decode` call site in `SstvDecoder::process` (around line 396 area — the match arm that detects buffer-full and dispatches). Pass `&mut self.find_sync_scratch`:

```rust
Self::run_findsync_and_decode(
    d,
    &mut self.channel_demod,
    &mut self.snr_est,
    &mut self.find_sync_scratch,  // NEW
    &mut out,
);
```

- [ ] **Step 7: Update the ~5 `find_sync_*` tests in `src/sync.rs::tests`**

Run:
```bash
grep -n "find_sync(" /data/source/slowrx.rs/src/sync.rs | head -20
```

Identify the test sites (probably 5-6 calls in `mod tests`). For each `find_sync(&track, rate, spec)` call, insert a local scratch construction immediately above and pass `&mut scratch`:

```rust
// Before:
let r = find_sync(&track, rate, spec);

// After:
let mut scratch = FindSyncScratch::new();
let r = find_sync(&track, rate, spec, &mut scratch);
```

The existing test bodies otherwise stay identical — `FindSyncScratch::new()` is the same buffers that `find_sync` used to allocate internally, just owned by the test now.

Touch points (approximate; verify with grep):
- `find_sync_locks_clean_track_to_90_degrees`
- `find_sync_empty_track_has_no_slant_detected`
- `find_sync_recovers_known_offset`
- `find_sync_handles_empty_track`
- `find_sync_corrects_0p5pct_slant_at_pd120`
- `find_sync_corrects_1pct_slant_via_retries`
- `find_sync_scottie_applies_skip_correction`

That's ~7 test sites. The `falling_edge_from_x_acc_*` tests (added in #88) are unaffected — they exercise the pure helper that still takes `&[u32]` directly.

- [ ] **Step 8: Run the full gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136** (no new tests; existing tests update in place).

**Critical regression checks:**
- All ~7 `find_sync_*` tests pass with the new scratch parameter.
- `tests/roundtrip.rs` 11/11 (the perf hoist must not change pixel output).
- `find_sync_off_by_one_a6` regression guard still asserts the A6 fix (xmax=345 on the constructed `x_acc`).

If clippy fires `clippy::needless_pass_by_value` or `clippy::needless_pass_by_ref_mut` on the new `&mut scratch` parameters, the parameter IS used mutably — these lints should not fire. If they do, add a function-level `#[allow(...)]` at minimum scope or restructure the signature.

If `Debug for FindSyncScratch` is needed (it's not derived by default), the new struct won't print under `dbg!(scratch)`. That's fine — no caller `Debug`s it. The struct is `pub(crate)` so external rustdoc concerns don't apply.

- [ ] **Step 9: Commit**

```bash
git add src/sync.rs src/decoder.rs
git commit -m "perf(sync): D6.3 — FindSyncScratch hoisted onto SstvDecoder (#93)

Adds FindSyncScratch (sync_img/lines/x_acc buffers — ~730 KB) as a
field on SstvDecoder. find_sync, hough_detect_slant, find_falling_edge
gain a scratch: &mut FindSyncScratch parameter; the per-call vec! allocs
are replaced with .fill() / .clear()+.resize() on the hoisted buffers.

~7 existing find_sync_* tests update to construct a local
FindSyncScratch::new() and pass &mut scratch.

falling_edge_from_x_acc unchanged (already pure over &[u32]).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: D3 — Hoist per-channel scratch onto `ChannelDemod`

Add `pixel_times`, `stored_lum`, `scratch_audio` (and any PD-specific line-pair scratches) as fields on `ChannelDemod`. Rewrite the allocation sites in `decode_one_channel_into` to use them.

**Files:**
- Modify: `src/demod.rs`
- Modify (if PD scratches live here): `src/mode_pd.rs`

- [ ] **Step 1: Add scratch fields to `ChannelDemod` in `src/demod.rs`**

Locate `pub(crate) struct ChannelDemod` (around line 255). Add three scratch fields after the existing ones:

```rust
pub(crate) struct ChannelDemod {
    fft: Arc<dyn rustfft::Fft<f32>>,
    fft_buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    hann_bank: HannBank,
    /// Per-channel scratch hoisted out of `decode_one_channel_into`.
    /// Reused across calls via `clear() + reserve()/resize()/extend()`
    /// at the top of each invocation. (Audit #93 D3.)
    pixel_times: Vec<i64>,
    stored_lum: Vec<u8>,
    scratch_audio: Vec<f32>,
}
```

Update `ChannelDemod::new` (around line 261) to initialize them as empty:

```rust
pub fn new() -> Self {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_LEN);
    let scratch_len = fft.get_inplace_scratch_len();
    Self {
        fft,
        fft_buf: vec![Complex { re: 0.0, im: 0.0 }; FFT_LEN],
        // rustfft returns scratch_len = 0 for power-of-two sizes
        // (FFT_LEN=1024 is radix-2). (Audit #92 C8.)
        scratch: vec![Complex { re: 0.0, im: 0.0 }; scratch_len],
        hann_bank: HannBank::new(),
        pixel_times: Vec::new(),
        stored_lum: Vec::new(),
        scratch_audio: Vec::new(),
    }
}
```

- [ ] **Step 2: Rewrite the `pixel_times` allocation in `decode_one_channel_into`**

In `src/demod.rs`, locate the function `pub(crate) fn decode_one_channel_into(...)` (search for `fn decode_one_channel_into`). The function takes `&mut ChannelDemod` (likely as `&mut self` if it's a method, or as a `demod: &mut ChannelDemod` parameter). Verify which form via:

```bash
grep -n "fn decode_one_channel_into" /data/source/slowrx.rs/src/demod.rs
```

Inside the function body, locate (around line 493):

```rust
let mut pixel_times: Vec<i64> = Vec::with_capacity(width);
for x in 0..width {
    // ... compute abs ...
    pixel_times.push(abs);
}
```

Replace with (assuming `&mut self` form; adjust to `demod.pixel_times` if the function takes `demod: &mut ChannelDemod`):

```rust
self.pixel_times.clear();
self.pixel_times.reserve(width);
for x in 0..width {
    // ... compute abs ... (unchanged)
    self.pixel_times.push(abs);
}
```

All downstream references to the local `pixel_times` (e.g., `pixel_times[0]`, `pixel_times[width - 1]`, `pixel_times[x]`) become `self.pixel_times[...]`.

- [ ] **Step 3: Rewrite the `stored_lum` allocation**

In the same function, locate (around line 508):

```rust
let mut stored_lum = vec![0_u8; sweep_len];
```

Replace with:

```rust
self.stored_lum.clear();
self.stored_lum.resize(sweep_len, 0);
```

All downstream references to `stored_lum[idx]`, `stored_lum[rel as usize]`, `stored_lum.len()` become `self.stored_lum[...]` / `self.stored_lum.len()`.

- [ ] **Step 4: Rewrite the `scratch_audio` allocation**

In the same function, locate (around line 548):

```rust
let scratch_audio: Vec<f32> = (sweep_start..sweep_end).map(read_audio).collect();
```

Replace with:

```rust
self.scratch_audio.clear();
self.scratch_audio.extend((sweep_start..sweep_end).map(read_audio));
```

Downstream reference (around line 587) `&scratch_audio` becomes `&self.scratch_audio`.

- [ ] **Step 5: Locate and hoist the 4× PD line-pair `vec![0u8; width]` sites**

Run:

```bash
grep -n "vec!\[0u8" /data/source/slowrx.rs/src/demod.rs /data/source/slowrx.rs/src/mode_pd.rs 2>/dev/null
```

The audit references `mode_pd.rs:345-348, 423, 437, 479` for 4 buffers — these are per-PD-line-pair luma/chroma accumulators (likely something like `y_odd_lum`, `y_even_lum`, `cr_lum`, `cb_lum`).

**If the 4 sites are in `src/mode_pd.rs`** (a separate file from `src/demod.rs`):

Add 4 scratch fields to `ChannelDemod` (in `src/demod.rs`, alongside the three added in Step 1):

```rust
    /// PD line-pair scratch buffers — luma + 2 chroma + 2nd luma
    /// accumulators per line pair. Hoisted out of decode_pd_line_pair.
    /// (Audit #93 D3.)
    pd_y_odd: Vec<u8>,
    pd_y_even: Vec<u8>,
    pd_cr: Vec<u8>,
    pd_cb: Vec<u8>,
```

Update `ChannelDemod::new` to initialize them as empty (`Vec::new()`).

In `src/mode_pd.rs::decode_pd_line_pair`, replace the 4 `let mut <name> = vec![0u8; width];` (or equivalent) lines with `clear() + resize(width, 0)` calls on the corresponding `ChannelDemod` field. The exact variable names depend on the existing code — use whatever the existing locals are called (just prepend `self.` or `demod.` as the function takes `ChannelDemod`).

**If the 4 sites are inline in `src/demod.rs`** (e.g., merged into `decode_one_channel_into` or a sibling fn in the same file): hoist them the same way, just edit `src/demod.rs` only.

**If only some of the 4 exist post-#85** (the audit's line numbers are pre-extraction): hoist what's there, document the count in the commit message, mark any missing as "audit reference was pre-#85 extraction; verified absent in current code."

- [ ] **Step 6: Run the full gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **136** (pure refactor; no test changes).

**Critical regression checks:**
- `tests/roundtrip.rs` 11/11 — the per-channel scratch hoist must not change pixel output. Each `decode_one_channel_into` call must produce identical `stored_lum` content as before; the `clear() + reserve()/resize()` pattern guarantees this since the buffers are fully overwritten at each call.
- All existing per-mode tests in `modespec::tests` pass (unchanged).
- `find_sync_*` tests from T1 still pass.

If clippy fires `clippy::needless_pass_by_value` on the `&mut ChannelDemod` parameter (or `&mut self`), that's because the scratch fields are read across the function and the borrow checker is happy — no action needed.

If `clippy::cast_*` lints fire on any new arithmetic, the function already carries `#[allow(...)]` blocks from prior audits; extend if necessary.

- [ ] **Step 7: Commit**

```bash
git add src/demod.rs src/mode_pd.rs
git commit -m "perf(demod): D3 — hoist per-channel scratch onto ChannelDemod (#93)

Adds pixel_times / stored_lum / scratch_audio (+ 4× PD line-pair
luma/chroma scratches if present in current source) as fields on
ChannelDemod. The 3 per-channel + 4 per-pair Vec allocations inside
decode_one_channel_into and decode_pd_line_pair (~7000 allocs per PD240
image) are replaced with clear() + reserve()/resize()/extend() on the
hoisted buffers.

tests/roundtrip.rs 11/11 unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

(Adjust the commit body to reflect whether 4 PD scratches were hoisted, fewer, or none — based on what grep finds in Step 5.)

---

## Task 3: D5 + D6.1 + D6.2 — Drop `LineDecoded.pixels`, add `current_image()`, refactor `run_findsync_and_decode`, `DecodingState` `with_capacity`

The big task. Public API change + consumer migration + ownership refactor + capacity hint.

**Files:**
- Modify: `src/decoder.rs` (the bulk)
- Modify: `src/bin/slowrx_cli.rs` (consumer migration)
- Modify: `tests/roundtrip.rs`, `tests/cli.rs`, `tests/multi_image.rs`, `tests/no_vis.rs`, `tests/unknown_vis.rs` (scan + update if they touch `LineDecoded.pixels`)

- [ ] **Step 1: Drop `pixels` from `SstvEvent::LineDecoded` in `src/decoder.rs`**

Locate `pub enum SstvEvent` (around line 17). Current `LineDecoded` variant (around line 68):

```rust
    LineDecoded {
        mode: SstvMode,
        line_index: u32,
        pixels: Vec<[u8; 3]>,
    },
```

Replace with:

```rust
    /// One image row has been fully decoded. Pixel data lives on the
    /// decoder's in-progress image; call
    /// [`SstvDecoder::current_image`] to borrow it.
    /// Row `line_index` is fully populated after this event fires.
    /// (Audit #93 D5.)
    LineDecoded {
        mode: SstvMode,
        line_index: u32,
    },
```

The doc-comment above the variant changes to point at `current_image()`. (The exact pre-edit doc text may vary; preserve whatever rationale was there and append the new framing.)

- [ ] **Step 2: Add `SstvDecoder::current_image()` method in `src/decoder.rs`**

Locate the `impl SstvDecoder` block. After `SstvDecoder::new` and before `pub fn process` (or anywhere inside the impl that fits the existing layout), add:

```rust
    /// Borrow the in-progress image. Returns `Some(&image)` while the
    /// decoder is in the `Decoding` state (after a `VisDetected` event
    /// and before the `ImageComplete` event); `None` while
    /// `AwaitingVis`. Row `N` is fully populated after the
    /// `LineDecoded { line_index: N, .. }` event for that row has been
    /// emitted by the most recent [`process`] call. (Audit #93 D5.)
    ///
    /// [`process`]: SstvDecoder::process
    #[must_use]
    pub fn current_image(&self) -> Option<&SstvImage> {
        match &self.state {
            State::Decoding(d) => Some(&d.image),
            State::AwaitingVis => None,
        }
    }
```

- [ ] **Step 3: Update `DecodingState` construction to use `with_capacity` (D6.1)**

In `src/decoder.rs::process`, locate the State::Decoding construction (around line 326 — the spot where `target_audio_samples: target` is set). Update the `audio` and `has_sync` fields:

```rust
// Before:
audio: Vec::new(),
has_sync: Vec::new(),
target_audio_samples: target,

// After:
audio: Vec::with_capacity(target),
has_sync: Vec::with_capacity(target / crate::sync::SYNC_PROBE_STRIDE),
target_audio_samples: target,
```

(If `target` is computed slightly differently or named differently in the actual code, use whatever local variable holds the value — the principle is "use the known target instead of letting the Vec grow from 0.")

- [ ] **Step 4: Refactor `run_findsync_and_decode` to take `DecodingState` by value (D6.2)**

In `src/decoder.rs`, locate `fn run_findsync_and_decode(d: &mut DecodingState, ...)` (around line 478). Change the signature to take `d` by value:

```rust
fn run_findsync_and_decode(
    mut d: DecodingState,
    channel_demod: &mut crate::demod::ChannelDemod,
    snr_est: &mut crate::snr::SnrEstimator,
    find_sync_scratch: &mut crate::sync::FindSyncScratch,
    out: &mut Vec<SstvEvent>,
) {
    let work_rate = f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ);
    let result = find_sync(&d.has_sync, work_rate, d.spec, find_sync_scratch);
    let rate = result.adjusted_rate_hz;
    let skip = result.skip_samples;

    // Image-complete burst: image_lines LineDecoded events + 1 ImageComplete.
    // Pre-reserve to avoid ~9 Vec growth reallocs. (Audit #93 D5.)
    out.reserve(d.spec.image_lines as usize + 1);

    let line_pixels = d.spec.line_pixels as usize;
    match d.spec.channel_layout {
        crate::modespec::ChannelLayout::PdYcbcr => {
            let pair_count = d.spec.image_lines / 2;
            for pair in 0..pair_count {
                let pair_seconds = f64::from(pair) * d.spec.line_seconds;
                crate::mode_pd::decode_pd_line_pair(
                    d.spec,
                    pair,
                    &d.audio,
                    skip,
                    pair_seconds,
                    rate,
                    &mut d.image,
                    channel_demod,
                    snr_est,
                    d.hedr_shift_hz,
                );
                let row0 = pair * 2;
                let row1 = row0 + 1;
                for r in [row0, row1] {
                    out.push(SstvEvent::LineDecoded {
                        mode: d.mode,
                        line_index: r,
                    });
                }
            }
        }
        crate::modespec::ChannelLayout::RobotYuv => {
            for line in 0..d.spec.image_lines {
                let line_seconds_offset = f64::from(line) * d.spec.line_seconds;
                crate::mode_robot::decode_line(
                    d.spec,
                    d.mode,
                    line,
                    &d.audio,
                    skip,
                    line_seconds_offset,
                    rate,
                    &mut d.image,
                    d.chroma_planes.as_mut(),
                    channel_demod,
                    snr_est,
                    d.hedr_shift_hz,
                );
                out.push(SstvEvent::LineDecoded {
                    mode: d.mode,
                    line_index: line,
                });
            }
        }
        crate::modespec::ChannelLayout::RgbSequential => {
            for line in 0..d.spec.image_lines {
                let line_seconds_offset = f64::from(line) * d.spec.line_seconds;
                crate::mode_scottie::decode_line(
                    d.spec,
                    d.mode,
                    line,
                    &d.audio,
                    skip,
                    line_seconds_offset,
                    rate,
                    &mut d.image,
                    channel_demod,
                    snr_est,
                    d.hedr_shift_hz,
                );
                out.push(SstvEvent::LineDecoded {
                    mode: d.mode,
                    line_index: line,
                });
            }
        }
    }

    // Move the now-populated image into the ImageComplete event.
    // No more mem::replace + fresh black SstvImage allocation. (Audit #93 D6.2.)
    out.push(SstvEvent::ImageComplete {
        image: d.image,
        partial: false,
    });
}
```

Key changes:
- Signature: `d: &mut DecodingState` → `mut d: DecodingState` (by value).
- New `find_sync_scratch: &mut FindSyncScratch` parameter (already added in T1 Step 6).
- `out.reserve(d.spec.image_lines as usize + 1)` near the top.
- Each `out.push(SstvEvent::LineDecoded { ..., pixels: d.image.pixels[start..end].to_vec() })` drops the `pixels:` field and the slice-clone.
- The `let row0/row1 ... let start ... let end` locals for PD are gone (no slice needed).
- The closing `mem::replace + SstvImage::new(...)` block is REPLACED with a direct `out.push(SstvEvent::ImageComplete { image: d.image, partial: false })` — moves `d.image` into the event.

- [ ] **Step 5: Update the `run_findsync_and_decode` call site in `SstvDecoder::process`**

In `src/decoder.rs::process`, locate where `run_findsync_and_decode` is invoked (the burst-emit transition; around the `audio.len() >= target_audio_samples` branch). Currently looks like:

```rust
// Before (approximate):
match &mut self.state {
    State::Decoding(d) if d.audio.len() >= d.target_audio_samples => {
        Self::run_findsync_and_decode(d, &mut self.channel_demod, &mut self.snr_est, &mut self.find_sync_scratch, &mut out);
        self.state = State::AwaitingVis;
    }
    // ... other arms ...
}
```

Refactor to swap `self.state` BEFORE calling, so we own `d` by value:

```rust
// After:
let buffer_full = matches!(
    &self.state,
    State::Decoding(d) if d.audio.len() >= d.target_audio_samples
);
if buffer_full {
    let State::Decoding(d) = std::mem::replace(&mut self.state, State::AwaitingVis) else {
        unreachable!("matches!() guard ensures Decoding");
    };
    Self::run_findsync_and_decode(
        *d,  // unbox; pass DecodingState by value
        &mut self.channel_demod,
        &mut self.snr_est,
        &mut self.find_sync_scratch,
        &mut out,
    );
}
```

The exact location and surrounding match-arm structure in `process()` will vary — the implementer locates the existing `Self::run_findsync_and_decode(d, ...)` call and refactors the surrounding control flow to extract `d` by value via `std::mem::replace`. The key invariants:
1. `self.state` ends up at `State::AwaitingVis` after the call.
2. `d` is moved into `run_findsync_and_decode` by value (no `&mut`).
3. No other code in `process()` references `self.state` between the `mem::replace` and the `run_findsync_and_decode` call.

If the existing `process()` uses a complex `match &mut self.state` block that makes the by-value extraction awkward, an alternative structure: do the buffer-fullness check first, then `std::mem::replace` only when it fires. Either approach is correct.

- [ ] **Step 6: Migrate `src/bin/slowrx_cli.rs` consumers**

Run:
```bash
grep -n "LineDecoded\|\.pixels" /data/source/slowrx.rs/src/bin/slowrx_cli.rs
```

For each `SstvEvent::LineDecoded { mode, line_index, pixels }` match arm, drop the `pixels` field:

```rust
// Before:
SstvEvent::LineDecoded { mode, line_index, pixels } => {
    // ... uses pixels ...
}

// After:
SstvEvent::LineDecoded { mode, line_index } => {
    // ... if pixels was used, replace with:
    // let img = decoder.current_image().expect("LineDecoded fires only during Decoding");
    // let start = (line_index as usize) * (img.width as usize);
    // let end = start + (img.width as usize);
    // let pixels = &img.pixels[start..end];
    // ... uses pixels (now a slice) ...
}
```

If the CLI doesn't actually use the pixels (just counts events or logs `line_index`), the match-arm pattern change is the only edit needed.

- [ ] **Step 7: Migrate test consumers in `tests/*.rs`**

Run:
```bash
grep -rn "LineDecoded\|\.pixels" /data/source/slowrx.rs/tests/
```

For each match arm or destructure, apply the same migration as Step 6. Most likely candidates:
- `tests/roundtrip.rs` — exercises the full decode pipeline; if it inspects per-line pixels it'll need `current_image()` migration. If it only checks the final `ImageComplete.image.pixels`, no change needed.
- `tests/multi_image.rs`, `tests/cli.rs`, `tests/no_vis.rs`, `tests/unknown_vis.rs` — most likely just count events; pattern-only update.

For tests inside `src/decoder.rs::tests` (if any match `LineDecoded`), apply the same migration.

- [ ] **Step 8: Add a `current_image()` regression test in `src/decoder.rs::tests`**

In `src/decoder.rs::tests`, append a small regression test:

```rust
    #[test]
    fn current_image_is_none_when_awaiting_vis() {
        let decoder = SstvDecoder::new(44100).expect("rate ok");
        assert!(decoder.current_image().is_none());
    }
```

(One test, no decode required — exercises the new API surface. The "Some(image) during Decoding" case is implicitly covered by `tests/roundtrip.rs` if any test inspects `current_image()` between line events. The spec marked that broader test as Optional; we skip it.)

This is the only net new test in this PR. Lib count: **136 → 137**.

Update the lib test count expectation in T4 to reflect 137 (the spec's success-criteria said 136→136; this small addition is a deviation worth documenting in the CHANGELOG bullet).

- [ ] **Step 9: Run the full gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected lib test count: **137** (136 + 1 new `current_image_is_none_when_awaiting_vis`).

**Critical regression checks:**
- `tests/roundtrip.rs` 11/11 — the pixel output across all 11 modes must be identical before and after the refactor.
- `tests/cli.rs` + `tests/multi_image.rs` + `tests/no_vis.rs` + `tests/unknown_vis.rs` all green.
- `LineDecoded` match arms in all test files compile (no `pixels` field references remaining).

If clippy fires `clippy::unused_async` or `clippy::needless_lifetimes` on the refactored `process()`, fix at minimum scope. If `clippy::let_underscore_must_use` or related fires on the `_ = std::mem::replace(...)` (it shouldn't — `mem::replace` returns the old value and we destructure it via `let State::Decoding(d) = ...`), confirm the pattern compiles cleanly.

If `RUSTDOCFLAGS="-D warnings"` fires `rustdoc::broken_intra_doc_links` on the new `[`process`]: SstvDecoder::process` doc link, the verbose `[name]: path` form should resolve. If it doesn't, switch to plain `\`process\`` code-span.

- [ ] **Step 10: Commit**

```bash
git add src/decoder.rs src/bin/slowrx_cli.rs tests/
git commit -m "feat(decoder)!: D5 + D6.1 + D6.2 — drop LineDecoded.pixels; current_image(); refactor (#93)

BREAKING CHANGE: SstvEvent::LineDecoded no longer carries
pixels: Vec<[u8; 3]>. Consumers call SstvDecoder::current_image()
to borrow the in-progress image instead. (Audit #93 D5.)

Also:
- New SstvDecoder::current_image() -> Option<&SstvImage> method.
- run_findsync_and_decode refactored to take DecodingState by value;
  d.image moves directly into the ImageComplete event, no more
  mem::replace + fresh SstvImage::new black image alloc. (Audit D6.2)
- DecodingState audio/has_sync use with_capacity(target). (Audit D6.1)
- out.reserve(image_lines + 1) before the burst emit.

Consumer migration: slowrx_cli + ~5 test files updated to drop the
pixels field from match arms (or migrate to current_image() if used).

Net new test: current_image_is_none_when_awaiting_vis. Lib 136 → 137.
tests/roundtrip.rs 11/11 unchanged — pixel output identical.

Release implication: requires 0.6.0 minor bump at release time.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

(The `!` after `feat(decoder)` is conventional-commits notation for a breaking change.)

---

## Task 4: CHANGELOG + final gate

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add CHANGELOG entries**

Open `CHANGELOG.md`. Under `## [Unreleased]`, prepend a new `### Changed` section (above the existing `### Internal`), and add a new perf bullet to `### Internal`:

```markdown
## [Unreleased]

### Changed

- **`SstvEvent::LineDecoded` no longer carries `pixels: Vec<[u8; 3]>`.**
  Consumers call [`SstvDecoder::current_image()`] (new) to borrow the
  in-progress image instead. Row `N` is fully populated after the
  `LineDecoded { line_index: N, .. }` event fires. **Breaking change** —
  bumps the next release to **0.6.0**. The migration is one-line per
  call site: drop the `pixels` field from the match-arm pattern; if
  the pixel data was being used, fetch it via
  `decoder.current_image().map(|img| &img.pixels[start..end])`.
  Eliminates ~496 per-line `.to_vec()` allocations per PD240 image.
  (#93; audit D5.)

  [`SstvDecoder::current_image()`]: https://docs.rs/slowrx/0.6.0/slowrx/struct.SstvDecoder.html#method.current_image

### Internal

- **Performance: hoist per-channel/per-line allocations into reusable scratch**
  (audit bundle 9 of 12). Three hot allocation sites moved out of the
  decode hot path: (1) per-channel scratch (`pixel_times`, `stored_lum`,
  `scratch_audio`, plus PD line-pair luma/chroma temps) now lives on
  `ChannelDemod` and is reused via `clear()` + `reserve()`/`resize()`/
  `extend()` — ~7000 allocations per PD240 image eliminated (audit D3);
  (2) `find_sync`'s `sync_img` / `lines` / `x_acc` buffers (~730 KB)
  moved to a new `pub(crate) FindSyncScratch` struct owned by
  `SstvDecoder`; `find_sync` + `hough_detect_slant` + `find_falling_edge`
  gain a `scratch: &mut FindSyncScratch` parameter; ~5 existing
  `find_sync_*` tests construct `FindSyncScratch::new()` locally
  (audit D6.3); (3) `DecodingState.audio` / `.has_sync` use
  `with_capacity(target_audio_samples)` instead of `Vec::new()`
  (audit D6.1); (4) `run_findsync_and_decode` now takes
  `DecodingState` by value and moves `d.image` directly into the
  `ImageComplete` event — eliminates the throwaway ~950 KB black-image
  `mem::replace` workaround (audit D6.2). All hot-loop allocation
  patterns become "0 allocs after first decode" instead of growing
  with image size. Plus `out.reserve(image_lines + 1)` before the
  burst-emit loop saves ~9 `Vec` growth reallocs. `tests/roundtrip.rs`
  11/11 unchanged — pixel output is identical before and after.
  (#93; audit D3/D6.)

- **API hygiene sweep** — ... [existing #92 bullet stays as-is below]

## [0.5.3] — 2026-05-14
...
```

(The `### Changed` subsection goes ABOVE `### Internal`. This matches Keep a Changelog ordering: `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`, then project-specific groupings like `Internal`.)

If the `[Unreleased]` section currently has only `### Internal`, the `### Changed` is a new subsection. If it has other subsections, slot `### Changed` in the canonical order.

**Note on the docs.rs URL:** the `0.6.0` URL won't resolve until the release lands and crates.io publishes. Acceptable — the link will work post-release. If preferred, drop the URL footnote and inline-format the link as plain `\`SstvDecoder::current_image()\`` (code-span without URL).

- [ ] **Step 2: Run the full CI gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected test counts:
- lib: **137** (136 pre-#93 + 1 new `current_image_is_none_when_awaiting_vis`).
- `tests/roundtrip.rs`: 11/11 (unchanged).
- All other integration tests: unchanged.
- Doc clean.

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(refactor): CHANGELOG for the perf scratch hoisting (#93)

Both ### Changed (D5 breaking — drop LineDecoded.pixels, add
current_image, requires 0.6.0) and ### Internal (D3 + D6 perf hoists)
entries. Next release: 0.6.0.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-review notes (for the implementer / reviewers)

- **Spec coverage:**
  - D3 (per-channel scratch hoist) → T2.
  - D5 (drop `LineDecoded.pixels`, add `current_image()`, pre-`reserve`) → T3 Steps 1, 2, 4 (the `out.reserve` line).
  - D6.1 (`DecodingState::with_capacity`) → T3 Step 3.
  - D6.2 (refactor `run_findsync_and_decode` by-value `d`; move `d.image` into event) → T3 Steps 4-5.
  - D6.3 (`FindSyncScratch` + plumb through `find_sync` + helpers + `SstvDecoder`) → T1.
  - CHANGELOG → T4.

- **Behavior preservation guarantees:**
  - All 3 scratch hoists are pure refactor — the buffers are fully overwritten at each use (clear() + reserve()/resize()/fill() pattern), so pixel output is bit-identical. `tests/roundtrip.rs` 11/11 is the load-bearing regression check.
  - The throwaway black-image fix (D6.2) doesn't change the `ImageComplete.image` content — same image, just no transient black allocation in the swap.
  - The `LineDecoded.pixels` removal (D5) doesn't change WHAT events fire or in what order — just removes a redundant data field. Consumers reading `current_image()` after `LineDecoded { line_index: N }` see the same pixels they used to get in the event.

- **TDD red moments:**
  - T3 Step 8 adds the only net new test (`current_image_is_none_when_awaiting_vis`). It's a "green from the start" structural test — `current_image()` exists after T3 Step 2; the test verifies the AwaitingVis branch returns None.
  - No deliberately-red tests in this PR; all changes are refactors with the existing test suite as the regression net.

- **Compile-time gating:**
  - Adding a field to `SstvEvent::LineDecoded` was historically protected by `#[non_exhaustive]` on the enum — verify that's still the case. If `SstvEvent` is `#[non_exhaustive]`, removing the `pixels` field is technically still a breaking change for callers that destructure (the match-arm pattern `LineDecoded { mode, line_index, pixels }` becomes a compile error).
  - Adding a new `pub fn current_image()` method on `SstvDecoder` is non-breaking (additive).

- **Out of scope** (tracked elsewhere):
  - SIMD'ifying the SNR power-sum (#77).
  - Sink/callback API for `process` (deferred — V2 milestone).
  - `impl Iterator` for events (deferred).

- **Release flow:** This PR's CHANGELOG entry stays under `[Unreleased]`. The next release is a separate `chore/release-0.6.0` PR (per the established pattern from 0.5.3) that:
  1. Bumps `Cargo.toml` 0.5.3 → 0.6.0.
  2. Changes `[Unreleased]` → `[0.6.0] — YYYY-MM-DD` in CHANGELOG.
  3. Adds a fresh empty `[Unreleased]` above.
  4. Tags `v0.6.0` post-merge.
  5. Publishes to crates.io.
