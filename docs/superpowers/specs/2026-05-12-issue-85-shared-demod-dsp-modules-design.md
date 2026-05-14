# Issue #85 — Shared DSP / channel-demod module — design

**Issue:** [#85](https://github.com/jasonherald/slowrx.rs/issues/85) (audit bundle 1 of 12 — IDs B1, B8, B3, B5, B16, C6, C20)
**Source of record:** `docs/audits/2026-05-11-deep-code-review-audit.md`
**Scope:** the audit's highest-leverage cleanup — extract two new crate-private modules from material that's currently scattered across `mode_pd.rs`, `snr.rs`, `vis.rs`, `lib.rs`, `sync.rs`. Pure refactor: identical behavior, no public-API change.

## Background

The per-pixel demod heart lives in `src/mode_pd.rs` but is called from
`mode_robot.rs` and `mode_scottie.rs` — making the module graph dishonest
(`mode_robot` depends on `mode_pd` despite not being a PD mode). Generic DSP
primitives (Hann-window builder, complex-power computation, FFT-bin lookup,
Goertzel power) are duplicated across `vis.rs` / `snr.rs` / `sync.rs` /
`mode_pd.rs` with subtly different edge handling. The shared
`decode_one_channel_into` fn takes 11 arguments — five `#[allow(too_many_arguments)]`
sites — including a dead `chan_bounds_abs` parameter (#45 / audit B5) computed
by three caller-side `std::array::from_fn` blocks with ~8 wasted `.round()`s.
And the `ycbcr_to_rgb` fn is `pub` on a `pub mod`, accidentally part of the
stable API surface.

## Design

### Part 1 — Module layout (two new modules)

**`src/dsp.rs`** — generic DSP primitives, all `pub(crate)`:

- `fn build_hann(n: usize) -> Vec<f32>` — the canonical Hann-window builder.
  Adopts the `snr.rs` zero/one-safe variant (handles `n <= 1` gracefully via
  `n.saturating_sub(1).max(1)`) since the four current copies have subtly
  different edge handling.
- `fn power(c: Complex<f32>) -> f64` — `|c|²` as `f64`. Replaces the three
  inline-closure copies in `vis.rs`, `snr.rs`, `mode_pd.rs`.
- `fn get_bin(hz: f64, fft_len: usize, sample_rate_hz: u32) -> usize` — moved
  verbatim from `lib.rs` (`#[inline]`, slowrx-truncation semantics, the full
  doc-comment with the bin table is preserved).
- `fn goertzel_power(samples: &[f32], target_hz: f64) -> f64` — moved verbatim
  from `vis.rs`.

**`src/demod.rs`** — SSTV-specific per-channel demod, all `pub(crate)`:

- `struct ChannelDemod` — renamed from `mode_pd::PdDemod`. Fields unchanged:
  `fft: Arc<dyn Fft<f32>>`, `hann_bank: HannBank`, `fft_buf`, `scratch`. Methods
  unchanged: `new()`, `pixel_freq(&mut self, audio, center_sample, win_idx, hedr_shift_hz) -> f64`.
- `fn decode_one_channel_into(...)` — moved from `mode_pd.rs`, with the new
  shape (see Part 2).
- `fn freq_to_luminance(freq_hz: f64, hedr_shift_hz: f64) -> u8` — moved verbatim.
- `fn ycbcr_to_rgb(y: u8, cr: u8, cb: u8) -> [u8; 3]` — moved verbatim. **Visibility:**
  the audit (C6) called for `pub(crate)`, but the existing `pub use
  crate::mode_pd::ycbcr_to_rgb;` inside `pub mod __test_support` requires the source
  to be `pub` (rustc errors on re-exporting a `pub(crate)` item at a `pub` path —
  "private item in public interface"). So instead: keep it `pub` and add
  `#[doc(hidden)]` with a one-line comment ("not part of stable API; surfaced only
  via `__test_support` for integration tests"). Same intent — keep `ycbcr_to_rgb`
  off the documented public surface — with a mechanism that doesn't break
  `tests/roundtrip.rs`. The `__test_support::mode_pd::ycbcr_to_rgb` re-export then
  becomes `pub use crate::demod::ycbcr_to_rgb;` (the consumer-facing path stays
  stable). If `__test_support` ever stops needing it (audit B10, tracked in #86),
  this can drop to `pub(crate)` cleanly.
- `const FFT_LEN: usize = 1024;` — moved from `mode_pd.rs` (which currently aliases
  `crate::snr::FFT_LEN`). Becomes the canonical `crate::demod::FFT_LEN`; `snr.rs`
  and `mode_pd.rs` now import it from `demod`.
- `const SNR_REESTIMATE_STRIDE: i64 = 64;` — moved verbatim from `mode_pd.rs`.
- `struct HannBank` — moved from `snr.rs`. `[Vec<f32>; 7]` with `new()` and
  `get(i)`. `new()` calls `crate::dsp::build_hann` for each length.
- `const HANN_LENS: [usize; 7] = [12, 16, 24, 32, 64, 128, 256];` — moved
  from `snr.rs`.
- `fn window_idx_for_snr_with_hysteresis(snr_db: f64, prev_idx: usize) -> usize` —
  moved from `snr.rs`. (It's a pure mapping fn — given an SNR value, pick a
  Hann length index — which is per-pixel-demod logic, not SNR estimation.)
- `struct ChannelDecodeCtx<'a>` and `struct DemodState<'a>` — see Part 2.

**Slim-downs:**

| File | Change |
|------|--------|
| `src/mode_pd.rs` | Keeps `decode_pd_line_pair` + PD-specific scaffolding (line indexing, in-place RGB compose). Imports `crate::demod::*` for per-channel demod. The `chan_bounds_abs` array-builder block (`std::array::from_fn` of 4 entries with ~8 `.round()`s) is deleted; the 4 `decode_one_channel_into` calls take `&ctx, &mut state` instead. The doc-comment block that explained the (now-defunct) zero-pad rationale (audit E5) is removed. |
| `src/snr.rs` | Keeps only `SnrEstimator` + its 1024-sample SNR-analysis Hann + its tests. `HannBank` / `HANN_LENS` / `window_idx_for_snr_with_hysteresis` gone (moved to `demod`). Local `build_hann` gone (uses `crate::dsp::build_hann`). Local `power` closure gone (uses `crate::dsp::power`). |
| `src/vis.rs` | Local `build_hann_window` gone (uses `crate::dsp::build_hann`); `goertzel_power` gone (moved to `dsp`); local `power` closure gone (uses `crate::dsp::power`). |
| `src/sync.rs` | Local Hann builder gone (uses `crate::dsp::build_hann`); any local `power` closure replaced. |
| `src/mode_robot.rs`, `src/mode_scottie.rs` | Call-site paths flip `crate::mode_pd::decode_one_channel_into` → `crate::demod::decode_one_channel_into`; `crate::mode_pd::PdDemod` → `crate::demod::ChannelDemod`. The `chan_bounds_abs` array (3 entries) at each call site is deleted. |
| `src/decoder.rs` | Field `pd_demod: crate::mode_pd::PdDemod` → `channel_demod: crate::demod::ChannelDemod`. ~4 usages in `process` / `run_findsync_and_decode` flow through the rename. |
| `src/lib.rs` | Removes the inline `get_bin` (moved to `dsp.rs`); adds `pub(crate) mod demod;` and `pub(crate) mod dsp;`. The `__test_support::mode_pd::ycbcr_to_rgb` re-export points at `crate::demod::ycbcr_to_rgb`. |

### Part 2 — API shapes

**`ChannelDecodeCtx` and `DemodState`** (in `demod.rs`, audit B3):

```rust
/// Per-channel-decode-call invariants — these don't change between channels
/// of the same image. Bundled to cut `decode_one_channel_into`'s signature
/// from 11 args down to 5.
pub(crate) struct ChannelDecodeCtx<'a> {
    pub audio: &'a [f32],
    pub skip_samples: i64,
    pub rate_hz: f64,
    pub hedr_shift_hz: f64,
    pub spec: crate::modespec::ModeSpec,
}

/// Per-call mutable state: the channel demod's FFT + Hann bank, plus the
/// SNR estimator. Lifetime-only borrow; neither field is owned here.
pub(crate) struct DemodState<'a> {
    pub demod: &'a mut ChannelDemod,
    pub snr: &'a mut crate::snr::SnrEstimator,
}
```

**`decode_one_channel_into` — new 5-param signature:**

```rust
pub(crate) fn decode_one_channel_into(
    out: &mut [u8],
    chan_start_sec: f64,
    radio_frame_offset_seconds: f64, // renamed from `time_offset_seconds` (B16)
    ctx: &ChannelDecodeCtx<'_>,
    state: &mut DemodState<'_>,
) {
    // SAFETY of the f64→i64 / f64→usize casts below: every `.round() as i64`
    // / `as usize` in this fn computes a sample-buffer index. Out-of-range
    // values either saturate to i64::MAX/MIN (turning into out-of-bounds reads
    // that `audio.get(...)` resolves to 0.0 = silence) or are clamped by
    // explicit `.max(0)` / `.min(audio.len())` before indexing. Nothing
    // panics on an unexpected f64; the worst case is a black pixel. (C20.)

    // … existing body, with field accesses:
    //   ctx.audio / ctx.skip_samples / ctx.rate_hz / ctx.hedr_shift_hz / ctx.spec
    //   state.demod / state.snr
}
```

The fn-level `#[allow(..., clippy::too_many_arguments)]` loses `too_many_arguments`
(5 params, well under clippy's 7). The other `cast_*` allows stay — narrowing
per-statement would be noisier than the value gained.

**Call-site flattening:**

- `mode_pd::decode_pd_line_pair` — drops the `let chan_bounds_abs: [(i64, i64); 4]
  = std::array::from_fn(...)` block (~8 `.round()`s); each `decode_one_channel_into`
  call becomes `crate::demod::decode_one_channel_into(out, chan_start_sec,
  pair_seconds, &ctx, &mut state);` where `ctx` is built once before the channel
  loop and `state` re-borrows `&mut demod` / `&mut snr` per call. The stale
  doc-comment about `chan_bounds_abs` zero-padding (audit E5) is deleted.
- `mode_robot::decode_r36_r24_line`, `mode_robot::decode_r72_line`,
  `mode_scottie::decode_line` — same pattern, the `chan_bounds_abs` array (3
  entries each) and its `array::from_fn`/`.round()` cost vanish.

**`PdDemod` → `ChannelDemod` rename ripples** through ~7 sites:

- `src/decoder.rs`: field `pd_demod: PdDemod` → `channel_demod: ChannelDemod`;
  4 usages in `process` / `run_findsync_and_decode`.
- `src/mode_pd.rs`, `src/mode_robot.rs`, `src/mode_scottie.rs`: param types in
  the per-line decode fns.
- `src/mode_pd.rs::tests`: 5 `PdDemod::new()` constructions in unit tests →
  `ChannelDemod::new()`. The tests themselves move to `demod::tests`.

**`time_offset_seconds` → `radio_frame_offset_seconds`** (B16) — the param name
on `decode_one_channel_into`. The caller's local-variable name (`pair_seconds`
in PD, `line_seconds_offset` elsewhere) gets renamed where it improves clarity;
where the caller's local name reads better, the rename is just at the callee.

### Part 3 — Tests, docs, verification, out-of-scope

**Tests** — pure refactor, no new test logic; tests follow their code:

- `mode_pd::tests` splits in two. Tests for `freq_to_luminance` /
  `ycbcr_to_rgb` / `pixel_freq` / `decode_one_channel_into` (the 5
  `PdDemod::new()` sites) move to `demod::tests`. Tests for
  `decode_pd_line_pair` and PD line-indexing stay in `mode_pd::tests`. Imports
  update.
- `vis::tests::goertzel_*` (`empty_input_returns_zero_power`,
  `goertzel_handcomputed_quarter_cycle`) and any Hann-builder test move to
  `dsp::tests`.
- `lib.rs::tests_common::get_bin_matches_slowrx_truncation` moves to
  `dsp::tests`.
- Whatever tests `HannBank` / `window_idx_for_snr_with_hysteresis` had in
  `snr::tests` move to `demod::tests`. `SnrEstimator` tests stay in `snr::tests`.
- All integration tests (`tests/roundtrip.rs`, `tests/unknown_vis.rs`,
  `tests/multi_image.rs`, `tests/no_vis.rs`) pass byte-for-byte — they touch
  only the public API and the stable `__test_support` paths.
- **No new tests.** Identical behavior; existing coverage already exercises
  every moved item.

**Docs:**

- New module-level doc on `src/demod.rs` (one paragraph: what it owns, that
  `HannBank` / `HANN_LENS` / `window_idx_for_snr_with_hysteresis` moved here
  from `snr.rs` because they're per-pixel-demod machinery, not SNR estimation).
- New module-level doc on `src/dsp.rs` (one paragraph: generic primitives,
  Hann/power/get_bin/goertzel consolidated).
- Intra-doc-link rot fixes: `crate::mode_pd::PdDemod` /
  `crate::mode_pd::ycbcr_to_rgb` / `crate::mode_pd::FFT_LEN` / `crate::snr::HannBank`
  / `crate::snr::HANN_LENS` / `crate::snr::window_idx_for_snr_with_hysteresis`
  → their new `crate::demod::` homes (otherwise `cargo doc -D warnings` fails).
- `CHANGELOG.md` `[Unreleased]` — `### Internal` entry: *"Extract `crate::demod`
  (per-channel demod machinery) and `crate::dsp` (generic Hann / power / get_bin
  / Goertzel primitives) from `mode_pd.rs` / `snr.rs` / `vis.rs`; rename
  `PdDemod` → `ChannelDemod`; drop the dead `chan_bounds_abs` parameter from
  `decode_one_channel_into` and the 3 call-site `array::from_fn` blocks that
  fed it; rename `time_offset_seconds` → `radio_frame_offset_seconds`; mark
  `ycbcr_to_rgb` `#[doc(hidden)]`. (#85; audit B1/B3/B5/B8/B16/C6/C20.)"*

**Verification** — full local CI gate, with extra care given the wide blast radius:

- `cargo test --all-features --locked --release` — the big one;
  `tests/roundtrip.rs` exercises every mode's per-channel demod, so any
  regression surfaces there.
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`
- Sanity grep — after the refactor, this should print **nothing**:
  ```bash
  grep -rn "PdDemod\|mode_pd::ycbcr_to_rgb\|mode_pd::FFT_LEN\|mode_pd::decode_one_channel_into\|mode_pd::pixel_freq\|mode_pd::freq_to_luminance\|crate::snr::HannBank\|crate::snr::HANN_LENS\|crate::snr::window_idx_for_snr_with_hysteresis\|chan_bounds_abs\|time_offset_seconds\|fn build_hann_window\|fn goertzel_power" src/ tests/ examples/
  ```

**Out of scope** (deliberate — keeps this PR scoped):

- `trait ModeDecoder` consolidation of `decoder.rs`'s three near-identical
  match arms (audit B12) — separate, bigger refactor; tracked in #96.
- Moving the test encoders off the public API (`__test_support` cleanup) —
  audit B10, tracked in #86.
- Shared test-tone module for the encoders (audit B9) — also #86.
- Per-channel allocation hoisting (D3/D5/D6) — tracked in #93.
- `process` decomposition (audit B14) — tracked in #96.

## Files touched

`src/dsp.rs` (new), `src/demod.rs` (new), `src/mode_pd.rs`, `src/mode_robot.rs`,
`src/mode_scottie.rs`, `src/snr.rs`, `src/vis.rs`, `src/sync.rs`,
`src/decoder.rs`, `src/lib.rs`, `CHANGELOG.md`.
