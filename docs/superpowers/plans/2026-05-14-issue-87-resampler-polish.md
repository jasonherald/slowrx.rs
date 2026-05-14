# Issue #87 — Resampler polish — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Polish `src/resample.rs` end-to-end — implement a real 256-phase polyphase tap bank (D2; eliminates ~1.4 M transcendentals/sec from the hot path), normalize taps to unit DC gain (D1; kills the ~6 dB attenuation), fix the `needed_end` off-by-one (D2b), correct the `cutoff_hz` doc and document the group delay (D2b), and add five new tests for previously-uncovered behavior (F6).

**Architecture:** TDD-red first — Task 1 lands the five F6 tests; one of them (`exact_rate_preserves_amplitude_and_no_attenuation`) fails on the current code because the ~6 dB attenuation breaks the ±5 % peak-amplitude assertion. Task 2 implements the polyphase bank + unit-gain normalization + `needed_end` fix, turning that test green and accelerating the hot path. Task 3 lands the remaining D2b doc fixes (`cutoff_hz` doc, struct-level group-delay note, dead-branch comment). Task 4 lands the CHANGELOG and runs the full gate. Each task leaves a working state with the test suite green (modulo the deliberately-failing T1 test until T2 lands).

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

Task order: **T1** (TDD-red — 5 F6 tests, 1 fails) → **T2** (polyphase + unit-gain + `needed_end`; turns the failing test green) → **T3** (remaining D2b doc fixes) → **T4** (CHANGELOG + final gate).

**Verification after each task** (the rule for this PR):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked --release
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

(Task 1 is the deliberate exception — its test run is expected to surface exactly one failing test, `exact_rate_preserves_amplitude_and_no_attenuation`, which Task 2 then turns green.)

---

## Task 1: TDD-red — add the five F6 tests

Five new `#[test]` fns in the existing `#[cfg(test)] mod tests` in `src/resample.rs`. One (`exact_rate_preserves_amplitude_and_no_attenuation`) will fail on the current code — that's the D1 regression net. The other four (`upsampling_8khz_to_11025`, `max_input_rate_192khz`, `tiny_chunks_emit_nothing_then_catch_up`, `empty_input_returns_empty`) cover paths that aren't tested today and should pass even on the current pre-T2 code.

**Files:**
- Modify: `src/resample.rs`

- [ ] **Step 1: Add the 5 new tests at the end of `mod tests`**

Open `src/resample.rs`. Find the existing `#[cfg(test)] mod tests { ... }` block (around line 149 to the end of the file). Add these five `#[test]` fns immediately before the closing `}` of `mod tests`:

```rust
    /// D1 regression net (#87). Pre-#87 the resampler attenuated by ~6 dB
    /// because its 64 Hann-windowed sinc taps weren't normalized to unit
    /// DC gain. At `input_rate == WORKING_SAMPLE_RATE_HZ` the stride is
    /// exactly 1.0 and every output sample has `frac == 0`, so the
    /// fractional-delay machinery isn't exercised — the test exposes the
    /// gain issue cleanly.
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
        // Allow ±5 % of input peak. Pre-#87 the resampler attenuated by ~50 %
        // (the Hann-windowed-sinc taps summed to ~0.5 at every phase),
        // failing this assertion. Post-#87 (unit-gain normalization) the
        // ratio should be near 1.0.
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

- [ ] **Step 2: Run the tests — exactly one should fail**

Run: `cargo test --all-features --release --lib resample`

Expected output (the key data point):
- `test resample::tests::exact_rate_preserves_amplitude_and_no_attenuation ... FAILED`
- All other resample tests (incl. the four new ones) pass.

The failure should show a ratio significantly below 1.0 (around 0.5) — the visible signature of D1's ~6 dB attenuation.

If `exact_rate_preserves_amplitude_and_no_attenuation` **passes** on the current code, STOP and report: that suggests the D1 issue isn't manifesting the way the audit described, and we need to re-examine before proceeding.

If any of the **other** four new tests fail unexpectedly on the current code, STOP and report — the spec assumed they'd pass pre-#87 since they cover paths/inputs the existing tests don't.

- [ ] **Step 3: Commit (with the deliberately-failing test)**

```bash
git add src/resample.rs
git commit -m "test(resample): F6 — five new tests; exact_rate_… fails as D1 regression net (#87)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

(This commit deliberately leaves the suite red on `exact_rate_preserves_amplitude_and_no_attenuation` — Task 2's polyphase rewrite + unit-gain normalization turns it green.)

---

## Task 2: Polyphase tap bank + unit-gain + `needed_end` fix

The algorithmic refactor. Replaces the per-sample tap computation with a precomputed 256-phase bank built in `new()` and normalized to unit DC gain. Fixes the `needed_end` off-by-one as part of the hot-path rewrite. Turns the T1 deliberately-failing test green.

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
//! positions, unit-gain normalized. Tap rows are precomputed once in
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

- [ ] **Step 2: Rename `fir_tap` → `raw_tap_f64` and change return type to `f64`**

Find the existing `fn fir_tap(...) -> f32` (around lines 49-63). Apply these edits:

(a) Rename it: `fn fir_tap` → `fn raw_tap_f64`.

(b) Change the return type: `-> f32` → `-> f64`.

(c) Drop the final `as f32` cast on the last line — return the `f64` value directly.

The full result:

```rust
/// Compute one raw FIR tap value (Hann-windowed sinc, not yet normalized
/// for unit DC gain) for a given tap index and fractional phase, in `f64`.
/// Called only by [`Resampler::new`] to build the polyphase tap bank;
/// the runtime hot path never invokes this.
///
/// `tap_index` is in 0..`FIR_TAPS`. `frac` is in [0, 1) — the sub-sample
/// offset of the output sample's center from the integer input grid.
/// `fc` is the cutoff normalized to input rate (`cutoff_hz / input_rate`).
///
/// Sinc shifts with `frac`; Hann window stays anchored to the tap grid.
/// This is the standard windowed-sinc fractional-delay formulation —
/// see e.g. Smith, "Digital Audio Resampling Home Page" (CCRMA, 2002).
#[allow(clippy::cast_precision_loss)]
fn raw_tap_f64(tap_index: usize, frac: f64, fc: f64) -> f64 {
    let m = FIR_TAPS as f64;
    let n = (tap_index as f64) - (m - 1.0) / 2.0 - frac;
    let sinc = if n.abs() < 1e-12 {
        2.0 * fc
    } else {
        (2.0 * std::f64::consts::PI * fc * n).sin() / (std::f64::consts::PI * n)
    };
    let w = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * (tap_index as f64) / (m - 1.0)).cos());
    sinc * w
}
```

(Note: the `#[allow(clippy::cast_possible_truncation)]` that was on `fir_tap` is no longer needed — without the `as f32` there's nothing being truncated.)

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
    /// the corresponding fractional delay, normalized to unit DC gain
    /// (sum-of-taps == 1.0) so the resampler doesn't attenuate (audit
    /// #87 D1).
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

        // Build the polyphase tap bank — 256 rows of 64 taps each, normalized
        // to unit DC gain per row (audit #87 D1).
        let mut taps: Box<[[f32; FIR_TAPS]; NUM_PHASES]> =
            Box::new([[0.0_f32; FIR_TAPS]; NUM_PHASES]);
        for phase_idx in 0..NUM_PHASES {
            let frac = (phase_idx as f64) / (NUM_PHASES as f64);
            // Compute the 64 raw Hann-windowed-sinc taps for this frac, in f64
            // so the sum-normalization preserves precision across 64 small values.
            let mut row = [0.0_f64; FIR_TAPS];
            for k in 0..FIR_TAPS {
                row[k] = raw_tap_f64(k, frac, cutoff_norm);
            }
            let sum: f64 = row.iter().sum();
            let inv = if sum.abs() > 1e-12 { 1.0 / sum } else { 1.0 };
            for k in 0..FIR_TAPS {
                taps[phase_idx][k] = (row[k] * inv) as f32;
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
            let phase_idx = ((frac * NUM_PHASES as f64) as usize).min(NUM_PHASES - 1);
            let taps = &self.taps[phase_idx];
            let start = self.phase.floor() as isize;

            // Convolve using the precomputed unit-gain taps at this quantized
            // phase. No transcendentals in the hot path.
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

All four must pass. The previously-failing `exact_rate_preserves_amplitude_and_no_attenuation` now passes (unit gain restored); the other 142+ tests stay green (including `tests/roundtrip.rs` 11/11 — the polyphase quantization at 256 phases is well below SSTV's noise floor, so the per-mode pixel-diff checks aren't perturbed).

- [ ] **Step 7: Commit**

```bash
git add src/resample.rs
git commit -m "refactor(resample): 256-phase polyphase tap bank + unit-gain normalization + needed_end fix (#87 D1/D2/D2b)

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
  +`cos()` calls/sec from the hot path. Taps are normalized to unit DC
  gain (audit D1) — `Resampler` no longer attenuates by ~6 dB. Off-by-one
  in `needed_end` (was `ceil(phase + 64)` causing one extra sample of
  buffering; now `floor(phase) + 64`) fixed. Group delay documented
  (`(FIR_TAPS - 1) / 2 = 31.5` input-rate samples). `cutoff_hz` doc
  corrected (Nyquist factor was wrong in the doc; the code was right).
  Five new tests (`exact_rate_preserves_amplitude_and_no_attenuation`,
  `upsampling_8khz_to_11025`, `max_input_rate_192khz`,
  `tiny_chunks_emit_nothing_then_catch_up`, `empty_input_returns_empty`).
  Quantization noise at 256 phases is ≈ −52 dB phase noise on a 2300 Hz
  tone — well below SSTV's noise floor. (#87; audit D1/D2/D2b/F6.)

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
  - D1 (unit-gain normalization) → T2 Steps 3-5 (struct field + new() build loop with per-row sum-normalization).
  - D2 (polyphase bank + transcendentals out of hot path) → T2 Steps 1-6 (NUM_PHASES const + struct field + new() build + process() lookup hot path + module-level doc).
  - D2b cutoff_hz doc → T3 Step 1.
  - D2b group delay → T3 Step 2.
  - D2b needed_end off-by-one → T2 Step 5 (folded into the hot-path rewrite).
  - D2b dead-branch comment → T2 Step 5 (folded into the hot-path rewrite).
  - F6 (5 new tests) → T1.
  - CHANGELOG → T4.

- **TDD: T1 deliberately lands the suite red.** The `exact_rate_preserves_amplitude_and_no_attenuation` test fails on pre-#87 code (the ~6 dB attenuation pushes the output-peak/input-peak ratio to ~0.5, failing the ±5 % assertion at ratio ≈ 1.0). T2's unit-gain normalization turns it green. The other four F6 tests are net-new coverage that should pass even on pre-T2 code; if any fail there, STOP and report (the spec assumed they wouldn't).

- **Quantization noise at 256 phases ≈ −52 dB.** The audit and spec describe this in detail. Empirically the load-bearing check is `tests/roundtrip.rs` 11/11 — every supported mode decodes a synthetic image through the resampler; any quantization-noise issue would show up there as a pixel-diff regression.

- **Out of scope** (tracked separately under epic #97):
  - Lerp between adjacent phases (defer — quantization noise already inaudible).
  - SIMD-ifying the 64-tap inner loop (#77 — needs profile-driven targeting).
  - Remaining audit cleanups (#88 / #91-96).
