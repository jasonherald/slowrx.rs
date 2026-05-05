//! SSTV mode specifications.
//!
//! Translated from slowrx's `modespec.c` (Oona Räisänen, ISC License).
//! See `NOTICE.md` for full attribution.
//!
//! Implemented as of V2.3 (0.4.0): PD120, PD180, PD240, Robot 24, Robot 36,
//! Robot 72, Scottie 1, Scottie 2, Scottie DX. Scottie 1/2/DX are present
//! in the [`SstvMode`] enum and the lookup tables as of 0.4.0 Phase 1
//! scaffolding; the per-line demodulation lands in Phase 3 (see
//! `mode_scottie::decode_line`). Remaining V2 modes (Martin 1/2) are
//! not yet present and will land with their own PR.

/// SSTV operating mode. Implemented: [`SstvMode::Pd120`], [`SstvMode::Pd180`],
/// [`SstvMode::Pd240`], [`SstvMode::Robot24`], [`SstvMode::Robot36`],
/// [`SstvMode::Robot72`], [`SstvMode::Scottie1`], [`SstvMode::Scottie2`],
/// [`SstvMode::ScottieDx`]. Additional V2 modes (Martin) planned.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SstvMode {
    /// PD-120 (640×496, ~120s per image)
    Pd120,
    /// PD-180 (640×496, ~180s per image)
    Pd180,
    /// PD-240 (640×496, ~240s per image)
    Pd240,
    /// Robot 24 (320×240, ~24s per image)
    Robot24,
    /// Robot 36 (320×240, ~36s per image)
    Robot36,
    /// Robot 72 (320×240, ~72s per image)
    Robot72,
    /// Scottie 1 — VIS `0x3C`, 320×256 GBR, 0.4320 ms/pixel.
    Scottie1,
    /// Scottie 2 — VIS `0x38`, 320×256 GBR, 0.2752 ms/pixel.
    Scottie2,
    /// Scottie DX — VIS `0x4C`, 320×256 GBR, 1.08053 ms/pixel.
    /// slowrx applies a +1 Hann-window-index bump in the per-pixel
    /// demod when this mode is active (see `mode_scottie::decode_line`).
    ScottieDx,
}

/// Mode timing + layout table entry.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ModeSpec {
    /// The mode this entry describes.
    pub mode: SstvMode,
    /// 7-bit VIS code identifying this mode on the wire.
    pub vis_code: u8,
    /// Visible image width in pixels.
    pub line_pixels: u32,
    /// Total visible scan lines per image.
    pub image_lines: u32,
    /// Total per-line duration including sync + porches, seconds.
    pub line_seconds: f64,
    /// Sync pulse duration, seconds.
    pub sync_seconds: f64,
    /// Porch (post-sync settling) duration, seconds.
    pub porch_seconds: f64,
    /// Per-pixel duration within a colour channel, seconds.
    pub pixel_seconds: f64,
    /// Channel separator pulse duration, seconds. Translated from slowrx's
    /// `SeptrTime` field (`modespec.c`). Zero for all PD-family modes; non-zero
    /// for Robot, Martin, and Scottie modes (V2). Stored here so the
    /// `chan_starts_sec` formula in `mode_pd::decode_pd_line_pair` matches
    /// slowrx's `video.c:88-92` term-for-term and won't silently break when
    /// non-PD modes are added.
    pub septr_seconds: f64,
    /// Channel layout used by per-mode decoders.
    pub channel_layout: ChannelLayout,
    /// Where the sync pulse sits within a radio line. See [`SyncPosition`]
    /// for the rationale (V2 carve-out forcing mid-line sync to be
    /// explicit when V2.3 Scottie lands).
    pub sync_position: SyncPosition,
}

/// Per-mode channel arrangement. PD-family modes use
/// [`ChannelLayout::PdYcbcr`]; Robot family uses [`ChannelLayout::RobotYuv`].
/// Future V2 mode families (Scottie, Martin) add their own values.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChannelLayout {
    /// PD-family: Y(odd) → Cr → Cb → Y(even). One radio line carries
    /// two image rows; chroma is shared between paired rows.
    PdYcbcr,
    /// Robot family: Y (single luma channel) plus chroma. R36/R24 carry
    /// alternating Cr/Cb per radio line with each chroma sample
    /// duplicated to the next image row; R72 carries Y/U/V sequentially
    /// per line. The shape difference is mode-internal — see
    /// `mode_robot::decode_line` for the per-mode dispatch.
    RobotYuv,
    /// Sequential single-line RGB layout — three channels per radio
    /// line. Used by Scottie (G→B→R, sync mid-line) and Martin (G→B→R,
    /// sync at line start).
    RgbSequential,
}

/// Where the sync pulse sits within a radio line.
///
/// PD/Robot/Martin all place sync at line start (the standard SSTV
/// convention). Scottie modes are the exception — sync sits between G
/// and B channels, not at line start. Stored here so future mode
/// decoders are forced to make their sync placement explicit at dispatch
/// time, surfacing the V1 line-clock-advance assumption that sync ==
/// line start.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyncPosition {
    /// Sync pulse at the start of each radio line. PD, Robot, Martin.
    /// Scottie family uses [`SyncPosition::Scottie`] instead.
    LineStart,
    /// Sync pulse between B and R within each radio line. Scottie family.
    Scottie,
}

/// Look up the [`ModeSpec`] for a given 7-bit VIS code. Returns `None`
/// if the code is reserved, undefined, or maps to a mode not yet
/// implemented in this release.
///
/// VIS codes are taken from Dave Jones (KB4YZ), 1998: "List of SSTV
/// Modes with VIS Codes".
///
/// **Parity-audit note (#27):** `0x00` is intentionally unmapped and
/// returns `None`. In slowrx (`vis.c:172-174`), an unknown VIS code causes
/// `GetVIS()` to return 0 and `Listen()` loops back to re-detect
/// (`do { ... } while (Mode == 0)`). Rust's equivalent is `None` from
/// this function: the caller in `SstvDecoder::process` drains the VIS
/// detector's buffer and stays in `AwaitingVis`, which has the same
/// effect as slowrx's re-detect loop. Both treat an unknown code as a
/// silent "try again" rather than an error.
#[must_use]
pub fn lookup(vis_code: u8) -> Option<ModeSpec> {
    match vis_code {
        0x04 => Some(ROBOT24),
        0x08 => Some(ROBOT36),
        0x0C => Some(ROBOT72),
        0x38 => Some(SCOTTIE2),
        0x3C => Some(SCOTTIE1),
        0x4C => Some(SCOTTIE_DX),
        0x5F => Some(PD120),
        0x60 => Some(PD180),
        0x61 => Some(PD240),
        _ => None,
    }
}

/// Look up the [`ModeSpec`] for a known [`SstvMode`].
///
/// Always returns `Some` for V1 modes. Reserved for symmetry with
/// [`lookup`] when V2 modes whose decoders are not yet implemented
/// land in the enum.
#[must_use]
pub fn for_mode(mode: SstvMode) -> ModeSpec {
    match mode {
        SstvMode::Pd120 => PD120,
        SstvMode::Pd180 => PD180,
        SstvMode::Pd240 => PD240,
        SstvMode::Robot24 => ROBOT24,
        SstvMode::Robot36 => ROBOT36,
        SstvMode::Robot72 => ROBOT72,
        SstvMode::Scottie1 => SCOTTIE1,
        SstvMode::Scottie2 => SCOTTIE2,
        SstvMode::ScottieDx => SCOTTIE_DX,
    }
}

// Mode timing constants — translated row-for-row from slowrx's
// modespec.c (PD120 lines 260-271, PD180 lines 286-297, PD240 lines 299-310,
// R72 lines 130-141, R36 lines 143-154, R24 lines 156-167).

const PD120: ModeSpec = ModeSpec {
    mode: SstvMode::Pd120,
    vis_code: 0x5F,
    line_pixels: 640,
    image_lines: 496,
    line_seconds: 0.508_48,
    sync_seconds: 0.020,
    porch_seconds: 0.002_08,
    pixel_seconds: 0.000_19,
    septr_seconds: 0.0, // modespec.c: SeptrTime = 0e-3 for PD-family
    channel_layout: ChannelLayout::PdYcbcr,
    sync_position: SyncPosition::LineStart,
};

const PD180: ModeSpec = ModeSpec {
    mode: SstvMode::Pd180,
    vis_code: 0x60,
    line_pixels: 640,
    image_lines: 496,
    line_seconds: 0.754_24,
    sync_seconds: 0.020,
    porch_seconds: 0.002_08,
    pixel_seconds: 0.000_286,
    septr_seconds: 0.0, // modespec.c: SeptrTime = 0e-3 for PD-family
    channel_layout: ChannelLayout::PdYcbcr,
    sync_position: SyncPosition::LineStart,
};

const PD240: ModeSpec = ModeSpec {
    mode: SstvMode::Pd240,
    vis_code: 0x61,
    line_pixels: 640,
    image_lines: 496,
    // slowrx modespec.c:299-310 — PD240 LineTime = 1000e-3,
    // PixelTime = 0.382e-3, SyncTime = 20e-3, PorchTime = 2.08e-3.
    line_seconds: 1.000,
    sync_seconds: 0.020,
    porch_seconds: 0.002_08,
    pixel_seconds: 0.000_382,
    septr_seconds: 0.0, // modespec.c: SeptrTime = 0e-3 for PD-family
    channel_layout: ChannelLayout::PdYcbcr,
    sync_position: SyncPosition::LineStart,
};

const ROBOT24: ModeSpec = ModeSpec {
    mode: SstvMode::Robot24,
    vis_code: 0x04,
    line_pixels: 320,
    image_lines: 240,
    // slowrx modespec.c:156-167 — R24 LineTime = 150e-3,
    // PixelTime = 0.1375e-3, SyncTime = 9e-3, PorchTime = 3e-3,
    // SeptrTime = 6e-3.
    line_seconds: 0.150,
    sync_seconds: 0.009,
    porch_seconds: 0.003,
    pixel_seconds: 0.000_137_5,
    septr_seconds: 0.006,
    channel_layout: ChannelLayout::RobotYuv,
    sync_position: SyncPosition::LineStart,
};

const ROBOT36: ModeSpec = ModeSpec {
    mode: SstvMode::Robot36,
    vis_code: 0x08,
    line_pixels: 320,
    image_lines: 240,
    // slowrx modespec.c:143-154 — R36 LineTime = 150e-3,
    // PixelTime = 0.1375e-3, SyncTime = 9e-3, PorchTime = 3e-3,
    // SeptrTime = 6e-3.  Identical timing to R24.
    line_seconds: 0.150,
    sync_seconds: 0.009,
    porch_seconds: 0.003,
    pixel_seconds: 0.000_137_5,
    septr_seconds: 0.006,
    channel_layout: ChannelLayout::RobotYuv,
    sync_position: SyncPosition::LineStart,
};

const ROBOT72: ModeSpec = ModeSpec {
    mode: SstvMode::Robot72,
    vis_code: 0x0C,
    line_pixels: 320,
    image_lines: 240,
    // slowrx modespec.c:130-141 — R72 LineTime = 300e-3,
    // PixelTime = 0.2875e-3, SyncTime = 9e-3, PorchTime = 3e-3,
    // SeptrTime = 4.7e-3.
    line_seconds: 0.300,
    sync_seconds: 0.009,
    porch_seconds: 0.003,
    pixel_seconds: 0.000_287_5,
    septr_seconds: 0.0047,
    channel_layout: ChannelLayout::RobotYuv,
    sync_position: SyncPosition::LineStart,
};

const SCOTTIE1: ModeSpec = ModeSpec {
    mode: SstvMode::Scottie1,
    vis_code: 0x3C,
    line_pixels: 320,
    image_lines: 256,
    // slowrx modespec.c:91-104 — S1 LineTime = 428.38e-3,
    // PixelTime = 0.4320e-3, SyncTime = 9e-3, PorchTime = 1.5e-3,
    // SeptrTime = 1.5e-3.
    line_seconds: 0.428_38,
    sync_seconds: 0.009,
    porch_seconds: 0.001_5,
    pixel_seconds: 0.000_432_0,
    septr_seconds: 0.001_5,
    channel_layout: ChannelLayout::RgbSequential,
    sync_position: SyncPosition::Scottie,
};

const SCOTTIE2: ModeSpec = ModeSpec {
    mode: SstvMode::Scottie2,
    vis_code: 0x38,
    line_pixels: 320,
    image_lines: 256,
    // slowrx modespec.c:105-117 — S2 LineTime = 277.692e-3,
    // PixelTime = 0.2752e-3, SyncTime = 9e-3, PorchTime = 1.5e-3,
    // SeptrTime = 1.5e-3.
    line_seconds: 0.277_692,
    sync_seconds: 0.009,
    porch_seconds: 0.001_5,
    pixel_seconds: 0.000_275_2,
    septr_seconds: 0.001_5,
    channel_layout: ChannelLayout::RgbSequential,
    sync_position: SyncPosition::Scottie,
};

const SCOTTIE_DX: ModeSpec = ModeSpec {
    mode: SstvMode::ScottieDx,
    vis_code: 0x4C,
    line_pixels: 320,
    image_lines: 256,
    // slowrx modespec.c:118-128 — SDX LineTime = 1050.3e-3,
    // PixelTime = 1.08053e-3, SyncTime = 9e-3, PorchTime = 1.5e-3,
    // SeptrTime = 1.5e-3.
    line_seconds: 1.050_3,
    sync_seconds: 0.009,
    porch_seconds: 0.001_5,
    pixel_seconds: 0.001_080_53,
    septr_seconds: 0.001_5,
    channel_layout: ChannelLayout::RgbSequential,
    sync_position: SyncPosition::Scottie,
};

#[cfg(test)]
#[allow(clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn pd120_vis_code_resolves() {
        let spec = lookup(0x5F).expect("PD120 VIS resolves");
        assert_eq!(spec.mode, SstvMode::Pd120);
        assert_eq!(spec.vis_code, 0x5F);
        assert_eq!(spec.line_pixels, 640);
        assert_eq!(spec.image_lines, 496);
        assert_eq!(spec.channel_layout, ChannelLayout::PdYcbcr);
        assert_eq!(spec.line_seconds, 0.508_48);
        assert_eq!(spec.sync_seconds, 0.020);
        assert_eq!(spec.porch_seconds, 0.002_08);
        assert_eq!(spec.pixel_seconds, 0.000_19);
    }

    #[test]
    fn pd180_vis_code_resolves() {
        let spec = lookup(0x60).expect("PD180 VIS resolves");
        assert_eq!(spec.mode, SstvMode::Pd180);
        assert_eq!(spec.pixel_seconds, 0.000_286);
    }

    #[test]
    fn unknown_vis_codes_return_none() {
        assert!(lookup(0x00).is_none());
        assert!(lookup(0x42).is_none()); // reserved
        assert!(lookup(0xFF).is_none());
    }

    #[test]
    fn for_mode_returns_matching_spec() {
        assert_eq!(for_mode(SstvMode::Pd120).vis_code, 0x5F);
        assert_eq!(for_mode(SstvMode::Pd180).vis_code, 0x60);
    }

    #[test]
    fn pd_modes_have_zero_septr_seconds() {
        // PD-family: SeptrTime = 0e-3 (modespec.c). The field exists for
        // V2 parity (Robot/Scottie/Martin have non-zero SeptrTime); for PD
        // modes it must be zero so chan_starts_sec is numerically unchanged.
        let pd120 = lookup(0x5F).expect("PD120");
        let pd180 = lookup(0x60).expect("PD180");
        let pd240 = lookup(0x61).expect("PD240");
        assert_eq!(pd120.septr_seconds, 0.0);
        assert_eq!(pd180.septr_seconds, 0.0);
        assert_eq!(pd240.septr_seconds, 0.0);
    }

    #[test]
    fn all_v2_modes_have_line_start_sync_position() {
        // V2 carve-out: ModeSpec.sync_position lets V2.3 Scottie declare
        // mid-line sync without retrofitting V1. PD/Robot/Martin all use
        // line-start sync; Scottie is the V2.3 exception.
        for mode in [
            SstvMode::Pd120,
            SstvMode::Pd180,
            SstvMode::Pd240,
            SstvMode::Robot24,
            SstvMode::Robot36,
            SstvMode::Robot72,
        ] {
            let spec = for_mode(mode);
            assert_eq!(spec.sync_position, SyncPosition::LineStart);
        }
    }

    #[test]
    fn pd240_vis_code_resolves() {
        let spec = lookup(0x61).expect("PD240 VIS resolves");
        assert_eq!(spec.mode, SstvMode::Pd240);
        assert_eq!(spec.vis_code, 0x61);
        assert_eq!(spec.line_pixels, 640);
        assert_eq!(spec.image_lines, 496);
        assert_eq!(spec.channel_layout, ChannelLayout::PdYcbcr);
        assert_eq!(spec.sync_position, SyncPosition::LineStart);
        assert_eq!(spec.line_seconds, 1.000);
        assert_eq!(spec.sync_seconds, 0.020);
        assert_eq!(spec.porch_seconds, 0.002_08);
        assert_eq!(spec.pixel_seconds, 0.000_382);
        assert_eq!(spec.septr_seconds, 0.0);
    }

    #[test]
    fn for_mode_returns_pd240_spec() {
        assert_eq!(for_mode(SstvMode::Pd240).vis_code, 0x61);
    }

    #[test]
    fn robot24_vis_code_resolves() {
        let spec = lookup(0x04).expect("R24 VIS resolves");
        assert_eq!(spec.mode, SstvMode::Robot24);
        assert_eq!(spec.vis_code, 0x04);
        assert_eq!(spec.line_pixels, 320);
        assert_eq!(spec.image_lines, 240);
        assert_eq!(spec.channel_layout, ChannelLayout::RobotYuv);
        assert_eq!(spec.sync_position, SyncPosition::LineStart);
        assert_eq!(spec.line_seconds, 0.150);
        assert_eq!(spec.sync_seconds, 0.009);
        assert_eq!(spec.porch_seconds, 0.003);
        assert_eq!(spec.septr_seconds, 0.006);
        assert_eq!(spec.pixel_seconds, 0.000_137_5);
    }

    #[test]
    fn robot36_vis_code_resolves() {
        let spec = lookup(0x08).expect("R36 VIS resolves");
        assert_eq!(spec.mode, SstvMode::Robot36);
        assert_eq!(spec.vis_code, 0x08);
        assert_eq!(spec.line_pixels, 320);
        assert_eq!(spec.image_lines, 240);
        assert_eq!(spec.channel_layout, ChannelLayout::RobotYuv);
        assert_eq!(spec.sync_position, SyncPosition::LineStart);
        assert_eq!(spec.line_seconds, 0.150);
        assert_eq!(spec.sync_seconds, 0.009);
        assert_eq!(spec.porch_seconds, 0.003);
        assert_eq!(spec.septr_seconds, 0.006);
        assert_eq!(spec.pixel_seconds, 0.000_137_5);
    }

    #[test]
    fn robot72_vis_code_resolves() {
        let spec = lookup(0x0C).expect("R72 VIS resolves");
        assert_eq!(spec.mode, SstvMode::Robot72);
        assert_eq!(spec.vis_code, 0x0C);
        assert_eq!(spec.line_pixels, 320);
        assert_eq!(spec.image_lines, 240);
        assert_eq!(spec.channel_layout, ChannelLayout::RobotYuv);
        assert_eq!(spec.sync_position, SyncPosition::LineStart);
        assert_eq!(spec.line_seconds, 0.300);
        assert_eq!(spec.sync_seconds, 0.009);
        assert_eq!(spec.porch_seconds, 0.003);
        assert_eq!(spec.septr_seconds, 0.0047);
        assert_eq!(spec.pixel_seconds, 0.000_287_5);
    }

    #[test]
    fn for_mode_returns_robot_specs() {
        assert_eq!(for_mode(SstvMode::Robot24).vis_code, 0x04);
        assert_eq!(for_mode(SstvMode::Robot36).vis_code, 0x08);
        assert_eq!(for_mode(SstvMode::Robot72).vis_code, 0x0C);
    }

    #[test]
    fn scottie1_modespec() {
        let spec = for_mode(SstvMode::Scottie1);
        assert_eq!(spec.mode, SstvMode::Scottie1);
        assert_eq!(spec.vis_code, 0x3C);
        assert_eq!(spec.line_pixels, 320);
        assert_eq!(spec.image_lines, 256);
        assert_eq!(spec.channel_layout, ChannelLayout::RgbSequential);
        assert_eq!(spec.sync_position, SyncPosition::Scottie);
        assert!((spec.pixel_seconds - 0.4320e-3).abs() < 1e-9);
        assert!((spec.line_seconds - 428.38e-3).abs() < 1e-9);
    }

    #[test]
    fn scottie2_modespec() {
        let spec = for_mode(SstvMode::Scottie2);
        assert_eq!(spec.mode, SstvMode::Scottie2);
        assert_eq!(spec.vis_code, 0x38);
        assert!((spec.pixel_seconds - 0.2752e-3).abs() < 1e-9);
        assert!((spec.line_seconds - 277.692e-3).abs() < 1e-9);
        assert_eq!(spec.channel_layout, ChannelLayout::RgbSequential);
        assert_eq!(spec.sync_position, SyncPosition::Scottie);
    }

    #[test]
    fn scottie_dx_modespec() {
        let spec = for_mode(SstvMode::ScottieDx);
        assert_eq!(spec.mode, SstvMode::ScottieDx);
        assert_eq!(spec.vis_code, 0x4C);
        assert!((spec.pixel_seconds - 1.08053e-3).abs() < 1e-9);
        assert!((spec.line_seconds - 1050.3e-3).abs() < 1e-9);
        assert_eq!(spec.channel_layout, ChannelLayout::RgbSequential);
        assert_eq!(spec.sync_position, SyncPosition::Scottie);
    }

    #[test]
    fn scottie_vis_codes_resolve() {
        // Codebase uses `lookup` (returning `Option<ModeSpec>`) rather
        // than `for_vis_code`; mirrors the existing
        // `pd120_vis_code_resolves` style.
        assert_eq!(
            lookup(0x3C).expect("S1 VIS resolves").mode,
            SstvMode::Scottie1
        );
        assert_eq!(
            lookup(0x38).expect("S2 VIS resolves").mode,
            SstvMode::Scottie2
        );
        assert_eq!(
            lookup(0x4C).expect("SDX VIS resolves").mode,
            SstvMode::ScottieDx
        );
    }
}
