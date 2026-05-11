# Issue #90 — Multi-image streaming — design

**Issue:** [#90](https://github.com/jasonherald/slowrx.rs/issues/90) (audit bundle 6 of 12 — IDs A2, D4)
**Source of record:** `docs/audits/2026-05-11-deep-code-review-audit.md` (A2, D4)
**Spun off:** [#99](https://github.com/jasonherald/slowrx.rs/issues/99) — `sample_offset` is relative-to-restart, not absolute (documented here, fixed there).
**Scope:** one localized change in `SstvDecoder::process`'s `State::Decoding` arm (the post-image transition), plus a doc fix and one integration test. No public API change.

## Background

`SstvDecoder::process(&mut self, audio: &[f32]) -> Vec<SstvEvent>` runs `loop { match &mut self.state { State::AwaitingVis => {…} State::Decoding(d) => {…} } }`. When a `Decoding` image's audio buffer fills (`d.audio.len() >= d.target_audio_samples`), `run_findsync_and_decode` decodes the whole image (emitting `LineDecoded` ×N + `ImageComplete`), and then the arm currently does:

```rust
let trailing = std::mem::take(&mut d.audio);
self.state = State::AwaitingVis;
Self::restart_vis_detection(&mut self.vis, self.working_samples_emitted, &trailing);
break;
```

Two problems (from the audit):

- **A2** — the `break` exits the `process()` loop, so a back-to-back VIS already sitting in `trailing` isn't surfaced until the *next* `process()` call. For true multi-image streams (ARISS transmits PD120 images back-to-back) the 2nd image needs a 2nd `process()` call to even get a `VisDetected`, and a 3rd to finish decoding — the "Closes #31" comment overpromises. The known-code / unknown-code branches in `State::AwaitingVis` already `continue` after re-seeding; this arm should too.
- **D4** — `std::mem::take(&mut d.audio)` hands the *entire* decoded-image audio buffer (~1.4–3 M samples for a PD image) to a fresh `VisDetector`, which then FFTs ~10⁴ sliding-window hops of video tones hunting for a VIS burst that physically cannot be in the first ~99 % of it. Only the tail — near where a follow-up VIS could plausibly start — needs re-scanning.

## Design

### The fix (one change, `src/decoder.rs`)

In `State::Decoding(d)`, after `run_findsync_and_decode(...)`, replace the four lines above with:

```rust
// Image complete. Carry forward only the tail of the image audio (a few
// lines, to absorb a fast TX clock) plus everything past it — the rest is
// decoded video tones a VIS burst can't be hiding in. (#90: A2 + D4.)
let work_rate = f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ);
let carryback =
    (f64::from(MULTI_IMAGE_CARRYBACK_LINES) * d.spec.line_seconds * work_rate) as usize;
let carry_from = d.target_audio_samples.saturating_sub(carryback);
Self::restart_vis_detection(&mut self.vis, self.working_samples_emitted, &d.audio[carry_from..]);
self.state = State::AwaitingVis;
remaining = &[]; // already folded into d.audio → now inside the fresh detector; don't re-feed
continue; // re-enter the loop — a back-to-back VIS surfaces this same process() call
```

with, next to `const FINDSYNC_AUDIO_HEADROOM: f64 = 1.00;`:

```rust
/// How many scan lines of the *just-decoded* image audio to keep when
/// re-arming the VIS detector after `ImageComplete` (issue #90 D4). A
/// back-to-back transmission's VIS leader starts right after the image's
/// last line; carrying back this many lines absorbs a fast transmitter
/// clock (4 lines ≈ 0.8 % of a PD240's airtime, ~1.7 % of Robot24's — far
/// more than any real clock error, <0.1 %) so the leader is always inside
/// the carry-forward window. For a single transmission this is just the
/// image's last few lines plus trailing silence; the fresh detector finds
/// nothing and waits for more audio.
const MULTI_IMAGE_CARRYBACK_LINES: u32 = 4;
```

Notes:
- `restart_vis_detection` already takes `&[f32]` (post-#89), so this passes a sub-slice of `d.audio` directly — no `std::mem::take`, no `trailing` `Vec`, no allocation. `VisDetector::process` copies the slice into its own internal buffer, so `d.audio` need not outlive the call (and it doesn't — `self.state = State::AwaitingVis` immediately after drops the `Box<DecodingState>`).
- Borrow check: `Self::restart_vis_detection(&mut self.vis, self.working_samples_emitted, &d.audio[carry_from..])` — `&mut self.vis` + a read of `self.working_samples_emitted` + an immutable reborrow of `d.audio` (which borrows `self.state` via `d`) — all disjoint fields of `self`; `restart_vis_detection` is an associated fn, not `&mut self`. Compiles. (Same pattern this arm already uses.)
- `carry_from`: we only reach this code when `d.audio.len() >= d.target_audio_samples`, and `carry_from = target.saturating_sub(carryback) <= target <= d.audio.len()` — so `&d.audio[carry_from..]` is always a valid (possibly empty) slice.
- `remaining = &[]`: the `working` slice the caller passed was folded into `d.audio` earlier in this arm (`d.audio.extend_from_slice(remaining)`), and `d.audio[carry_from..]` (which includes all of it, since `working` sits at the end of `d.audio` past `target` ≥ `carry_from`) is now inside the fresh detector. Without resetting `remaining`, the next `State::AwaitingVis` iteration would re-feed it.

**Cascade:** with `continue`, the next loop iteration is `State::AwaitingVis` → `self.vis.process(&[], …)` (no-op) → `take_detected()`. If the fresh detector found a 2nd VIS in the carry-forward, it surfaces now → known code → `VisDetected` emitted → `State::Decoding` for image 2, with `residual` = the detector's post-stop-bit buffer = image 2's video audio (which was also in the carry-forward). `d2.audio.len()` ≈ that residual ≈ `target` → `run_findsync_and_decode` for image 2 → `ImageComplete` ×2 in one `process()` call. That's the multi-image win.

### Doc fix — `working_samples_emitted` field (`src/decoder.rs`)

The field doc currently states that `DetectedVis::end_sample` "is computed directly from `total_samples_consumed` and `buffer.len()` … so `sample_offset` in `SstvEvent::VisDetected` is always correct." That is true only for the *first* detection. After a restart (post-image here, or post-unknown-VIS from #89), the fresh `VisDetector` starts `hops_completed` at 0, so a subsequent detection's `end_sample = hops_into_carry_forward * HOP_SAMPLES` — relative to where the carry-forward began, not absolute from sample 0. Replace the "always correct" claim with an honest note: first detection's `sample_offset` is absolute; after a restart it is relative to the carry-forward start ("informational only"; tracked in #99 — fixing it needs a `VisDetector` API change).

### Out of scope

- The `sample_offset`-relative-to-restart fix itself — [#99](https://github.com/jasonherald/slowrx.rs/issues/99) (needs a `VisDetector` constructor that takes the absolute starting sample index).
- The mid-image-VIS-detection TODO in the `State::Decoding` arm (a *new* VIS arriving *during* decoding, before the buffer fills → flush `partial: true`) — unrelated; its comment stays put.
- `FINDSYNC_AUDIO_HEADROOM` / the bottom-of-image-truncation question (pre-existing; not #90's).
- Any wider `decoder.rs` decomposition (audit B12/B14).

## Tests

### `tests/multi_image.rs` (new; `#![cfg(feature = "test-support")]`, like `tests/roundtrip.rs`)

`decoder_decodes_two_back_to_back_images`:
- Build a synthetic image (`(w, h, Vec<[u8;3]>)` of YCrCb-ish data — reuse the gradient-stripe pattern from `tests/roundtrip.rs`'s `test_image`, or a simpler hand-rolled one) twice (call them `img1`, `img2` — can be the same data).
- `audio = synth_vis(0x5F, 0.0) ++ encode_pd(Pd120, &img1_ycrcb) ++ synth_vis(0x5F, 0.0) ++ encode_pd(Pd120, &img2_ycrcb) ++ [0.0; 512]` (the trailing pad covers the resampler FIR group delay so image 2's last line / VIS 2 produce full windows).
- `decoder.process(&audio)` — **one** call. Collect events.
- Assert exactly `2` × `SstvEvent::VisDetected { mode: SstvMode::Pd120, .. }` and exactly `2` × `SstvEvent::ImageComplete { partial: false, .. }`.
- Assert ordering: the first `VisDetected` < the first `ImageComplete` < the second `VisDetected` < the second `ImageComplete` (by position in the event vector).
- (Fails on the current code: the single `process()` call returns only image 1's events; image 2 needs further calls.)

Helpers come from `slowrx::__test_support::vis::synth_vis` and `slowrx::__test_support::mode_pd::encode_pd` (both already used by `tests/roundtrip.rs`). Register the test in `Cargo.toml` as a `[[test]]` with `required-features = ["test-support"]`, matching `roundtrip` / `unknown_vis`.

### Existing tests

`tests/roundtrip.rs` (PD/Robot/Scottie/Martin round-trips) and `decoder::tests` already exercise the single-image path; the post-image-completion change must not regress them — the full suite stays green. (`tests/unknown_vis.rs` from #89 exercises the post-unknown-VIS reseed, which is untouched here.)

## CHANGELOG (`[Unreleased]`)

```markdown
### Fixed

- **Multi-image streams now decode all images within a single `process()` call.**
  After `ImageComplete`, `SstvDecoder::process` re-enters VIS detection in-place
  (`continue`, not `break`), so a back-to-back transmission's next VIS — and the
  image after it — surface in the same call instead of requiring further
  `process()` calls (#90 A2). Note: `sample_offset` on detections *after* the
  first is relative to the carry-forward start, not absolute (#99).

### Performance

- After `ImageComplete`, the VIS detector is re-armed on only the last few scan
  lines of the decoded image audio (plus everything past), not the entire
  ~1.4–3 M-sample buffer — no more FFT-scanning ~10⁴ hops of decoded video tones
  for a VIS burst that can't be there (#90 D4).
```

## Files touched

- `src/decoder.rs` — `const MULTI_IMAGE_CARRYBACK_LINES`; the post-image transition in the `State::Decoding` arm (carry-forward slice + `continue` + `remaining = &[]`); `working_samples_emitted` field-doc fix.
- `tests/multi_image.rs` — new integration test.
- `Cargo.toml` — `[[test]] name = "multi_image" required-features = ["test-support"]`.
- `CHANGELOG.md` — `[Unreleased]` Fixed + Performance entries.

## Verification

Full local CI gate, per repo convention:
- `cargo test --all-features --locked --release`
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`
