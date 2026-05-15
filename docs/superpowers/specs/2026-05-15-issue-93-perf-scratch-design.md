# Issue #93 — Perf: hoist per-channel/per-line allocations into reusable scratch — Design

**Issue:** [#93](https://github.com/jasonherald/slowrx.rs/issues/93) (audit bundle 9 of 12 — IDs D3, D5, D6).

**Scope:** eliminate per-decode allocation churn at three sites — per-channel scratch (D3, ~7000 allocs per PD240 image), the events Vec + per-line pixel clones (D5), and the `DecodingState` capacity + `find_sync` scratch + throwaway black-image workaround (D6). One **public-API-breaking** change: drop `pixels: Vec<[u8; 3]>` from `SstvEvent::LineDecoded` and expose `SstvDecoder::current_image() -> Option<&SstvImage>` for callers that want the in-progress row data. This is a **0.6.0 minor bump** (the audit-cleanup-wave releases 0.5.1/0.5.2/0.5.3 were all non-breaking; #93 is the first since 0.5.0 to touch the public surface).

---

## Background — audit findings

- **D3 — per-channel `Vec` churn.** `decode_one_channel_into` (the shared per-channel path used by PD / Robot / Scottie via `crate::demod`) allocates `pixel_times`, `stored_lum`, and `scratch_audio` fresh per call. Plus 4× `vec![0u8; width]` per PD line pair somewhere in the PD-specific code path. At PD240 (~496 lines × 4 channels) that's roughly 7000 allocations per decoded image.
- **D5 — `LineDecoded.pixels` and the events Vec.** `LineDecoded` currently carries `pixels: Vec<[u8; 3]>` cloned from `d.image.pixels[start..end]`. For PD240 that's ~496 small allocations (~1920 B each, ~950 KB total in tiny allocs) duplicating data already in `d.image.pixels`. The events Vec itself grows from 0 to `image_lines + 1` entries, requiring ~9 reallocs.
- **D6 — `DecodingState` capacity, throwaway black image, `find_sync` scratch.** Three sub-items:
  - `DecodingState.audio` and `.has_sync` start at `Vec::new()` despite `target_audio_samples` being known at construction time.
  - `run_findsync_and_decode` does `mem::replace(&mut d.image, SstvImage::new(...))` — allocating a fresh ~950 KB black image just to satisfy the borrow checker — so the real image can be moved into `ImageComplete`.
  - `find_sync` (with its three helpers `hough_detect_slant`, `find_falling_edge`, `falling_edge_from_x_acc`) allocates ~730 KB of `sync_img` / `lines` / `x_acc` buffers per call.

---

## Architecture

### D3 — Hoist channel scratch onto `ChannelDemod`

`ChannelDemod` is already per-`SstvDecoder` and is the shared owner of FFT plans + Hann bank for the per-channel demod path. Adding three scratch fields keeps ownership local and avoids a new top-level struct.

```rust
// src/demod.rs
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

`ChannelDemod::new` initializes them as `Vec::new()` — they grow once on the first decode and stay sized for the lifetime of the `SstvDecoder`.

`decode_one_channel_into` rewrites three current allocation sites to operate on `&mut self.{pixel_times, stored_lum, scratch_audio}`:

```rust
// Before (src/demod.rs:493-498):
let mut pixel_times: Vec<i64> = Vec::with_capacity(width);
for x in 0..width {
    let abs = ...;
    pixel_times.push(abs);
}

// After:
self.pixel_times.clear();
self.pixel_times.reserve(width);
for x in 0..width {
    let abs = ...;
    self.pixel_times.push(abs);
}
```

```rust
// Before (src/demod.rs:508):
let mut stored_lum = vec![0_u8; sweep_len];

// After:
self.stored_lum.clear();
self.stored_lum.resize(sweep_len, 0);
```

```rust
// Before (src/demod.rs:548):
let scratch_audio: Vec<f32> = (sweep_start..sweep_end).map(read_audio).collect();

// After:
self.scratch_audio.clear();
self.scratch_audio.extend((sweep_start..sweep_end).map(read_audio));
```

Downstream references to the local bindings (`&pixel_times`, `&stored_lum`, `&scratch_audio` in subsequent lines of the function) become `&self.pixel_times`, `&self.stored_lum`, `&self.scratch_audio`.

**4× `vec![0u8; width]` per PD line pair.** The audit references `mode_pd.rs:345-348, 423, 437, 479` — those 4 buffers are PD-specific (luma + chroma temp accumulators per line pair). They live in either `src/demod.rs::decode_pd_line_pair` or `src/mode_pd.rs` (depending on the post-#85 layout — the implementer locates them via `grep -n "vec!\[0u8" src/`). If they cluster in `decode_pd_line_pair`, hoist them as 4 more fields on `ChannelDemod` (next to the three above) — `pd_line_*_scratch` — with the same `clear()+resize()` pattern at the top of each line-pair decode.

### D5 — Drop `LineDecoded.pixels`; expose `current_image()`

**Public API change** (breaking — 0.6.0 minor bump):

```rust
// src/decoder.rs — SstvEvent enum
pub enum SstvEvent {
    /// VIS detected; mode resolved. Decoder is now in `Decoding` state.
    VisDetected { mode: SstvMode, sample_offset: u64 },
    /// Unknown VIS code received; decoder reset to `AwaitingVis`.
    UnknownVis { code: u8, sample_offset: u64 },
    /// One image row has been fully decoded. The pixel data lives on
    /// the decoder's in-progress image; call
    /// [`SstvDecoder::current_image`] to borrow it.
    LineDecoded {
        mode: SstvMode,
        line_index: u32,
        // `pixels: Vec<[u8; 3]>` is REMOVED (audit #93 D5). Consumers
        // call `decoder.current_image()` to borrow the row data.
    },
    /// Image complete; the owned image is in this event.
    ImageComplete { image: SstvImage, partial: bool },
}
```

**New method on `SstvDecoder`:**

```rust
impl SstvDecoder {
    /// Borrow the in-progress image. Returns `Some(&image)` while the
    /// decoder is in the `Decoding` state (after a `VisDetected` event
    /// and before the `ImageComplete` event); `None` while
    /// `AwaitingVis`. Row `N` is fully populated after the
    /// `LineDecoded { line_index: N, .. }` event for that row has been
    /// emitted by the most recent `process()` call. (audit #93 D5)
    #[must_use]
    pub fn current_image(&self) -> Option<&SstvImage> {
        match &self.state {
            State::Decoding(d) => Some(&d.image),
            State::AwaitingVis => None,
        }
    }
}
```

**Pre-`reserve` the events Vec at the burst-emit transition.** Inside `run_findsync_and_decode` (now taking `d: DecodingState` by value per D6):

```rust
// Image-complete burst: image_lines `LineDecoded` events + 1
// `ImageComplete`. Pre-reserve to avoid ~9 Vec growth reallocs.
out.reserve(d.spec.image_lines as usize + 1);
```

**Consumer migration:**
- The 3 `out.push(SstvEvent::LineDecoded { ..., pixels: d.image.pixels[start..end].to_vec() })` sites in `run_findsync_and_decode` (one per `ChannelLayout` arm — PD, Robot, Scottie) drop the `pixels:` field and the `.to_vec()`.
- `src/bin/slowrx_cli.rs` — scan for `LineDecoded` matches and `.pixels` field access; update to call `decoder.current_image().map(|img| &img.pixels[start..end])` where appropriate. If the CLI only matches the variant shape (no `pixels` field access), the match arm pattern needs the `pixels` removed.
- `tests/roundtrip.rs`, `tests/cli.rs`, `tests/multi_image.rs`, `tests/no_vis.rs`, `tests/unknown_vis.rs` — same scan-and-update. Likely most tests just match the variant shape (counting events); only a few inspect `pixels`.

**Perf win:** ~496 `to_vec()` allocations + ~9 Vec growth reallocs → 0.

### D6 — Three sub-items

**D6.1 — `DecodingState` `with_capacity` at construction.** In `process()` around line 326 (the State::AwaitingVis → State::Decoding transition):

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

`target` is the computed `target_audio_samples` (image_lines / 2 × line_seconds × FINDSYNC_AUDIO_HEADROOM × work_rate, per the existing comment). The `has_sync` track holds one entry per `SYNC_PROBE_STRIDE`-spaced sample. Pre-sizing saves ~10 reallocs as both buffers grow.

**D6.2 — Black-image throwaway fix.** Current shape at `decoder.rs:589-592`:

```rust
let final_image = std::mem::replace(
    &mut d.image,
    SstvImage::new(d.mode, d.spec.line_pixels, d.spec.image_lines),
);
out.push(SstvEvent::ImageComplete { image: final_image, partial: false });
```

Refactor: take `d` by value into `run_findsync_and_decode`, move `d.image` directly into the event.

```rust
// In SstvDecoder::process — at the burst-emit transition:
let State::Decoding(d) = std::mem::replace(&mut self.state, State::AwaitingVis) else {
    unreachable!("matched State::Decoding above");
};
Self::run_findsync_and_decode(
    *d,                              // unbox; pass by value
    &mut self.channel_demod,
    &mut self.snr_est,
    &mut self.find_sync_scratch,     // D6.3
    &mut out,
);

// run_findsync_and_decode signature:
fn run_findsync_and_decode(
    mut d: DecodingState,            // by value (was: &mut DecodingState)
    channel_demod: &mut ChannelDemod,
    snr_est: &mut SnrEstimator,
    find_sync_scratch: &mut FindSyncScratch,
    out: &mut Vec<SstvEvent>,
) {
    out.reserve(d.spec.image_lines as usize + 1);  // D5
    // ... per-channel-layout branches continue to use &mut d.image
    // internally for mutation. At the end:
    out.push(SstvEvent::ImageComplete { image: d.image, partial: false });
    //                                  ^^^^^^^ moves; no replace, no fresh alloc.
}
```

`mut d` lets the per-pair / per-line decoders take `&mut d.image`. The final `d.image` move consumes `d` cleanly. State transition (`State::Decoding → AwaitingVis`) moves from "inside `run_findsync_and_decode` via mem::replace" to "in the caller, before the call" — cleaner ownership.

**D6.3 — `FindSyncScratch` on `SstvDecoder`.** New struct in `src/sync.rs`:

```rust
/// Scratch buffers for `find_sync`. Hoisted onto `SstvDecoder` so
/// they're reused across decode passes — the largest two are
/// allocated once at decoder construction. (Audit #93 D6.)
pub(crate) struct FindSyncScratch {
    sync_img: Vec<bool>,    // X_ACC_BINS × SYNC_IMG_Y_BINS = 441,000 entries
    lines: Vec<u16>,        // LINES_D_BINS × n_slant_bins (varies; resized per-call)
    x_acc: Vec<u32>,        // X_ACC_BINS = 700
}

impl FindSyncScratch {
    pub(crate) fn new() -> Self {
        Self {
            sync_img: vec![false; X_ACC_BINS * SYNC_IMG_Y_BINS],
            lines: Vec::new(),  // mode-dependent line_width; resized at use site
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

`find_sync` (and the three helpers it orchestrates — `hough_detect_slant`, `find_falling_edge`, `falling_edge_from_x_acc`) gain a `scratch: &mut FindSyncScratch` parameter. Inside:
- `let mut sync_img = vec![false; X_ACC_BINS * SYNC_IMG_Y_BINS]` → `scratch.sync_img.fill(false)` (already sized).
- `let mut lines = vec![0u16; LINES_D_BINS * n_slant_bins]` → `scratch.lines.clear(); scratch.lines.resize(LINES_D_BINS * n_slant_bins, 0);`.
- `let mut x_acc = vec![0u32; X_ACC_BINS]` → `scratch.x_acc.fill(0);` (already sized).

`hough_detect_slant` and `find_falling_edge` borrow only the buffers they need (`sync_img` + `lines` for the Hough vote; `x_acc` for the convolution); pass `&mut scratch.sync_img`, `&mut scratch.lines`, `&mut scratch.x_acc` from `find_sync` rather than threading the whole struct.

`SstvDecoder` gains a field:

```rust
pub struct SstvDecoder {
    resampler: Resampler,
    vis: crate::vis::VisDetector,
    channel_demod: crate::demod::ChannelDemod,
    snr_est: crate::snr::SnrEstimator,
    find_sync_scratch: crate::sync::FindSyncScratch,  // NEW
    state: State,
    samples_processed: u64,
    working_samples_emitted: u64,
}

impl SstvDecoder {
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
}
```

`reset()` does NOT recreate `find_sync_scratch` (the buffers are state-free; `find_sync` clears them at each call).

The `Debug for SstvDecoder` impl (added in #92) does not surface `find_sync_scratch` — it's purely internal scratch.

**Tests in `src/sync.rs::tests`:** ~5 existing `find_sync_*` tests update to construct a local scratch:

```rust
// Before:
let r = find_sync(&track, rate, spec);

// After:
let mut scratch = FindSyncScratch::new();
let r = find_sync(&track, rate, spec, &mut scratch);
```

**Perf win:** ~730 KB allocated per `find_sync` call → 0 (after first call). Decoder construction allocs once and keeps them.

---

## File touch list

| File | Status | Role |
|------|--------|------|
| `src/demod.rs` | modify | D3 fields on `ChannelDemod`; rewrite `decode_one_channel_into` to use them; if PD line-pair scratches are here, hoist those 4 too. |
| `src/mode_pd.rs` | modify (if exists) | If the PD line-pair `vec![0u8; width]` × 4 sites live here, hoist them onto `ChannelDemod` (or a sibling `PdLineScratch` if PD-specific). |
| `src/sync.rs` | modify | New `FindSyncScratch` struct; `find_sync` + helpers gain `scratch: &mut FindSyncScratch` parameter; existing 5 tests update. |
| `src/decoder.rs` | modify | `SstvEvent::LineDecoded` drops `pixels`; `SstvDecoder` gains `find_sync_scratch` field; `current_image()` method added; `run_findsync_and_decode` rewritten (by-value `d`, `out.reserve`, move `d.image` into event); `DecodingState` constructor uses `with_capacity`. |
| `src/bin/slowrx_cli.rs` | modify | `LineDecoded` consumer update (drop `pixels` field from match arm or migrate to `current_image()`). |
| `tests/roundtrip.rs`, `tests/cli.rs`, `tests/multi_image.rs`, `tests/no_vis.rs`, `tests/unknown_vis.rs` | modify | `LineDecoded` consumer updates; scan-and-fix. |
| `CHANGELOG.md` | modify | New `[Unreleased]` `### Changed` (breaking) section + `### Internal` for the perf bullets. |

---

## Out of scope

- **SIMD'ifying the SNR power-sum inner loops** — separately tracked as #77; the empirical finding there was "function-level `multiversion` wrap is a no-op; needs profile-driven targeting."
- **Sink/callback API** (`process(&mut self, audio, &mut impl FnMut(SstvEvent))`) — also a breaking change, larger lift; deferred.
- **`impl Iterator` for events** — same; deferred.
- **Pooling scratch across decoder instances** — single-decoder use case is the V1 norm; per-decoder ownership is fine.

---

## Release implications

The `LineDecoded.pixels` removal is the first public-API-breaking change since 0.5.0. Per Keep a Changelog + 0.x semver convention, this warrants a **minor version bump** at release time:

- Current published: 0.5.3 (from PR #108, audit-cleanup-wave release).
- This PR's release: **0.6.0**.
- CHANGELOG entry uses both `### Changed` (breaking: `LineDecoded.pixels` removed, `current_image()` added) and `### Internal` (perf hoists for D3 / D6) subsections.

The release itself is a separate chore PR (per the established `chore/release-X.Y.Z` pattern from 0.5.3). This PR's CHANGELOG entry goes under `[Unreleased]` — the release PR finalizes the version and the date.

---

## Success criteria

- All 3 in-scope audit findings addressed (D3, D5, D6).
- Full CI gate green: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features --locked --release`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`.
- Lib test count: **136 → 136** (no new tests; ~5 existing `find_sync_*` tests update to construct local scratch).
- `tests/roundtrip.rs` 11/11 unchanged (perf change must not affect pixel output).
- CLI continues to produce identical filename slugs (already guaranteed by #91/#92; this PR doesn't touch filename logic).
- Allocation counts (qualitatively): the 3 hot-loop allocation patterns (per-channel scratch, per-line `to_vec()`, per-`find_sync` buffers) all become **0 allocs after first decode** instead of growing with image size. Quantitative profiling is out-of-scope here; if someone wants to verify, `cargo bench`-style or `dhat`-instrumented runs would show the per-PD240-image diff.
- `SstvEvent` size (mem::size_of) shrinks by `Vec<[u8;3]>` size (3 words on x86_64 = 24 B per event).
- New `current_image()` API gets a doctest or unit test verifying:
  - `current_image()` returns `None` immediately after `SstvDecoder::new`.
  - After a `LineDecoded` event for row N, `current_image().unwrap().pixels[start..end]` matches what `LineDecoded.pixels` used to contain. (Optional regression check; the existing `tests/roundtrip.rs` pixel-equality checks against the final `ImageComplete.image` cover this transitively.)
