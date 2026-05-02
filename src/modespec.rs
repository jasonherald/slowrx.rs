//! SSTV mode specifications.
//!
//! Translated from slowrx's `modespec.c` (Oona Räisänen, ISC License).
//! See `NOTICE.md` for full attribution.
//!
//! V1 implements PD120 + PD180. V2 modes (PD240, Robot 36/72, Scottie 1/2/DX,
//! Martin 1/2) are planned but not yet present in the [`SstvMode`] enum or
//! the lookup tables; each will land with its own PR.

/// SSTV operating mode. V1 implements [`SstvMode::Pd120`] and
/// [`SstvMode::Pd180`]; additional modes are planned for V2.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SstvMode {
    /// PD-120 (640×496, ~120s per image)
    Pd120,
    /// PD-180 (640×496, ~180s per image)
    Pd180,
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

/// Per-mode channel arrangement. PD-family modes use [`ChannelLayout::PdYcbcr`].
/// Future V2 variants add their own values.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChannelLayout {
    /// PD-family: Y(odd) → Cr → Cb → Y(even). One radio line carries
    /// two image rows; chroma is shared between paired rows.
    PdYcbcr,
}

/// Where the sync pulse sits within a radio line.
///
/// PD/Robot/Martin all place sync at line start (the standard SSTV
/// convention). Scottie modes are the exception — sync sits between G
/// and B channels, not at line start. Stored here so future mode
/// decoders are forced to make their sync placement explicit at dispatch
/// time, surfacing the V1 line-clock-advance assumption that sync ==
/// line start.
///
/// V1 + V2.1 only emit [`SyncPosition::LineStart`]; [`SyncPosition::Scottie`]
/// lands with the V2.3 Scottie family epic.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyncPosition {
    /// Sync pulse at the start of each radio line. PD, Robot, Martin.
    LineStart,
}

/// Look up the [`ModeSpec`] for a given 7-bit VIS code. Returns `None`
/// if the code is reserved, undefined, or maps to a mode not yet
/// implemented in this V1 release.
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
        0x5F => Some(PD120),
        0x60 => Some(PD180),
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
    }
}

// Mode timing constants — translated row-for-row from slowrx's
// modespec.c (PD120 lines 260-271, PD180 lines 286-297).

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
        assert_eq!(pd120.septr_seconds, 0.0);
        assert_eq!(pd180.septr_seconds, 0.0);
    }

    #[test]
    fn pd_modes_have_line_start_sync_position() {
        // V2 carve-out: ModeSpec.sync_position lets V2.3 Scottie declare
        // mid-line sync without retrofitting V1. PD120/PD180/PD240 all use
        // line-start sync (slowrx video.c:88-92 places sync at the start of
        // each line for PD modes).
        for mode in [SstvMode::Pd120, SstvMode::Pd180] {
            let spec = for_mode(mode);
            assert_eq!(spec.sync_position, SyncPosition::LineStart);
        }
    }
}
