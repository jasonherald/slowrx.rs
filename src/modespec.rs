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
    /// Channel layout used by per-mode decoders.
    pub channel_layout: ChannelLayout,
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

/// Look up the [`ModeSpec`] for a given 7-bit VIS code. Returns `None`
/// if the code is reserved, undefined, or maps to a mode not yet
/// implemented in this V1 release.
///
/// VIS codes are taken from Dave Jones (KB4YZ), 1998: "List of SSTV
/// Modes with VIS Codes".
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
    channel_layout: ChannelLayout::PdYcbcr,
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
    channel_layout: ChannelLayout::PdYcbcr,
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
}
