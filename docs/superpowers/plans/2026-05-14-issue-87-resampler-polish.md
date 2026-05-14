# Issue #87 — Resampler polish — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Polish `src/resample.rs` end-to-end — implement a real 256-phase polyphase tap bank (D2; eliminates ~1.4 M transcendentals/sec from the hot path), fix the `needed_end` off-by-one (D2b), correct the `cutoff_hz` doc and document the group delay (D2b), and add five new tests for previously-uncovered behavior (F6). **D1 is a phantom finding** — empirically the existing taps already sum to ~1.0 (verified in T1); the audit confused the Hann *window*'s mean (= 0.5) with the Hann-*windowed-sinc*'s DC gain. T1's amplitude test stays as a regression guard.

**Architecture:** Task 1 lands the five F6 tests — all five pass on current code (including the amplitude test, which acts as a regression guard rather than the TDD-red target the audit predicted). Task 2 implements the polyphase bank + `needed_end` fix, accelerating the hot path (D2 perf win) without changing measurable amplitude behavior. Task 3 lands the remaining D2b doc fixes (`cutoff_hz` doc, struct-level group-delay note, dead-branch comment). Task 4 lands the CHANGELOG and runs the full gate. Each task leaves a working state with the test suite green.

**Tech Stack:** Rust 2021, MSRV 1.85. CI gate: `cargo test --all-features --locked --release`, `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features`. No GPG signing.

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-14-issue-87-resampler-polish-design.md`
- Audit: `docs/audits/2026-05-11-deep-code-review-audit.md` (IDs D1, D2, D2b, F6)

---

## File Structure

| File | Status | Role |
|------|--------|------|
| `src/resample.rs` | modify | The entire change. `Resampler` struct gains `taps: Box<[[f32; FIR_TAPS]; NUM_PHASES]>`, drops `cutoff_norm`. `new()` builds the polyphase bank. `process()` hot loop uses the bank (no transcendentals). 5 new tests in the existing `tests` mod. Module-level doc + struct doc + `cutoff_hz` doc updates. |
| `CHANGELOG.md` | modify | `[Unreleased]` `### Internal` bullet (T4). |

Task order: **T1** (5 F6 tests, all pass — amplitude test is a regression guard, not TDD-red) → **T2** (polyphase + `needed_end`; D2 perf win) → **T3** (remaining D2b doc fixes) → **T4** (CHANGELOG + final gate).

**Verification after each task** (the rule for this PR):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass after every task (including T1 — see T1 Step 2 expectations).

---

## Task 1: Add the five F6 tests (regression guards)

Five new `#[test]` fns in the existing `#[cfg(test)] mod tests` in `src/resample.rs`. All five pass on current code. `exact_rate_preserves_amplitude_and_no_attenuation` was originally planned as a TDD-red target for D1, but D1 turned out to be a phantom finding — the test passes on current code because the Hann-windowed-sinc taps already sum to ~1.0 (the audit confused the Hann window's mean with the Hann-windowed-sinc's DC gain). We keep it as a regression guard against any future change that breaks unit gain unexpectedly. The other four (`upsampling_8khz_to_11025`, `max_input_rate_192khz`, `tiny_chunks_emit_nothing_then_catch_up`, `empty_input_returns_empty`) cover paths that aren't tested today.

**Files:**
- Modify: `src/resample.rs`

- [ ] **Step 1: Add the 5 new tests at the end of `mod tests`**

Open `src/resample.rs`. Find the existing `#[cfg(test)] mod tests { ... }` block (around line 149 to the end of the file). Add these five `#[test]` fns immediately before the closing `}` of `mod tests`:

```rust
    /// Unit-gain regression guard (#87). The audit (D1) claimed the 64
    /// Hann-windowed sinc taps weren't normalized to unit DC gain and the
    /// resampler attenuated by ~6 dB. Empirically false — the windowed-sinc
    /// form `2·fc · sin(2π·fc·n)/(π·n)` already sums to ~1.0 at typical
    /// `fc` (the audit appears to have confused the Hann *window*'s mean
    /// (= 0.5) with the Hann-*windowed-sinc*'s DC gain). This test passes
    /// on current code and stays as a guard against any future change
    /// (rate changes, tap-count tweaks, window swaps) that breaks unit
    /// gain unexpectedly. At `input_rate == WORKING_SAMPLE_RATE_HZ` the
    /// stride is exactly 1.0 and every output sample has `frac == 0`, so
    /// the fractional-delay machinery isn't exercised — gain issues show
    /// up cleanly.
    #[test]
    fn exact_rate_preserves_amplitude_and_no_attenuation() {
        let mut r = Resampler::new(WORKING_SAMPLE_RATE_HZ).unwrap();
        // 200 samples at amplitude 0.8 — well past the 64-tap kernel ramp-up.
        let amplitude = 0.8_f32;
        let in_audio: Vec<f32> = (0..200)
            .map(|i| {
                let t = (i as f64) / f64::from(WORKING_SAMPLE_RATE_HZ);
                (amplitude as f64 * (2.0 * PI * 1500.0 * t).sin()) as f32
            })
            .collect();
        let out = r.process(&in_audio);
        // Skip the first FIR_TAPS samples — the kernel is ramping up against
        // the left zero-pad and the peak amplitude is reduced there.
        let mid_start = FIR_TAPS.min(out.len());
        let out_peak = out[mid_start..]
            .iter()
            .fold(0.0_f32, |m, &x| m.max(x.abs()));
        let in_peak = in_audio.iter().fold(0.0_f32, |m, &x| m.max(x.abs()));
        // Allow ±5 % of input peak. The audit predicted ~50 % attenuation
        // (taps would sum to ~0.5); empirically the ratio is ~1.0 — the
        // windowed-sinc taps already have unity DC gain.
        let ratio = out_peak / in_peak;
        assert!(
            (ratio - 1.0).abs() < 0.05,
            "expected ~1.0 output peak/input peak ratio (unit gain), got {ratio} (in_peak={in_peak}, out_peak={out_peak})"
        );
    }

    /// F6 (#87). Upsampling 8 kHz → 11025 Hz exercises the `stride < 1`
    /// path that no existing test hits. Output length should be ~11025
    /// samples (1 second at working rate) ±64; Goertzel power at 1500 Hz
    /// should dominate adjacent off-band bins.
    #[test]
    fn upsampling_8khz_to_11025() {
        let mut r = Resampler::new(8_000).unwrap();
        let in_audio = synth_tone_at(8_000, 1500.0, 1.0);
        let out = r.process(&in_audio);
        let expected = WORKING_SAMPLE_RATE_HZ as usize;
        assert!(
            (out.len() as isize - expected as isize).abs() < 200,
            "out.len()={} expected≈{expected}",
            out.len()
        );
        let p_target = crate::dsp::goertzel_power(&out, 1500.0);
        let p_off1 = crate::dsp::goertzel_power(&out, 1200.0);
        let p_off2 = crate::dsp::goertzel_power(&out, 1800.0);
        assert!(
            p_target > 10.0 * p_off1.max(p_off2),
            "p1500={p_target} p1200={p_off1} p1800={p_off2}"
        );
    }

    /// F6 (#87). 192 kHz input — the max supported rate. Stride ≈ 17.41;
    /// many input samples per output. Just verify no panic, output length
    /// is in the right ballpark, and the tone survives.
    #[test]
    fn max_input_rate_192khz() {
        let mut r = Resampler::new(MAX_INPUT_SAMPLE_RATE_HZ).unwrap();
        let in_audio = synth_tone_at(MAX_INPUT_SAMPLE_RATE_HZ, 2000.0, 0.5);
        let out = r.process(&in_audio);
        // 0.5 s at WORKING_SAMPLE_RATE_HZ.
        let expected = (WORKING_SAMPLE_RATE_HZ / 2) as usize;
        assert!(
            (out.len() as isize - expected as isize).abs() < 200,
            "out.len()={} expected≈{expected}",
            out.len()
        );
        let p_target = crate::dsp::goertzel_power(&out, 2000.0);
        let p_off1 = crate::dsp::goertzel_power(&out, 1700.0);
        let p_off2 = crate::dsp::goertzel_power(&out, 2300.0);
        assert!(
            p_target > 10.0 * p_off1.max(p_off2),
            "p2000={p_target} p1700={p_off1} p2300={p_off2}"
        );
    }

    /// F6 (#87). Tiny chunks: each call passes fewer samples than the
    /// 64-tap kernel needs, so the resampler should accumulate them in
    /// `tail` and emit nothing until `tail.len() >= FIR_TAPS`. Verifies
    /// the streaming-buffer carry-over correctness — the production
    /// decoder's per-call audio chunks can be small.
    #[test]
    fn tiny_chunks_emit_nothing_then_catch_up() {
        let mut r = Resampler::new(44_100).unwrap();
        let chunk = [0.5_f32, 0.5, 0.5];
        let mut emitted_before_threshold = 0;
        // 21 chunks of 3 samples = 63 < FIR_TAPS = 64. No output yet.
        for _ in 0..21 {
            let out = r.process(&chunk);
            emitted_before_threshold += out.len();
        }
        assert_eq!(
            emitted_before_threshold, 0,
            "expected no output before FIR_TAPS samples buffered, got {emitted_before_threshold}"
        );
        // One more chunk pushes us past FIR_TAPS — at least one sample emerges.
        let out_after = r.process(&chunk);
        assert!(
            !out_after.is_empty(),
            "expected at least one output sample after crossing the FIR_TAPS threshold"
        );
    }

    /// F6 (#87). Empty input is a no-op — returns an empty Vec and
    /// leaves the resampler state untouched. Plus: an empty call
    /// sandwiched between two non-empty calls doesn't perturb the output
    /// (streaming idempotence).
    #[test]
    fn empty_input_returns_empty() {
        let mut r = Resampler::new(44_100).unwrap();
        assert!(r.process(&[]).is_empty());

        // Sandwich: process(non-empty) → process(empty) → process(non-empty)
        // should produce the same output as process(non-empty ++ non-empty).
        let mut a = Resampler::new(44_100).unwrap();
        let in_audio = synth_tone_at(44_100, 1500.0, 0.2);
        let mid = in_audio.len() / 2;

        let mut sandwiched = a.process(&in_audio[..mid]);
        let empty_call = a.process(&[]);
        assert!(empty_call.is_empty());
        sandwiched.extend_from_slice(&a.process(&in_audio[mid..]));

        let mut b = Resampler::new(44_100).unwrap();
        let combined = b.process(&in_audio);

        // Same length within 1, same per-sample values within tiny tolerance
        // (the empty call shouldn't have moved the FIR's internal state).
        assert!(
            (sandwiched.len() as isize - combined.len() as isize).abs() <= 1,
            "sandwiched.len()={} combined.len()={}",
            sandwiched.len(),
            combined.len()
        );
        let common = sandwiched.len().min(combined.len());
        let max_diff = (0..common)
            .map(|i| (sandwiched[i] - combined[i]).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_diff < 1e-6, "max_diff={max_diff}");
    }
```

- [ ] **Step 2: Run the gate — all five should pass**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

Expected: all five new tests pass. The amplitude test's ratio should be very close to 1.0 (not the ~0.5 the audit predicted).

If `exact_rate_preserves_amplitude_and_no_attenuation` **fails** with a ratio near 0.5, STOP and report — that would mean D1 *is* real on this machine and the plan needs to revert to the original normalization scope.

If any of the **other** four new tests fail, STOP and report — they cover paths/inputs the existing tests don't and they should all pass on current code.

- [ ] **Step 3: Commit**

```bash
git add src/resample.rs
git commit -m "test(resample): F6 — five regression-guard tests for resampler (#87)

Includes \`exact_rate_preserves_amplitude_and_no_attenuation\` as a guard
for unit gain — verifies the audit's D1 claim is phantom (taps already
sum to ~1.0) and protects against future regressions.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Polyphase tap bank + `needed_end` fix

The algorithmic refactor. Replaces the per-sample tap computation with a precomputed 256-phase bank built in `new()`. **No normalization pass** — D1 was a phantom finding; the raw windowed-sinc taps already sum to ~1.0 (verified empirically in T1). Fixes the `needed_end` off-by-one as part of the hot-path rewrite. Test suite stays green throughout (T1's amplitude test continues to pass; quantization noise at 256 phases ≈ −52 dB on a 2300 Hz tone, well below the ±5 % tolerance).

**Files:**
- Modify: `src/resample.rs`

- [ ] **Step 1: Add `NUM_PHASES` const + update the module-level doc**

In `src/resample.rs`, immediately after `const FIR_TAPS: usize = 64;` (around line 20), add:

```rust
/// Number of polyphase positions. Each fractional output sample's `frac`
/// is quantized to one of `NUM_PHASES` precomputed tap rows. 256 gives a
/// max sub-sample position error of `1 / (2·NUM_PHASES) = 1/512` sample,
/// which at our 11025 Hz output rate corresponds to ≈ 177 ns time error
/// — phase noise on a 2300 Hz tone (SSTV's highest video frequency) of
/// `≈ −52 dB`, well below the audible threshold and SSTV's noise floor.
/// Memory cost: `NUM_PHASES × FIR_TAPS × 4 B` = 64 KB per `Resampler`.
const NUM_PHASES: usize = 256;
```

Update the module-level doc at the top of the file. Currently:

```rust
//! Internal rational resampler: caller's audio rate → 11025 Hz working rate.
//!
//! Hand-rolled 64-tap Hann-windowed-sinc polyphase FIR. We picked this over
//! `rubato` for zero extra deps and a small file. Quality target is "audible
//! loss < 0.1 dB across SSTV-relevant frequencies (1500-2300 Hz)" — easily
//! met at typical input rates (44.1k, 48k). Translated in spirit from
//! slowrx's implicit resampling inside `pcm.c`'s 44.1 kHz read loop.
```

Replace with:

```rust
//! Internal rational resampler: caller's audio rate → 11025 Hz working rate.
//!
//! Hand-rolled 64-tap Hann-windowed-sinc polyphase FIR with 256 phase
//! positions. Tap rows are precomputed once in
//! [`Resampler::new`] (~64 KB); the hot path in [`Resampler::process`] is
//! a quantized-phase lookup + 64-tap multiply-accumulate — no
//! transcendentals per output sample.
//!
//! We picked this over `rubato` for zero extra deps and a small file.
//! Quality target is "audible loss < 0.1 dB across SSTV-relevant
//! frequencies (1500-2300 Hz)" — easily met at typical input rates
//! (44.1k, 48k). Translated in spirit from slowrx's implicit resampling
//! inside `pcm.c`'s 44.1 kHz read loop.
```

- [ ] **Step 2: Re-purpose `fir_tap` — caller is now `Resampler::new` only**

The existing `fn fir_tap(tap_index: usize, frac: f64, fc: f64) -> f32` (around lines 49-63) is mathematically correct; only its caller site changes (from per-output-sample in `process` to per-(`NUM_PHASES × FIR_TAPS`) in `new`). Leave the body unchanged, but update its doc comment to reflect the new role:

Replace the existing doc comment immediately preceding `fn fir_tap`:

```rust
/// Compute one Hann-windowed sinc FIR tap value for a given tap index
/// and fractional phase. Called once per `(phase, tap)` pair from
/// [`Resampler::new`] to populate the polyphase tap bank — never called
/// from the hot path.
///
/// `tap_index` is in 0..`FIR_TAPS`. `frac` is in [0, 1) — the sub-sample
/// offset of the output sample's center from the integer input grid.
/// `fc` is the cutoff normalized to input rate (`cutoff_hz / input_rate`).
///
/// Sinc shifts with `frac`; Hann window stays anchored to the tap grid.
/// This is the standard windowed-sinc fractional-delay formulation —
/// see e.g. Smith, "Digital Audio Resampling Home Page" (CCRMA, 2002).
///
/// The taps already sum to ~1.0 at typical `fc` (the audit's D1 claim of
/// "~6 dB attenuation" was a phantom finding — see the
/// `exact_rate_preserves_amplitude_and_no_attenuation` test for the
/// regression guard).
```

Body stays exactly as it is. No rename, no return-type change.

- [ ] **Step 3: Replace the `Resampler` struct: drop `cutoff_norm`, add `taps`**

Find the struct definition (currently around lines 22-34):

```rust
pub struct Resampler {
    input_rate: u32,
    /// `input_rate / WORKING_SAMPLE_RATE_HZ`, expressed as a stride.
    stride: f64,
    /// Position into the input buffer (fractional, accumulates across calls).
    phase: f64,
    /// Carry-over input samples from the previous call.
    tail: Vec<f32>,
    /// Cutoff frequency normalized to input rate (taps spaced at `1/input_rate`).
    cutoff_norm: f64,
}
```

Replace with:

```rust
/// Polyphase FIR resampler. Stateful — holds a tail buffer to avoid
/// glitches across `process` calls.
pub struct Resampler {
    input_rate: u32,
    /// `input_rate / WORKING_SAMPLE_RATE_HZ`, expressed as a stride.
    stride: f64,
    /// Position into the input buffer (fractional, accumulates across calls).
    phase: f64,
    /// Carry-over input samples from the previous call.
    tail: Vec<f32>,
    /// 256-phase polyphase tap bank, indexed by `frac` quantized to
    /// 1/256 sub-sample. Built once in [`Resampler::new`] (~64 KB, static
    /// for the resampler's lifetime). Each row is a Hann-windowed sinc at
    /// the corresponding fractional delay. Raw taps (no normalization
    /// pass) — the windowed-sinc form already sums to ~1.0 at typical
    /// `fc` (the audit's D1 claim of "~6 dB attenuation" was a phantom
    /// finding, verified by the `exact_rate_…` test in T1).
    taps: Box<[[f32; FIR_TAPS]; NUM_PHASES]>,
}
```

(The struct-level `///` doc comment immediately preceding it stays as the existing one; Task 3 will append the group-delay note.)

- [ ] **Step 4: Rewrite `Resampler::new()` to build the polyphase bank**

Find `pub fn new(input_rate: u32) -> Result<Self>` (around lines 70-83). Replace the entire fn body with:

```rust
    pub fn new(input_rate: u32) -> Result<Self> {
        if input_rate == 0 || input_rate > MAX_INPUT_SAMPLE_RATE_HZ {
            return Err(Error::InvalidSampleRate { got: input_rate });
        }
        let cutoff_norm = cutoff_hz(input_rate) / f64::from(input_rate);

        // Build the 256-phase polyphase tap bank — one 64-tap row per
        // quantized fractional phase. Computed once here, looked up in
        // the hot path (no transcendentals per output sample). No
        // normalization pass: raw Hann-windowed-sinc taps already sum
        // to ~1.0 at typical `fc` (audit #87 D1 — phantom finding,
        // verified by `exact_rate_preserves_amplitude_and_no_attenuation`).
        let mut taps: Box<[[f32; FIR_TAPS]; NUM_PHASES]> =
            Box::new([[0.0_f32; FIR_TAPS]; NUM_PHASES]);
        for phase_idx in 0..NUM_PHASES {
            let frac = (phase_idx as f64) / (NUM_PHASES as f64);
            for k in 0..FIR_TAPS {
                taps[phase_idx][k] = fir_tap(k, frac, cutoff_norm);
            }
        }

        Ok(Self {
            input_rate,
            stride: f64::from(input_rate) / f64::from(WORKING_SAMPLE_RATE_HZ),
            phase: 0.0,
            tail: Vec::new(),
            taps,
        })
    }
```

The `cutoff_norm` local is used only during construction and dropped — the runtime no longer needs it on the struct.

- [ ] **Step 5: Rewrite the `process()` hot loop**

Find `pub fn process(&mut self, input: &[f32]) -> Vec<f32>` (around lines 86-130). The body needs three changes:
1. The hot loop reads from `self.taps[phase_idx]` instead of computing taps fresh.
2. `needed_end` becomes `(self.phase.floor() as usize) + FIR_TAPS` (D2b off-by-one fix).
3. The `half` / `center` locals from the old code go away.

Replace the entire fn body with:

```rust
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        // Concatenate carry-over with the new chunk.
        let mut buf = std::mem::take(&mut self.tail);
        buf.extend_from_slice(input);

        let mut out = Vec::new();
        loop {
            // D2b off-by-one fix (#87): the kernel reads indices
            // `floor(phase)..floor(phase) + FIR_TAPS`, so it needs
            // `floor(phase) + FIR_TAPS` samples in `buf`. Pre-#87 this
            // was `(phase + FIR_TAPS).ceil()`, which over-reserved by one
            // sample for fractional `phase`.
            let needed_end = (self.phase.floor() as usize) + FIR_TAPS;
            if needed_end > buf.len() {
                break;
            }
            let frac = self.phase.fract();
            let phase_idx = ((frac * NUM_PHASES as f64).round() as usize).min(NUM_PHASES - 1);
            let taps = &self.taps[phase_idx];
            let start = self.phase.floor() as isize;

            // Convolve using the precomputed taps at this quantized phase.
            // No transcendentals in the hot path.
            let mut acc: f32 = 0.0;
            for k in 0..FIR_TAPS {
                let idx = start + k as isize;
                if (0..buf.len() as isize).contains(&idx) {
                    acc += taps[k] * buf[idx as usize];
                }
            }
            out.push(acc);
            self.phase += self.stride;
        }

        // Keep the trailing samples that the next call will need.
        let drop = self.phase.floor() as usize;
        if drop < buf.len() {
            self.tail = buf[drop..].to_vec();
            self.phase -= drop as f64;
        } else {
            // Unreachable under MAX_INPUT_SAMPLE_RATE_HZ — defensive against a
            // future cap relaxation. (#87 D2b acknowledgment.)
            self.tail.clear();
            self.phase -= buf.len() as f64;
        }
        out
    }
```

- [ ] **Step 6: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. The amplitude test continues to pass (T1 already established it does on raw taps; the polyphase bank changes nothing about the per-row tap values for `frac == 0`). The full test suite stays green — including `tests/roundtrip.rs` 11/11 — since 256-phase quantization noise is ≈ −52 dB on a 2300 Hz tone, well below SSTV's noise floor.

- [ ] **Step 7: Commit**

```bash
git add src/resample.rs
git commit -m "refactor(resample): 256-phase polyphase tap bank + needed_end fix (#87 D2/D2b)

Precomputes 256 × 64 Hann-windowed-sinc taps once in new() (~64 KB);
process() hot path is now a quantized-phase lookup + 64-tap MAC, with
no transcendentals per output sample. ~1.4 M sin()+cos() calls/sec
removed from the SSTV decode pipeline. needed_end off-by-one folded
into the rewrite. D1 (unit-gain normalization) was a phantom finding
— raw taps already sum to ~1.0; see T1's regression-guard test.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: D2b remaining doc fixes

The three documentation polishes: correct `cutoff_hz` doc, append a group-delay note to the struct, and acknowledge the dead `else` branch.

**Files:**
- Modify: `src/resample.rs`

- [ ] **Step 1: Fix the `cutoff_hz` doc comment**

Find the `cutoff_hz` fn (around line 37-42 in the post-T2 file):

```rust
/// Cutoff frequency (Hz) for the resampler, derived from the input rate.
/// Min of (`input_rate/2`, `working_rate/2`) × 0.45, hard-capped at 4500 Hz.
fn cutoff_hz(input_rate: u32) -> f64 {
    (f64::from(input_rate.min(WORKING_SAMPLE_RATE_HZ)) * 0.45).min(4500.0)
}
```

Replace the doc comment (the body of the fn stays unchanged):

```rust
/// Cutoff frequency (Hz) for the resampler, derived from the input rate.
/// `min(input_rate, WORKING_SAMPLE_RATE_HZ) × 0.45`, hard-capped at 4500 Hz —
/// i.e. ~0.9 × Nyquist of the lower of the two rates. The cap keeps the
/// passband from extending past SSTV's 2300 Hz video band at typical input
/// rates (44.1k → 4961 Hz uncapped → 4500 capped).
fn cutoff_hz(input_rate: u32) -> f64 {
    (f64::from(input_rate.min(WORKING_SAMPLE_RATE_HZ)) * 0.45).min(4500.0)
}
```

(The audit pointed out the prior doc said `input_rate/2` — Nyquist — while the code is `input_rate`, full rate. The code is right; only the doc changes.)

- [ ] **Step 2: Append the group-delay note to the `Resampler` struct doc**

Find the doc comment immediately preceding the `Resampler` struct. The post-T2 doc reads:

```rust
/// Polyphase FIR resampler. Stateful — holds a tail buffer to avoid
/// glitches across `process` calls.
pub struct Resampler {
```

Replace with:

```rust
/// Polyphase FIR resampler. Stateful — holds a tail buffer to avoid
/// glitches across `process` calls.
///
/// **Group delay:** the 64-tap symmetric FIR has linear-phase group delay
/// of `(FIR_TAPS - 1) / 2 = 31.5` input-rate samples (≈ 715 µs at 44.1 kHz,
/// ≈ 2.86 ms at 11.025 kHz). Output is shifted right by this amount
/// relative to input. SSTV's `find_sync` re-anchors the rate against sync
/// pulses, so this is invisible inside the decoder pipeline; standalone
/// consumers should compensate if they need sample-accurate alignment.
pub struct Resampler {
```

- [ ] **Step 3: Run the gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Doc-only changes; the test suite should be unchanged from T2.

- [ ] **Step 4: Commit**

```bash
git add src/resample.rs
git commit -m "docs(resample): D2b — cutoff_hz doc + group delay on Resampler struct (#87)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

(The `else { tail.clear(); phase -= buf.len() as f64; }` dead-branch comment is already in place from T2 Step 5's hot-path rewrite — no separate edit needed here.)

---

## Task 4: CHANGELOG + final gate

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add the `CHANGELOG.md` `[Unreleased]` entry**

In `CHANGELOG.md`, under the `## [Unreleased]` header (which already has an `### Internal` subsection from #85 and #86), prepend this bullet to the existing `### Internal` block (so the most recent change is first):

```markdown
### Internal

- **Resampler polish** — `Resampler` now uses a true 256-phase polyphase
  tap bank (precomputed in `new()`, ~64 KB) instead of recomputing all 64
  Hann-windowed-sinc taps per output sample; eliminates ~1.4 M `sin()`
  +`cos()` calls/sec from the hot path. Off-by-one in `needed_end` (was
  `ceil(phase + 64)` causing one extra sample of buffering; now
  `floor(phase) + 64`) fixed. Group delay documented (`(FIR_TAPS - 1) / 2
  = 31.5` input-rate samples). `cutoff_hz` doc corrected (Nyquist factor
  was wrong in the doc; the code was right). Five new tests
  (`exact_rate_preserves_amplitude_and_no_attenuation`,
  `upsampling_8khz_to_11025`, `max_input_rate_192khz`,
  `tiny_chunks_emit_nothing_then_catch_up`, `empty_input_returns_empty`).
  Audit D1 ("~6 dB attenuation from un-normalized taps") was a phantom
  finding — empirically the raw Hann-windowed-sinc taps already sum to
  ~1.0 at typical `fc` (the audit appears to have confused the Hann
  window's mean with the windowed-sinc's DC gain); the amplitude test
  stays as a regression guard. Quantization noise at 256 phases is ≈ −52
  dB phase noise on a 2300 Hz tone — well below SSTV's noise floor.
  (#87; audit D2/D2b/F6, D1 closed-as-phantom.)

- **Extracted `crate::test_tone`** — [existing #86 bullet stays as-is below]
```

- [ ] **Step 2: Run the full CI gate**

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

All four must pass. Expected test counts: lib ~129 (124 pre-#87 + 5 new F6 tests); roundtrip 11/11; everything else unchanged. Doc clean.

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(refactor): CHANGELOG for the resampler polish (#87)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-review notes (for the implementer / reviewers)

- **Spec coverage:**
  - D1 (unit-gain normalization) → **phantom finding** — closed via T1's `exact_rate_…` test passing on raw taps (no code change needed; test stays as regression guard). Documented in plan/spec/CHANGELOG.
  - D2 (polyphase bank + transcendentals out of hot path) → T2 Steps 1-6 (NUM_PHASES const + struct field + new() build + process() lookup hot path + module-level doc).
  - D2b cutoff_hz doc → T3 Step 1.
  - D2b group delay → T3 Step 2.
  - D2b needed_end off-by-one → T2 Step 5 (folded into the hot-path rewrite).
  - D2b dead-branch comment → T2 Step 5 (folded into the hot-path rewrite).
  - F6 (5 new tests) → T1.
  - CHANGELOG → T4.

- **D1 is a phantom finding.** T1's `exact_rate_preserves_amplitude_and_no_attenuation` test passes on current code with ratio ~1.0, not the ~0.5 the audit predicted. The Hann-windowed-sinc form `2·fc · sin(2π·fc·n)/(π·n)` already has unity passband DC gain — the audit appears to have conflated the Hann window's mean (= 0.5 by definition) with the Hann-*windowed-sinc*'s DC gain. The amplitude test stays as a regression guard (cheap insurance against any future rate-change/tap-count/window swap that breaks unit gain).

- **Quantization noise at 256 phases ≈ −52 dB.** The audit and spec describe this in detail. Empirically the load-bearing check is `tests/roundtrip.rs` 11/11 — every supported mode decodes a synthetic image through the resampler; any quantization-noise issue would show up there as a pixel-diff regression.

- **Out of scope** (tracked separately under epic #97):
  - Lerp between adjacent phases (defer — quantization noise already inaudible).
  - SIMD-ifying the 64-tap inner loop (#77 — needs profile-driven targeting).
  - Remaining audit cleanups (#88 / #91-96).
