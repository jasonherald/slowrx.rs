# slowrx C ↔ Rust parity audit — round 2

Date: 2026-05-02
Audit target: slowrx.rs commit `2ca0edb` (post Phases 1-4)
Reference: `/home/jherald/source/rtl-sdr/original/slowrx` (ISC)

## Summary

11 new findings: 0 critical, 1 important, 5 minor, 5 cosmetic.

The biggest find: the YCbCr→RGB conversion in `mode_pd::ycbcr_to_rgb` was NOT
fixed by PR #47 (which only fixed `freq_to_luminance`). It still uses C-style
integer division (truncation toward zero) while slowrx uses float-divide
followed by `clip()` (round-to-nearest). This causes a 1-LSB darker bias on
every R/G/B channel of every YCbCr-decoded pixel — which is every pixel in PD
mode. **Round 1 finding #4 was reported "fixed" but it was only half-fixed.**

The other notable cluster: every place that translates slowrx's `GetBin(hz,
fft_len)` (which uses C double-to-int truncation) into a Rust `bin_for(hz)`
lambda, the Rust code uses `.round()` instead of truncation. This is
mathematically more correct (round-to-nearest beats truncate-toward-zero for
bin selection) but it diverges from slowrx for several frequencies — most
visibly at 1500 Hz (slowrx 34, Rust 35) and 800 Hz (slowrx 18, Rust 19) — and
that propagates into the SNR estimator's bandwidth-correction divisors.

Subsystems audited:
- `vis.c::GetVIS` ↔ `vis.rs::VisDetector`
- `common.c::{GetBin, power, clip, deg2rad}` ↔ inline lambdas across crate
- `modespec.c` ↔ `modespec.rs`
- `sync.c::FindSync` ↔ `sync.rs::find_sync` + `SyncTracker`
- `video.c::GetVideo` ↔ `mode_pd.rs::decode_pd_line_pair` + `decode_one_channel_into`
- `video.c` SNR estimator ↔ `snr.rs::SnrEstimator`
- `video.c` Hann bank + adaptive selector ↔ `snr.rs::HannBank` + `window_idx_for_snr`
- `slowrx.c::Listen` ↔ `decoder.rs::SstvDecoder::process`
- Resampler (no slowrx analog, sanity-checked end-to-end)

## Side-by-side coverage map

For each major area, one-line state. (Re-checked round-1 deferrals; rationales
still hold.)

### `vis.c` ↔ `src/vis.rs`
- Hann window construction. Checked. Match (`build_hann_window` ↔ `vis.c:30`).
- FFT plan (FFT_LEN=512 ↔ slowrx 2048, both → 21.5 Hz/bin). Checked. Match.
- Hop scan loop (10 ms hop, 20 ms window). Checked. Match.
- 9-iteration alignment (i × j). Checked. Match.
- Bit decoding + parity. Checked. **Finding 5** (parity-failure outer-loop break
  divergence — Rust is more permissive than slowrx's buggy behavior).
- HedrShift extraction. Checked. Match.
- Stop-bit detection + residual buffer. Checked. **Finding 11** (stop-bit-end
  formula gives 5 ms offset uniformly across `i` in Rust vs slowrx's 5–25 ms
  range — non-material because find_sync corrects).
- Post-stop-bit handling. Checked. See round-1 #39 (still intentional).
- Round-1 `take_residual_buffer` re-anchor contract (#40). Re-verified: still
  honored — `decoder.rs::process` always replaces `self.vis = VisDetector::new()`
  after ImageComplete.

### `sync.c` ↔ `src/sync.rs`
- SyncTracker `Praw`/`Psync`. Checked. **Finding 4** (Praw normalization off by
  one bin from slowrx, propagated from `bin_for` rounding).
- Hough transform shape. Checked. Match.
- Slant rate update + retry loop. Checked. **Finding 7** (slant-lock interval
  half-open `[89, 91)` in Rust vs open `(89, 91)` in slowrx).
- Falling-edge convolution. Checked. **Finding 6** (`xmax` init `i32::MIN` vs
  slowrx `int xmax=0` and `maxconvd=0` causes degenerate-input divergence).
- Skip composition (PD-only branch). Checked. Match.
- Round-1 #42 (90° deadband). Re-verified: rationale unchanged — without the
  deadband, half-degree Hough quantization noise creates 0.0085% rate
  perturbation that compounds across 248 line pairs. Still correct.

### `video.c` ↔ `src/mode_pd.rs` + `src/snr.rs`
- Channel time formulas. Checked. Match (`chan_starts_sec` formula tracks
  `video.c:88-92` after PR #47's `septr_seconds` field).
- StoredLum sweep (per-channel). Checked. Match (Phase 3 rewrite).
- SNR estimator (formula). Checked. **Finding 2** (bandwidth-correction
  divisors differ from slowrx by 2.2% – 8.4% due to `bin_for` round vs trunc).
- Adaptive Hann (window selection). Checked. See round-1 #44 (still
  intentional — currently hard-coded `win_idx = 6`; rationale unchanged because
  the synthetic encoder still produces hard tonal cliffs).
- Per-FFT loop with PIXEL_FFT_STRIDE = 1. Checked. Match in spirit.
- StoredLum→pixel sampling. Checked. Match.
- YCbCr→RGB matrix. Checked. **Finding 1** (integer division truncates while
  slowrx uses float divide + round — round-1 #4 was only partially fixed).
- HedrShift pixel-band shift. Checked. Match (PR #41 applied).

### `common.c` ↔ scattered Rust
- `clip` ↔ `freq_to_luminance`. Checked. Match after PR #47.
- `clip` ↔ `ycbcr_to_rgb`. Checked. **Finding 1** (NOT fixed by PR #47).
- `power(coeff)` (= r² + i²). Checked. Match (inline in 4 sites).
- `GetBin(hz, fft_len)`. Checked. **Finding 2/3/4** (round vs trunc divergence
  at 4 sites: `vis.rs::estimate_peak_freq`, `mode_pd.rs::pixel_freq`,
  `snr.rs::estimate`, `sync.rs::SyncTracker::new`).
- `deg2rad`. Checked. Match.

### `modespec.c` ↔ `src/modespec.rs`
- `septr_seconds` field added by PR #47. Re-verified for PD120/PD180 = 0e-3.
  Match `modespec.c::SeptrTime`.
- VIS code lookup. Checked. Match.
- All numeric fields (SyncTime / PorchTime / PixelTime / LineTime / ImgWidth /
  NumLines). Checked. Match.

### `slowrx.c::Listen` ↔ `src/decoder.rs`
- State transitions. Checked. Match.
- Reset behavior. Checked. Match.
- Buffer management. Checked. Match (residual transfer pattern is correct).
- Sample counting. Checked (round-1 #29/#34 deferred — informational only).
- Round-1 #31 (back-to-back VIS bursts). Re-verified: trailing audio is fed
  into a fresh `VisDetector` after ImageComplete. Match contract.

### Resampler (`src/resample.rs`)
- 64-tap polyphase FIR. Checked.
- Round-trip tone preservation in tests. Verified.
- Group delay / FIR transient. See round-1 finding #18 (still applies to
  burst-at-audio[0] edge case; not changed since round 1).

---

## Findings

### Finding 1 — YCbCr→RGB conversion uses integer division (round-1 #4 only half-fixed)

**Severity:** Important
**Likely impact on real-audio decoder failure:** No (image quality)
**Files:** `common.c:49-53` + `video.c:447-451` ↔ `src/mode_pd.rs:46-60`

#### Context

PR #47 fixed the luminance code path (`freq_to_luminance`) so it uses
`v.clamp(0, 255).round() as u8`, matching slowrx's `clip(double)` in
`common.c:49-53` (`(guchar)round(a)`). Round-1 finding #4 explicitly called out
the YCbCr→RGB conversion as having "pre-divide truncation, slightly different
math" — but the fix in PR #47 only covered `freq_to_luminance`. The YCbCr path
still uses C-style integer division.

The Rust source comment at `mode_pd.rs:52` says:
> Integer division truncates toward zero (matches slowrx's `(int)` cast).

This comment is **factually wrong**. Slowrx does NOT use an `(int)` cast on the
YCbCr conversion — it uses `/ 100.0` (float division by a `double` literal,
producing a `double`) and then passes the `double` to `clip()` which calls
`round()`.

#### C source (slowrx reference)

```c
// common.c:49-53
guchar clip (double a) {
  if      (a < 0)   return 0;
  else if (a > 255) return 255;
  return  (guchar)round(a);
}

// video.c:447-451
case YUV:
  p[0] = clip((100 * Image[tx][y][0] + 140 * Image[tx][y][1] - 17850) / 100.0);
  p[1] = clip((100 * Image[tx][y][0] -  71 * Image[tx][y][1] - 33 *
      Image[tx][y][2] + 13260) / 100.0);
  p[2] = clip((100 * Image[tx][y][0] + 178 * Image[tx][y][2] - 22695) / 100.0);
```

`100 * Image[…]` is `int` arithmetic (guchar promotes to int). `/ 100.0` is
double division (the literal forces float promotion). The result is `double`,
passed to `clip(double)` which `round()`s before clamping.

#### Rust source (current state)

```rust
// mode_pd.rs:46-60
#[must_use]
#[doc(hidden)]
pub fn ycbcr_to_rgb(y: u8, cr: u8, cb: u8) -> [u8; 3] {
    let yi = i32::from(y);
    let cri = i32::from(cr);
    let cbi = i32::from(cb);
    // i32 multiplications: max magnitude is 255 * 178 = 45_390, well within i32.
    // Integer division truncates toward zero (matches slowrx's `(int)` cast).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let r = ((100 * yi + 140 * cri - 17_850) / 100).clamp(0, 255) as u8;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let g = ((100 * yi - 71 * cri - 33 * cbi + 13_260) / 100).clamp(0, 255) as u8;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let b = ((100 * yi + 178 * cbi - 22_695) / 100).clamp(0, 255) as u8;
    [r, g, b]
}
```

Rust uses Rust's `i32 / i32` integer division which truncates toward zero.

#### Quantitative example

Y=128, Cr=128, Cb=128 (neutral grey):

| Channel | Numerator | slowrx (`/100.0` then `round`) | Rust (`/100` integer) | Diff |
|---|---|---|---|---|
| R | 12870 | 128.7 → 129 | 128 | 1 |
| G | 12748 | 127.48 → 127 | 127 | 0 |
| B | 12889 | 128.89 → 129 | 128 | 1 |

Y=178, Cr=100, Cb=100:

| Channel | Numerator | slowrx | Rust | Diff |
|---|---|---|---|---|
| R | 13950 | 139.5 → 140 | 139 | 1 |

#### Why this matters

Every YCbCr-decoded pixel has a 1-LSB darker bias on average. For PD modes (the
only modes implemented), this affects every pixel in every image. Cumulative
visual effect: image is consistently ~0.5 luminance units dimmer than slowrx
would produce. Side-by-side comparison would show a slight darker tint and
slightly off colors, particularly in mid-grey regions where the rounding error
is most consistent.

The synthetic round-trip test (`tests/roundtrip.rs`) currently has a `≤ 25` max
diff and `< 5` mean diff threshold which is loose enough to mask this issue.
The round-trip test uses `ycbcr_to_rgb` on BOTH ends (encoder and decoder),
which masks the truncation error completely — both ends have the same bias.
A real-radio capture compared to slowrx's output would surface this.

#### Acceptance criteria

- `ycbcr_to_rgb` rounds to nearest, not truncates: e.g.,
  `((100*yi + 140*cri - 17_850) as f64 / 100.0).round().clamp(0.0, 255.0) as u8`
  — or, equivalently, add 50 before truncating positive numerators (with care
  for negative values).
- Update the misleading comment from "matches slowrx's `(int)` cast" to "matches
  slowrx's `clip()` round-to-nearest in `common.c:49-53`".
- Add a parity test that picks a Y/Cr/Cb combination producing a fractional
  result (e.g., Y=128 Cr=128 Cb=128) and asserts the R/B channels are 129, not
  128 (matching slowrx's float-then-round).

#### Related

Round 1 finding #4. PR #47 was supposed to close this but only addressed
`freq_to_luminance`.

---

### Finding 2 — `bin_for` lambdas use `.round()` while slowrx's `GetBin` uses C truncation; bandwidth correction in SNR estimator off by 2-9%

**Severity:** Important
**Likely impact on real-audio decoder failure:** No (degrades SNR estimate, which currently doesn't drive window selection anyway — but will once round-1 #44 is engaged)
**Files:** `common.c:39-41` ↔ `src/mode_pd.rs:139-142`, `src/snr.rs:180-184`, `src/sync.rs:89-92`, `src/vis.rs:271-273`

#### Context

slowrx's `GetBin(Freq, FFTLen)` is a single-line function:

```c
guint GetBin (double Freq, guint FFTLen) {
  return (Freq / 44100 * FFTLen);
}
```

The implicit conversion `double → guint` in C **truncates toward zero**. So
`GetBin(1500, 1024) = (guint)(1500 / 44100 * 1024) = (guint)(34.83) = 34`.

Rust translates this with `.round()` everywhere. There are 4 distinct
`bin_for` lambdas in the crate — `mode_pd.rs:139`, `snr.rs:180`,
`sync.rs:89`, `vis.rs:271` — each using `.round() as usize`. This is a
systematic deviation from slowrx's truncation semantics.

The downstream impact varies by site. The most consequential is the SNR
estimator's bandwidth-correction divisors (Finding 3 below); the other sites
have edge cases where the round-vs-truncate flips a single bin and slightly
shifts the analysis range.

#### Specific Rust deviation table (FFT_LEN=256 at 11025 Hz; slowrx FFT_LEN=1024 at 44100 Hz; same Hz/bin)

| Frequency | slowrx GetBin (trunc) | Rust bin_for (round) | Diff |
|---|---|---|---|
| 400 Hz   | 9  | 9  | 0  |
| 800 Hz   | 18 | 19 | +1 |
| 1200 Hz  | 27 | 28 | +1 |
| 1500 Hz  | 34 | 35 | +1 |
| 1900 Hz  | 44 | 44 | 0  |
| 2300 Hz  | 53 | 53 | 0  |
| 2700 Hz  | 62 | 63 | +1 |
| 3400 Hz  | 78 | 79 | +1 |

The pattern: when `Freq * FFT/SR` has fractional part > 0.5, slowrx truncates
DOWN while Rust rounds UP. Five of the eight production frequencies exhibit
the divergence.

#### C source (slowrx reference)

```c
// common.c:39-41
guint GetBin (double Freq, guint FFTLen) {
  return (Freq / 44100 * FFTLen);
}
```

#### Rust source (current state)

```rust
// mode_pd.rs:139-142
let bin_for = |hz: f64| -> usize {
    (hz * (FFT_LEN as f64) / f64::from(crate::resample::WORKING_SAMPLE_RATE_HZ)).round()
        as usize
};
```

Same pattern in `snr.rs:180-184`, `sync.rs:89-92`, `vis.rs:271-273`.

#### Why this matters

By itself, a 1-bin shift on 5 of 8 frequencies is small. But it compounds:

1. **SNR estimator bandwidth correction** (see Finding 3 — broken out for
   visibility): slowrx's `Pnoise = Pnoise_only * (receiver_bins/noise_only_bins)`
   is `Pnoise_only * (69/27) = Pnoise_only * 2.5556` while Rust computes
   `Pnoise_only * (70/28) = Pnoise_only * 2.5000`. **Rust biases SNR
   estimates downward by ~0.1 dB** on the same audio. Same for the
   `Psignal_subtractor`: slowrx 20/27=0.7407, Rust 19/28=0.6786 — Rust
   subtracts 8% LESS noise contribution, biasing Psignal UP. Net SNR error
   is small but real.

2. **`SyncTracker.video_lo_bin` excludes bin 34** (1464.5 Hz) which slowrx
   includes. For radios mistuned ~30 Hz LOW so the actual video range is
   ~1470-2270 Hz, slowrx captures the lower edge that Rust misses.

3. **`SyncTracker.sync_target_bin` is bin 28 in Rust vs 27 in slowrx**. Bin 28
   = 1205.7 Hz is actually CLOSER to 1200 Hz than bin 27 = 1162.7 Hz, so
   here Rust is mathematically more correct. But it's a divergence from
   slowrx's behavior, so a real-audio capture decoded by both should expect a
   measurable Psync difference.

4. **Per-pixel `pixel_freq` peak search range**: lo and hi are both `±1`-padded
   in slowrx and Rust, so the round-vs-trunc difference is partially absorbed
   by the padding. But the inclusive `[lo, hi]` range is shifted up by 1 in
   Rust, so on the LOW end Rust misses bin 33 = 1421 Hz which slowrx includes
   (within the +1 padded boundary). This matters only for radios with extreme
   negative HedrShift.

#### Acceptance criteria

- Decide whether parity (`(... ).floor() as usize` or `as usize` direct) or
  correctness (current `.round()`) is the policy. The *parity* answer is to
  match slowrx exactly and use `as usize` truncation throughout. The
  *correctness* answer is to keep `.round()` and document it.
- Either way, all 4 sites should use the same convention. Right now they're
  consistent (all use `.round()`), so the choice is policy.
- If keeping `.round()`, document the divergence in each lambda's enclosing
  doc comment so future maintainers don't think it matches slowrx.

#### Related

Independent of round-1 findings. Surfaces because of the multi-site bin lookup
abstraction.

---

### Finding 3 — SNR bandwidth correction divisors: 2.2 % – 8.4 % off from slowrx

**Severity:** Minor
**Likely impact on real-audio decoder failure:** No (downstream consumer is currently disengaged via round-1 #44)
**Files:** `video.c:329-338` ↔ `src/snr.rs:215-227`

#### Context

A direct consequence of Finding 2 above, broken out because the bandwidth
correction is the most numerically visible site of the round/trunc divergence.

The slowrx SNR formula is:

```
P_noise = P_noise_only × (receiver_bins / noise_only_bins)
P_signal = P_video_plus_noise − P_noise_only × (video_plus_noise_bins / noise_only_bins)
SNR = 10 × log10(P_signal / P_noise) [floored at -20 dB]
```

The `*_bins` counts come from `GetBin(hi) - GetBin(lo) + 1` style calculations.
With the bin discrepancies from Finding 2, the divisors and multipliers come
out differently:

| Quantity | slowrx (trunc) | Rust (round) | Ratio |
|---|---|---|---|
| `video_plus_noise_bins` (1500..2300) | 53−34+1 = 20 | 53−35+1 = 19 | 0.95 |
| `noise_only_bins` (400..800 ∪ 2700..3400) | (18−9+1) + (78−62+1) = 27 | (19−9+1) + (79−63+1) = 28 | 1.04 |
| `receiver_bins` (3400 − 400) | 78−9 = 69 | 79−9 = 70 | 1.014 |
| Pnoise multiplier (rec/noise) | 69/27 = 2.5556 | 70/28 = 2.5000 | 0.978 |
| Psignal subtractor (vid/noise) | 20/27 = 0.7407 | 19/28 = 0.6786 | 0.916 |

So `Pnoise` is computed slightly LOWER in Rust (×0.978), and the noise
subtraction from `Pvideo+noise` is LOWER (×0.916) → `Psignal` is HIGHER.
Net effect: Rust's SNR estimate is ~0.1–0.5 dB higher than slowrx's on
identical audio.

#### Why this matters

The round-1 #44 deferral means `win_idx` is currently hard-coded to 6
(longest window) regardless of the SNR estimate. So this finding has zero
effect on Rust's current decode quality. **However:** when round-1 #44 is
engaged (the adaptive window selection), the 0.1-0.5 dB upward bias may
shift the window-index decision boundary on borderline-SNR audio. For SNR
near the 9.0 dB threshold (between WinIdx 1 and 2) or the 3.0 dB threshold
(between WinIdx 3 and 4), the bias could flip the selection.

#### Acceptance criteria

Tied to the resolution of Finding 2. If `bin_for` switches to truncation,
this finding closes automatically. If `.round()` is kept, document the
SNR bias in the SNR-estimator's doc comment so future tuning of round-1 #44
accounts for it.

---

### Finding 4 — `SyncTracker` Praw normalization integrates one fewer bin than slowrx

**Severity:** Minor
**Likely impact on real-audio decoder failure:** No
**Files:** `video.c:282-288` ↔ `src/sync.rs:138-146`

#### Context

This is another consequence of Finding 2, surfaced separately because the
Sync tracker is a different code path from the SNR estimator.

`Praw` is the average per-bin power across the video band, used as the
denominator in the `Psync > 2 × Praw` decision that fills the `has_sync`
track for FindSync.

slowrx (`video.c:282-288`):
```c
for (i = GetBin(1500+HedrShift, FFTLen); i <= GetBin(2300+HedrShift, FFTLen); i++)
  Praw += power(fft.out[i]);
...
Praw /= (GetBin(2300+HedrShift, FFTLen) - GetBin(1500+HedrShift, FFTLen));
```

For HedrShift=0: integrates bins [34, 53] = 20 bins, normalizes by 19.
**slowrx already has an off-by-one in the divisor (`hi - lo` instead of
`hi - lo + 1`).**

#### Rust source (current state)

```rust
// sync.rs:138-146
let mut p_raw = 0.0_f64;
let lo = self.video_lo_bin.max(1);
let hi = self.video_hi_bin.min(SYNC_FFT_LEN / 2 - 1);
if hi >= lo {
    for k in lo..=hi {
        p_raw += power(self.fft_buf[k]);
    }
    p_raw /= (hi - lo).max(1) as f64;
}
```

Rust integrates `[35, 53]` = 19 bins, divides by 18. **Rust preserves slowrx's
divisor off-by-one**, but with different bin counts.

#### Quantitative impact

For a uniform-power-density input across the video band:
- slowrx Praw = (20 × P_per_bin) / 19 = 1.0526 × P_per_bin
- Rust Praw   = (19 × P_per_bin) / 18 = 1.0556 × P_per_bin

Difference: ~0.3 % — basically nothing.

For a single-tone input at exactly 1500 Hz (e.g., a porch tone): slowrx's
bin 34 captures most of the power (since 1500 Hz lands at bin 34.83 in
slowrx's quantization); Rust's bin 35 sees half the power (sidelobe). So
slowrx's Praw for a 1500 Hz tone is ~2× higher than Rust's.

In real audio the porch is ~2 ms long (PD120: PorchTime = 2.08e-3) — only a
tiny fraction of the FFT analysis window — so the discrepancy averages out
across many sync probes.

#### Why this matters

Almost certainly doesn't affect decode outcome on real audio. Worth knowing
about because it's a *systematic* shift in `has_sync` state in noisy regions
near 1500 Hz (the porch frequency).

#### Acceptance criteria

Same as Finding 2: if `bin_for` switches to truncation, this closes. Keep the
deliberate divisor off-by-one (slowrx behavior) regardless of the rounding
policy.

---

### Finding 5 — VIS `match_vis_pattern` retries all (i, j) on parity failure; slowrx terminates after the first pattern match

**Severity:** Minor
**Likely impact on real-audio decoder failure:** Likely no, but slightly different on edge-case noise
**Files:** `vis.c:80-130` ↔ `src/vis.rs:319-365`

#### Context

The slowrx VIS detection loop has a subtle behavioral quirk in `vis.c:82-83`:

```c
for (i = 0; i < 3; i++) {
  if (CurrentPic.HedrShift != 0) break;   // ← outer-loop early exit
  for (j = 0; j < 3; j++) {
    if ( /* pattern matches */ ) {
      gotvis = TRUE;
      // ... bit decode ...
      if (gotvis) {
        CurrentPic.HedrShift = tone[0+j] - 1900;   // ← sets HedrShift
        // ... parity check ...
        if (Parity != ParityBit) gotvis = FALSE;   // ← parity fail, but HedrShift remains set
        else if (VISmap[VIS] == UNKNOWN) gotvis = FALSE;
        else { /* good */ break; }
      }
    }
  }
}
```

If the pattern matches at (i=0, j=0) but parity FAILS, slowrx sets
`HedrShift = tone[0] - 1900` and resets `gotvis = FALSE`. The inner loop
continues; if no other (j) match within (i=0), the inner loop ends. The OUTER
loop's `if (HedrShift != 0) break;` then fires for (i=1), terminating the
search. **A second valid (i, j) candidate that would have passed parity is
NOT tried — UNLESS** `tone[0+j]` happened to be exactly 1900 Hz (HedrShift
== 0), in which case the outer break does NOT fire and slowrx continues
trying. So the divergence only triggers when the radio is mistuned (HedrShift
nonzero) AND the first pattern match has parity failure.

This is almost certainly a slowrx C bug (the HedrShift-set-before-parity-check
pattern is suspicious), but it's the actual reference behavior on mistuned
radios.

#### C source

(See above.)

#### Rust source (current state)

```rust
// vis.rs:319-365
fn match_vis_pattern(tones: &[f64; HISTORY_LEN]) -> Option<(u8, f64, usize)> {
    let tol = TONE_TOLERANCE_HZ;
    for i in 0..3 {
        for j in 0..3 {
            let leader = tones[j];
            if !within(tones[3 + i], leader, tol) /* ... */ {
                continue;
            }
            // ... bit decode + parity ...
            let mut bit_ok = true;
            for k in 0..8 {
                /* ... */
                if k < 7 {
                    code |= bit << k;
                    parity ^= bit;
                } else if parity != bit {
                    bit_ok = false;
                }
            }
            if bit_ok {
                return Some((code, leader - LEADER_HZ, i));
            }
        }
    }
    None
}
```

Rust correctly tries all 9 (i, j) combinations before returning None. There is
no analog of slowrx's HedrShift-leak-out-of-inner-loop quirk.

#### Why this matters

Edge case: the FIRST pattern match has parity fail, and a LATER (i, j) would
have passed both pattern + parity. Rust accepts the later match; slowrx
discards both and re-loops 10 ms later for another attempt.

In practice this is rare — if the first 14 hops formed a coherent VIS pattern,
parity is likely correct. Parity-fail-then-good-match within the same 45-hop
history would require the bit pattern to be ambiguous between two adjacent
bit-aligned phases, which is unusual.

But it IS a divergence from slowrx semantics. Worth noting.

#### Acceptance criteria

- Document the divergence in `match_vis_pattern`'s doc comment so future
  maintainers don't conclude it matches slowrx 1:1.
- Keep the Rust behavior — slowrx's is buggy.

#### Related

Independent of round-1 findings. Comes from carefully reading the slowrx
control flow in `vis.c:80-130`.

---

### Finding 6 — `find_sync` falling-edge convolution: degenerate (zero) input gives `xmax=4` in Rust vs `xmax=0` in slowrx

**Severity:** Cosmetic
**Likely impact on real-audio decoder failure:** No
**Files:** `sync.c:29-30, 105-113` ↔ `src/sync.rs:303-316`

#### Context

The `xmax` initialization differs:

slowrx (`sync.c:29-30`):
```c
double convd, maxconvd=0;
int    xmax=0;
```

Rust (`sync.rs:303-305`):
```rust
let kernel: [i32; 8] = [1, 1, 1, 1, -1, -1, -1, -1];
let mut xmax: i32 = 0;
let mut max_convd: i32 = i32::MIN;
```

slowrx's `maxconvd = 0` means a `convd <= 0` for every `x` (which happens for
zero or constant `xAcc`) leaves `xmax = 0`. Rust's `max_convd = i32::MIN`
means **any** `convd` value (including 0) at the FIRST `x` triggers the
update, making `xmax = 4`.

#### Why this matters

For a totally-empty `has_sync` array (no detected sync pulses across the
entire image — pathological case):

- slowrx: `xmax = 0` → `s = 0/700 × line_seconds − sync_seconds = −sync_seconds`
  → `Skip ≈ −20 ms × Rate`.
- Rust: `xmax = 4` → `s = 4/700 × line_seconds − sync_seconds = ε − sync_seconds`
  → `Skip ≈ (4/700 × line_seconds − 0.020) × rate`. About `−19 ms × Rate`.

So Rust's degenerate-case Skip is ~1 ms different from slowrx's. Both produce
negative Skip values that read zero-padded audio as line 0. Functional
outcome (a totally-corrupted image) is the same.

#### Acceptance criteria

- Initialize `max_convd = 0_i32` to match slowrx's semantic. (Or document why
  the divergence is intentional.) The Rust comment doesn't currently note this.

---

### Finding 7 — Slant-lock interval: Rust uses half-open `[89, 91)`; slowrx uses open `(89, 91)`

**Severity:** Cosmetic
**Likely impact on real-audio decoder failure:** No
**Files:** `sync.c:83` ↔ `src/sync.rs:284`

#### Context

slowrx (`sync.c:83`):
```c
if (slantAngle > 89 && slantAngle < 91) {
  // locked
  break;
}
```

Open interval (89, 91). At slant=89.0° exactly, slowrx does NOT break — it
applies the rate correction and retries.

Rust (`sync.rs:284`):
```rust
if (SLANT_OK_LO_DEG..SLANT_OK_HI_DEG).contains(&slant_angle) || retry == MAX_SLANT_RETRIES {
    break;
}
```

`SLANT_OK_LO_DEG = 89.0`, `SLANT_OK_HI_DEG = 91.0`. The Rust range syntax
`89.0..91.0` is half-open: `[89.0, 91.0)`. So at slant=89.0° exactly, Rust
treats as locked and breaks — slowrx does not.

#### Why this matters

The Hough quantization is 0.5° per bin. Slant angles are discretized to
{30.0, 30.5, ..., 89.0, 89.5, 90.0, 90.5, ..., 149.5}. The angles that
fall within the locked range are:

- slowrx (89, 91): 89.5, 90.0, 90.5 = 3 bins
- Rust [89, 91):    89.0, 89.5, 90.0, 90.5 = 4 bins

So Rust is ~33% MORE permissive at the lower edge. For real audio this
slightly biases toward "locked early" (less retry, less rate correction).

In practice, decodes that lock at 89.0° exactly are rare and the rate
correction at 89.0° is tiny anyway.

#### Acceptance criteria

- Match slowrx exactly: replace `(SLANT_OK_LO_DEG..SLANT_OK_HI_DEG).contains(&slant_angle)`
  with `slant_angle > SLANT_OK_LO_DEG && slant_angle < SLANT_OK_HI_DEG` — OR
  document why the divergence is intentional.

---

### Finding 8 — Sync probe stride: Rust 3 samples@11025 (≈272 µs) vs slowrx 13 samples@44100 (≈295 µs); ~7.7% finer probing

**Severity:** Cosmetic
**Likely impact on real-audio decoder failure:** No
**Files:** `video.c:295` (`NextSyncTime += 13`) ↔ `src/sync.rs:32` (`SYNC_PROBE_STRIDE = 3`)

#### Context

slowrx samples `HasSync[]` every 13 audio samples at 44100 Hz = 0.2948 ms /
probe. Rust uses `SYNC_PROBE_STRIDE = 3` at 11025 Hz = 0.2721 ms / probe.

Equivalence: slowrx's 13 / 44100 = 0.000295 s; Rust's 3 / 11025 = 0.000272 s.
Rust probes ~7.7 % more often.

#### Rust source (current state)

```rust
/// Stride between sync-band probes (working-rate samples). slowrx uses
/// 13 samples@44.1kHz (`video.c:295`) ≈ 3.25@11.025kHz; we round to 3.
/// Index math in [`find_sync`] scales by `SYNC_PROBE_STRIDE` for parity.
pub(crate) const SYNC_PROBE_STRIDE: usize = 3;
```

The comment correctly states the rationale (rounding 3.25 down to 3).

#### Why this matters

The `has_sync` array is denser in Rust (~7.7 %), so the Hough transform's
`SyncImg[700][630]` has marginally more samples mapping to each cell. Edge
cases at the boundary of "did/didn't see a sync pulse here?" might tilt
slightly differently.

The slant correction itself is essentially sample-rate independent: the Hough
finds lines through the 2D image regardless of how dense the input pixels are.

#### Acceptance criteria

- Either accept the 7.7% difference (document it) or use `SYNC_PROBE_STRIDE = 4`
  (1.5% finer than slowrx) or a fractional stride. Currently the comment
  says "we round to 3" but doesn't justify why not 4.

---

### Finding 9 — `pixel_freq` Gaussian interpolation always runs; slowrx falls back to clipped 1500/2300 Hz at boundary

**Severity:** Minor
**Likely impact on real-audio decoder failure:** No (edge case)
**Files:** `video.c:389-398` ↔ `src/mode_pd.rs:166-186`

#### Context

slowrx (`video.c:389-398`):
```c
if (MaxBin > GetBin(1500 + CurrentPic.HedrShift, FFTLen) - 1 && MaxBin < GetBin(2300 + CurrentPic.HedrShift, FFTLen) + 1) {
  Freq = MaxBin + (log( Power[MaxBin + 1] / Power[MaxBin - 1] )) /
                 (2 * log( pow(Power[MaxBin], 2) / (Power[MaxBin + 1] * Power[MaxBin - 1])));
  Freq = Freq / FFTLen * 44100;
} else {
  // Clip if out of bounds
  Freq = ( (MaxBin > GetBin(1900 + CurrentPic.HedrShift, FFTLen)) ? 2300 : 1500 ) + CurrentPic.HedrShift;
}
```

If the FFT peak lands on either of the boundary bins (`lo - 1` or `hi + 1`),
slowrx skips the interpolation entirely and returns a hard-clipped 1500 Hz or
2300 Hz value (whichever side of 1900 Hz the peak was on).

Rust (`mode_pd.rs:166-186`):
```rust
let p_prev = power(self.fft_buf[max_bin - 1]);
let p_curr = max_p;
let p_next = power(self.fft_buf[max_bin + 1]);

let interp_ok = p_prev > 0.0 && p_curr > 0.0 && p_next > 0.0;
let freq_bin = if interp_ok {
    let num = (p_next / p_prev).ln();
    let denom = 2.0 * (p_curr * p_curr / (p_next * p_prev)).ln();
    if denom.abs() > 1e-12 {
        (max_bin as f64) + num / denom
    } else {
        max_bin as f64
    }
} else {
    max_bin as f64
};
```

Rust **always interpolates** if all three neighbor powers are positive,
regardless of where `max_bin` is in the search range.

#### Why this matters

For a peak at the boundary bin (= the search range's edge), slowrx returns a
clean clipped value. Rust returns whatever the interpolation formula
produces, which could pull the estimate toward the neighboring noise bin.

In typical operation the search range covers the entire video band plus a
1-bin margin, so peak-at-boundary is rare. But it does happen on heavily
mistuned signals (extreme HedrShift) or low-SNR audio where the FFT peak
genuinely lands at the boundary.

Worse, **for extremely negative HedrShift** the lower bound `lo` can land near
DC (bin 1 or 2), and the FFT's DC bin can have huge power from sample bias.
Rust's interpolation would pull the freq estimate toward DC. slowrx would
clip to 1500 Hz instead.

#### Acceptance criteria

- Add a boundary check matching slowrx: if `max_bin` is at the lo-1 or hi+1
  edge, return a clipped value `(if max_bin > bin_for(1900) { 2300.0 } else { 1500.0 }) + hedr_shift_hz`.
- Or document the divergence and the practical reasons it doesn't matter for
  V1.

#### Related

Independent of round-1 findings.

---

### Finding 10 — `slant_deg_last` not updated when `max_count == 0`

**Severity:** Cosmetic
**Likely impact on real-audio decoder failure:** No
**Files:** `src/sync.rs:208, 267-269, 332`

#### Context

```rust
let mut slant_deg_last = 90.0;
// ...
if max_count == 0 {
    break;  // ← break BEFORE updating slant_deg_last
}
let slant_angle = MIN_SLANT_DEG + (q_most as f64) * SLANT_STEP_DEG;
slant_deg_last = slant_angle;
// ...
SyncResult {
    ...
    slant_deg: slant_deg_last,
}
```

If the first iteration's Hough found no cells (all `has_sync` was false), the
loop breaks without updating `slant_deg_last`. So the returned `slant_deg`
field is the default `90.0`, not a meaningful "no sync detected" signal.

#### Why this matters

`slant_deg` is currently dead code (`#[allow(dead_code)]` at line 187). Only
tests read it. The default of 90.0 is misleading — the decoder thinks the
slant was detected at 90° when actually it was never detected.

If a future caller decides to use `slant_deg` for diagnostics ("was sync
even detected?"), they'd see 90.0 (the locked-on value!) for both
"perfectly aligned input" and "nothing-detected-at-all input" — not
distinguishable.

#### Acceptance criteria

- Either drop `slant_deg` from the public-ish `SyncResult` struct (it's
  `#[allow(dead_code)]`) or use `Option<f64>` and `None` for the
  no-detection path.

---

### Finding 11 — Stop-bit-end calculation: Rust gives uniform ~5 ms past true end across `i`; slowrx varies 5–25 ms

**Severity:** Minor
**Likely impact on real-audio decoder failure:** No (find_sync absorbs the offset)
**Files:** `vis.c:165-170` ↔ `src/vis.rs:130-148`

#### Context

slowrx's "skip 20 ms after VIS detection" (`vis.c:169`) lands the
`pcm.WindowPtr` at:
- i=0: stop bit end + 25 ms
- i=1: stop bit end + 15 ms
- i=2: stop bit end + 5 ms

(Because slowrx's `WindowPtr` after detection is at the CENTER of the most-
recent FFT window, which is `(2-i) × 10 ms` past the latest stop-bit-anchored
hop. Then `+20 ms` is added uniformly.)

Rust's formula gives a uniform `5 ms` past stop-bit end across all `i`:

```rust
// vis.rs:130-145
let stop_end_abs =
    (self.hops_completed.saturating_add(i_match as u64)) * HOP_SAMPLES as u64;
let drain_to_buf =
    usize::try_from(stop_end_abs.saturating_sub(self.audio_origin_sample))
        .unwrap_or(usize::MAX)
        .min(self.audio_buffer.len());
self.detected = Some(DetectedVis {
    code,
    hedr_shift_hz,
    end_sample: stop_end_abs,
});
self.audio_buffer.drain(..drain_to_buf);
```

Tracing through (using `K = hops_completed - 1`, the index of the most recent
processed hop):

- i=0 case (stop bit at hop K-2, true end at `(K + 0.5) × HOP`):
  `stop_end_abs = (K + 1) × HOP` → 5 ms past true end.
- i=1 case (stop bit at hop K-1, true end at `(K + 1.5) × HOP`):
  `stop_end_abs = (K + 2) × HOP` → 5 ms past true end.
- i=2 case (stop bit at hop K, true end at `(K + 2.5) × HOP`):
  `stop_end_abs = (K + 3) × HOP` → 5 ms past true end.

So Rust gives a uniform 5 ms post-stop-bit offset; slowrx varies 5–25 ms.

The Rust comment claims:
> slowrx vis.c lines 168-170 instead skips a fixed
> 20 ms regardless of `i`; we use the precise i-aware
> boundary so per-pixel image alignment stays tight

This is partly misleading. slowrx's behavior IS i-dependent (because the
`WindowPtr` at detection is i-dependent), just not in the way the Rust
comment implies. Slowrx ends up with a 5-25 ms range; Rust uniformly 5 ms.

#### Why this matters

For i=0 (slowrx's "skip 25 ms past stop"), the Rust-vs-slowrx audio offset
to line 0's expected sync pulse differs by 20 ms. Find_sync is supposed to
absorb this offset — and it does, for typical audio. But:

1. The 20 ms divergence shifts the `xAcc` pattern by 20 ms in absolute time,
   which is ~700/700 × 20 ms / line_time (PD120: 0.50848) = 0.039 = 4 % of
   one line worth. The 700-bin xAcc resolution is 0.7 ms/bin, so 20 ms is
   ~28 bins. The Hough finds the right line angle regardless.

2. After find_sync produces a Skip, the per-pair decode reads at `skip + ...`
   absolute audio indices. So the 20 ms divergence is absorbed by the skip.

3. The only path that bypasses find_sync is the `find_sync` "no sync detected"
   degenerate case, which produces Skip ≈ -sync_time (Finding 6). In that
   pathological case the decoded image is garbled regardless.

So the divergence is real but find_sync is a quasi-perfect absorber. Worth
noting because it underlines that the Rust port is NOT a 1:1 translation
even where it claims to be.

#### Acceptance criteria

- Update the misleading comment in `vis.rs:130-145` to reflect the actual
  divergence pattern (not "we use the precise i-aware boundary" — slowrx is
  also i-aware, just differently).

---

## Section 4 — Severity-ranked summary

1. **[Important]** Finding 1 — YCbCr→RGB integer division (round-1 #4
   half-fixed). Every YCbCr-decoded pixel is 0–1 LSB darker than slowrx.
2. **[Important]** Finding 2 — `bin_for` rounds while slowrx truncates;
   systematic 1-bin shift at 5 of 8 production frequencies.
3. **[Minor]** Finding 3 — SNR bandwidth correction divisors 2-9 % off due to #2.
4. **[Minor]** Finding 4 — `SyncTracker.Praw` integrates one fewer bin due to #2.
5. **[Minor]** Finding 5 — VIS retries all (i,j) on parity fail; slowrx breaks early.
6. **[Cosmetic]** Finding 6 — `find_sync` xmax init `i32::MIN` vs slowrx's 0.
7. **[Cosmetic]** Finding 7 — slant-lock interval `[89,91)` vs slowrx's `(89,91)`.
8. **[Cosmetic]** Finding 8 — sync probe stride 7.7% finer than slowrx.
9. **[Minor]** Finding 9 — `pixel_freq` always interpolates; slowrx clips at
   boundary.
10. **[Cosmetic]** Finding 10 — `slant_deg_last` stays 90° on no-detect.
11. **[Minor]** Finding 11 — stop-bit-end uniform 5 ms vs slowrx's 5–25 ms.

## Section 5 — Re-verification of round-1 deferred findings

- **Round-1 #39 (VIS stop-bit boundary):** still intentional. Rust's window-
  end semantic differs from slowrx's window-center semantic by design; the
  `take_residual_buffer` doc comment correctly explains why no extra skip is
  needed in Rust.

- **Round-1 #42 (FindSync 90° deadband):** still correct. Without the
  deadband, half-degree Hough quantization noise creates a 0.0085 % rate
  perturbation per call to `find_sync`, which compounds across PD180's 248
  line pairs to cause measurable per-pair drift.

- **Round-1 #44 (adaptive Hann deferred):** still correct. The synthetic
  encoder (`pd_test_encoder.rs`) produces hard tone-step transitions that
  break SNR estimates. `win_idx = 6` is hard-coded with the correct
  rationale documented. Once a realistic FM-slewing synthetic encoder lands
  the adaptive selector should engage.

  **However:** Finding 3 (this audit) shows the SNR estimator's bandwidth
  correction is off by 2-9 % from slowrx, so if/when the adaptive selector
  is engaged, the threshold decisions may shift slightly. Note this for
  future tuning.

- **Round-1 #45 (channel-mask deferred):** still correct. The Rust
  channel-isolation strategy is a deliberate deviation from slowrx's
  cross-channel-leak FFT. The doc comment in `decode_pd_line_pair` correctly
  explains the synthetic-encoder rationale.

## Section 6 — What I checked and saw NO divergence

- Hann window construction (`vis.rs::build_hann_window`, `snr.rs::build_hann`).
  Match `vis.c:30` and `video.c:54`.
- VIS bit ordering (LSB-first), parity convention (even). Match.
- Channel time formulas after PR #47's `septr_seconds`. Match.
- ModeSpec PD120 / PD180 numeric fields. Match `modespec.c` row-by-row.
- YCbCr→RGB matrix coefficients (100/140/-17850, 100/-71/-33/13260,
  100/178/-22695). Match — but the DIVISION is wrong (Finding 1).
- Goertzel implementation `vis.rs::goertzel_power`. Match the standard form;
  not used by VIS detection (which now uses real FFT) but used by tests and
  `decoder::estimate_freq` (the latter is dead-coded production-wise).
- Resampler quality (covered by tests).
- `SstvDecoder` state-machine transitions vs `Listen()`. Match in spirit.
- `take_residual_buffer` re-anchor contract. Honored in `decoder.rs::process`.
- Linear Hough transform's `(d, q)` shape filter (`d > 0 && d < LineWidth`).
  Match.
- Slant rate adjustment formula `Rate += tan(deg2rad(90 - slantAngle)) /
  LineWidth * Rate`. Match.
- `xAcc` column accumulator dimensions (700 × NumLines). Match.
- 8-tap convolution kernel `[1,1,1,1,-1,-1,-1,-1]`. Match.
- Skip computation for PD modes (Scottie branch unreachable). Match.

## Section 7 — Things I expected to find but did not

- I expected to find a sign error in HedrShift application somewhere.
  Couldn't find one — every `hedr_shift_hz` site (mode_pd, snr, sync) applies
  it as `+hedr` to the band edges, matching slowrx. Match.

- I expected to find a sample-counter off-by-one or rounding-direction issue
  in the per-pair decode. Phase 3's rewrite (PR #46) eliminated these via
  the single-`round()` formula `skip + ((pair_seconds + chan_start_sec +
  pixel_secs * (x + 0.5)) * rate).round()`. Confirmed sound.

- I expected the PD test encoder's chroma-averaging convention (`midpoint`)
  to differ from what the decoder reads. It uses `u8::midpoint` which
  rounds-up on tie (e.g., midpoint(0, 1) = 1) — but the decoder doesn't read
  back averaged values, it reads the encoded freq directly, so the
  mismatch is masked by the encoder/decoder chroma path being symmetric on
  the synthetic test. Not a parity issue.

## Section 8 — Final notes

The user said "we are sure we are missing more things." Round 2 found 11
more divergences. The most consequential — by a wide margin — is **Finding 1
(YCbCr→RGB integer division)**, which round-1 partially flagged but PR #47
left unfixed. Every YCbCr-decoded pixel is biased darker by 0-1 LSB
relative to slowrx. The synthetic round-trip test's encoder/decoder
symmetry has been masking this.

Findings 2-4 are a coherent cluster around the `bin_for` `.round()` policy.
They aren't material to V1 decode quality on their own (the SNR estimator's
output is currently unused thanks to round-1 #44, and the Praw/Psync logic
absorbs sub-bin shifts), but they will shift behavior measurably when round-1
#44 is engaged.

Findings 5-11 are minor or cosmetic — worth fixing for parity hygiene and
future-maintainer accuracy, but unlikely to affect real-audio decode outcomes.

The user's hypothesis that "we are sure we are missing more things" is
confirmed: round 1 found 23, round 2 finds 11 more. Total: 34 across both
audits. Future rounds may find a few more around the resampler quality vs
real-audio (round-1 #18 deferred) and around mid-image VIS detection
(deferred to PR-3). For now, **Finding 1 is the highest-priority fix**.
