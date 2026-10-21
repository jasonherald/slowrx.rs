//! SSTV mode specifications.
//!
//! Translated from slowrx's `modespec.c` (Oona Räisänen, ISC License).
//! See `NOTICE.md` for full attribution.
//!
//! Implemented modes: PD120, PD180, PD240, Robot 24, Robot 36, Robot 72,
//! Robot 8 / 12 / 24 B/W, Scottie 1, Scottie 2, Scottie DX, Martin 1,
//! Martin 2, Wraase SC-2 120, Wraase SC-2 180. All RGB-sequential modes
//! (Scottie, Martin, Wraase SC-2) share a single decode path; the per-line
//! offsets branch on [`SyncPosition`] and the channel order on
//! [`ColorEncoding`].

/// SSTV operating mode. Implemented: [`SstvMode::Pd120`], [`SstvMode::Pd180`],
/// [`SstvMode::Pd240`], [`SstvMode::Robot24`], [`SstvMode::Robot36`],
/// [`SstvMode::Robot72`], [`SstvMode::Robot24Bw`], [`SstvMode::Robot12Bw`],
/// [`SstvMode::Robot8Bw`], [`SstvMode::Scottie1`], [`SstvMode::Scottie2`],
/// [`SstvMode::ScottieDx`], [`SstvMode::Martin1`], [`SstvMode::Martin2`],
/// [`SstvMode::Wraase120`], [`SstvMode::Wraase180`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SstvMode {
    /// PD-120. VIS `0x5F`. See [`for_mode`] for full timing.
    Pd120,
    /// PD-180. VIS `0x60`.
    Pd180,
    /// PD-240. VIS `0x61`.
    Pd240,
    /// Robot 24 (conventional name — decode buffer is ~36 s). VIS `0x04`.
    Robot24,
    /// Robot 36. VIS `0x08`.
    Robot36,
    /// Robot 72. VIS `0x0C`.
    Robot72,
    /// Robot 24 B/W (monochrome, 320 × 240). VIS `0x0A`.
    Robot24Bw,
    /// Robot 12 B/W (monochrome, 320 × 120, shown at
    /// [`ModeSpec::line_height`] 2). VIS `0x06`, sent with an inverted
    /// parity bit.
    Robot12Bw,
    /// Robot 8 B/W (monochrome, 320 × 120, shown at
    /// [`ModeSpec::line_height`] 2). VIS `0x02`.
    Robot8Bw,
    /// Scottie 1. VIS `0x3C`.
    Scottie1,
    /// Scottie 2. VIS `0x38`.
    Scottie2,
    /// Scottie DX. VIS `0x4C`.
    ScottieDx,
    /// Martin 1. VIS `0x2C`.
    Martin1,
    /// Martin 2. VIS `0x28`.
    Martin2,
    /// Wraase SC-2 120. VIS `0x3F`.
    Wraase120,
    /// Wraase SC-2 180. VIS `0x37`.
    Wraase180,
}

/// Mode timing + layout table entry.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ModeSpec {
    /// The mode this entry describes.
    pub mode: SstvMode,
    /// CLI/filename slug. Stable across releases (filenames like
    /// `img-NNN-{short_name}.png` depend on this). lowercase, no
    /// separators: "pd120", "robot24", "scottiedx", "scottie1",
    /// "martin1", etc. (audit #91 B13)
    pub short_name: &'static str,
    /// Human-readable mode name. For log lines and any future
    /// user-facing display. "PD-120", "Robot 24", "Scottie DX", etc.
    /// (audit #91 B13)
    pub name: &'static str,
    /// 7-bit VIS code identifying this mode on the wire.
    pub vis_code: u8,
    /// Visible image width in pixels.
    pub line_pixels: u32,
    /// Total visible scan lines per image — the decoded image's height.
    pub image_lines: u32,
    /// Display rows per decoded image row: slowrx's `LineHeight`
    /// (`modespec.c`). 2 for Robot 12 B/W and Robot 8 B/W, whose 120 lines
    /// are meant to be shown at 320 × 240; 1 for every other mode here.
    /// Display-only — slowrx decodes `NumLines` rows and applies it only
    /// when scaling the image for display and for its saved PNG
    /// (`video.c:243-244`, `common.c:86-87`). The decoded [`SstvImage`]
    /// stays `image_lines` tall; stretch each row `line_height` times for
    /// the intended aspect.
    ///
    /// [`SstvImage`]: crate::image::SstvImage
    pub line_height: u32,
    /// Total per-line duration including sync + porches, seconds.
    pub line_seconds: f64,
    /// Sync pulse duration, seconds.
    pub sync_seconds: f64,
    /// Porch (post-sync settling) duration, seconds.
    pub porch_seconds: f64,
    /// Per-pixel duration within a colour channel, seconds.
    pub pixel_seconds: f64,
    /// Channel separator pulse duration, seconds. Translated from slowrx's
    /// `SeptrTime` field (`modespec.c`). Zero for the PD-family, Robot B/W
    /// and Wraase SC-2 modes; non-zero for colour Robot, Martin, and Scottie
    /// modes (V2). Stored here so the
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
    /// How the line's channels make an RGB pixel: slowrx's `ColorEnc`
    /// column (`modespec.c`), applied by its pixel switch
    /// (`video.c:432-455`). For the RGB-sequential modes it is the channel
    /// order: Martin and Scottie send G, B, R ([`ColorEncoding::Gbr`]),
    /// Wraase SC-2 sends R, G, B ([`ColorEncoding::Rgb`]).
    pub color_encoding: ColorEncoding,
}

/// Per-mode channel arrangement. PD-family modes use
/// [`ChannelLayout::PdYcbcr`]; the colour Robot modes use
/// [`ChannelLayout::RobotYuv`] and the Robot B/W modes
/// [`ChannelLayout::Monochrome`]; Scottie, Martin and Wraase SC-2 use
/// [`ChannelLayout::RgbSequential`].
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
    /// `mode_robot::RobotYuvDecoder` for the per-mode dispatch.
    RobotYuv,
    /// Sequential single-line RGB layout — three channels per radio
    /// line. Used by Scottie (G→B→R, sync mid-line), Martin (G→B→R,
    /// sync at line start) and Wraase SC-2 (R→G→B, sync at line start, no
    /// separator). The channel order is the mode's
    /// [`ModeSpec::color_encoding`].
    RgbSequential,
    /// One luminance channel per radio line, one image row per radio line,
    /// shown as grey (`R = G = B = Y`). Robot 24 / 12 / 8 B/W — slowrx's
    /// `NumChans = 1` modes (`video.c:103-109`) with `ColorEnc = BW`
    /// (`video.c:453`).
    Monochrome,
}

/// How a radio line's channels become an RGB pixel — slowrx's `ColorEnc`
/// (`GBR`, `RGB`, `YUV`, `BW`; `common.h:105-107`), one per mode in
/// `modespec.c` and applied by the pixel switch at `video.c:432-455`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorEncoding {
    /// Three colour channels sent green, blue, red. Martin, Scottie.
    Gbr,
    /// Three colour channels sent red, green, blue. Wraase SC-2.
    Rgb,
    /// Luminance and colour difference, converted with slowrx's `Y'CbCr`
    /// formula. PD, colour Robot.
    Yuv,
    /// One luminance channel, shown grey. Robot B/W.
    Bw,
}

impl ColorEncoding {
    /// The RGB pixel for one column, from slowrx's three stored channel
    /// values `Image[tx][y][0..3]` (`video.c:432-455`): the channels in
    /// transmit order for [`Self::Gbr`] / [`Self::Rgb`], `[Y, Cr, Cb]` for
    /// [`Self::Yuv`], and only `[0]` read for [`Self::Bw`].
    #[must_use]
    pub(crate) fn to_rgb(self, stored: [u8; 3]) -> [u8; 3] {
        let [c0, c1, c2] = stored;
        match self {
            Self::Rgb => [c0, c1, c2],
            Self::Gbr => [c2, c0, c1],
            Self::Yuv => crate::demod::ycbcr_to_rgb(c0, c1, c2),
            Self::Bw => [c0; 3],
        }
    }
}

/// Where the sync pulse sits within a radio line.
///
/// PD/Robot/Martin/Wraase all place sync at line start (the standard SSTV
/// convention). Scottie modes are the exception — sync sits between B
/// and R channels, not at line start. Stored here so future mode
/// decoders are forced to make their sync placement explicit at dispatch
/// time, surfacing the V1 line-clock-advance assumption that sync ==
/// line start.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyncPosition {
    /// Sync pulse at the start of each radio line. PD, Robot, Martin,
    /// Wraase SC-2.
    /// Scottie family uses [`SyncPosition::Scottie`] instead.
    LineStart,
    /// Sync pulse between B and R within each radio line. Scottie family.
    Scottie,
}

impl ModeSpec {
    /// Radio lines per image — the `line_seconds` periods the decoder
    /// buffers, each starting with one sync pulse. PD packs two image rows
    /// into each radio line (`image_lines / 2`); Robot (colour and B/W),
    /// Scottie, Martin and Wraase SC-2 send one row per radio line. Mirrors
    /// slowrx `video.c:251-254`.
    /// Requires an even `image_lines` for PD (true of every PD mode; pinned
    /// by a test).
    #[must_use]
    pub(crate) fn radio_lines(&self) -> u32 {
        match self.channel_layout {
            ChannelLayout::PdYcbcr => self.image_lines / 2,
            ChannelLayout::RobotYuv | ChannelLayout::RgbSequential | ChannelLayout::Monochrome => {
                self.image_lines
            }
        }
    }

    /// Offset (seconds) applied to the raw `xmax`-derived skip to
    /// land on line 0's content start. `LineStart` modes return 0;
    /// `Scottie` modes return `-chan_len/2 + 2 × porch_seconds` (the
    /// Scottie sync is mid-line, so the slip-wrapped `xmax` needs
    /// to be hoisted back left to align with line 0's content
    /// start). Audit #88 B4.
    #[must_use]
    pub(crate) fn skip_correction_seconds(&self) -> f64 {
        match self.sync_position {
            SyncPosition::LineStart => 0.0,
            SyncPosition::Scottie => {
                let chan_len = f64::from(self.line_pixels) * self.pixel_seconds;
                -chan_len / 2.0 + 2.0 * self.porch_seconds
            }
        }
    }
}

/// Look up the [`ModeSpec`] for a given 7-bit VIS code. Returns `None`
/// if the code is reserved, undefined, or maps to a mode not yet
/// implemented in this release. Derived from `ALL_SPECS`.
///
/// VIS codes are taken from Dave Jones (KB4YZ), 1998: "List of SSTV
/// Modes with VIS Codes".
///
/// **Parity-audit note (#27):** `0x00` is intentionally unmapped and
/// returns `None`. In slowrx (`vis.c:172-174`), an unknown VIS code causes
/// `GetVIS()` to return 0 and `Listen()` loops back to re-detect
/// (`do { ... } while (Mode == 0)`). Rust's equivalent is `None` from
/// this function: the caller in `SstvDecoder::process` emits
/// `SstvEvent::UnknownVis`, reseeds the VIS detector on the post-stop-bit
/// residue, and stays in `AwaitingVis` — the same "try again" effect as
/// slowrx's re-detect loop (slowrx's `printf("Unknown VIS")` becomes the
/// `UnknownVis` event). An unknown code is never an `Error`.
#[must_use]
pub fn lookup(vis_code: u8) -> Option<ModeSpec> {
    ALL_SPECS.iter().find(|s| s.vis_code == vis_code).copied()
}

/// Look up the [`ModeSpec`] for an [`SstvMode`].
///
/// Total over [`SstvMode`] — every implemented variant has a `const`
/// entry. Adding a new variant without adding its `const ModeSpec`
/// (and an arm here) is a compile error, by design. Pair with
/// [`lookup`] when starting from a VIS code on the wire.
#[must_use]
pub fn for_mode(mode: SstvMode) -> ModeSpec {
    match mode {
        SstvMode::Pd120 => PD120,
        SstvMode::Pd180 => PD180,
        SstvMode::Pd240 => PD240,
        SstvMode::Robot24 => ROBOT24,
        SstvMode::Robot36 => ROBOT36,
        SstvMode::Robot72 => ROBOT72,
        SstvMode::Robot24Bw => ROBOT24_BW,
        SstvMode::Robot12Bw => ROBOT12_BW,
        SstvMode::Robot8Bw => ROBOT8_BW,
        SstvMode::Scottie1 => SCOTTIE1,
        SstvMode::Scottie2 => SCOTTIE2,
        SstvMode::ScottieDx => SCOTTIE_DX,
        SstvMode::Martin1 => MARTIN1,
        SstvMode::Martin2 => MARTIN2,
        SstvMode::Wraase120 => WRAASE120,
        SstvMode::Wraase180 => WRAASE180,
    }
}

// Mode timing constants — translated row-for-row from slowrx's
// modespec.c (PD120 lines 260-271, PD180 lines 286-297, PD240 lines 299-310,
// R72 lines 130-141, R36 lines 143-154, R24 lines 156-167, R24BW lines
// 169-180, R12BW lines 182-193, R8BW lines 195-206, W2120 lines 208-219,
// W2180 lines 221-232). `line_height` is slowrx's `LineHeight` column,
// `color_encoding` its `ColorEnc`.

const PD120: ModeSpec = ModeSpec {
    mode: SstvMode::Pd120,
    short_name: "pd120",
    name: "PD-120",
    vis_code: 0x5F,
    line_pixels: 640,
    image_lines: 496,
    line_height: 1,
    line_seconds: 0.508_48,
    sync_seconds: 0.020,
    porch_seconds: 0.002_08,
    pixel_seconds: 0.000_19,
    septr_seconds: 0.0, // modespec.c: SeptrTime = 0e-3 for PD-family
    channel_layout: ChannelLayout::PdYcbcr,
    sync_position: SyncPosition::LineStart,
    color_encoding: ColorEncoding::Yuv, // modespec.c:271 ColorEnc = YUV
};

const PD180: ModeSpec = ModeSpec {
    mode: SstvMode::Pd180,
    short_name: "pd180",
    name: "PD-180",
    vis_code: 0x60,
    line_pixels: 640,
    image_lines: 496,
    line_height: 1,
    line_seconds: 0.754_24,
    sync_seconds: 0.020,
    porch_seconds: 0.002_08,
    pixel_seconds: 0.000_286,
    septr_seconds: 0.0, // modespec.c: SeptrTime = 0e-3 for PD-family
    channel_layout: ChannelLayout::PdYcbcr,
    sync_position: SyncPosition::LineStart,
    color_encoding: ColorEncoding::Yuv, // modespec.c:297 ColorEnc = YUV
};

const PD240: ModeSpec = ModeSpec {
    mode: SstvMode::Pd240,
    short_name: "pd240",
    name: "PD-240",
    vis_code: 0x61,
    line_pixels: 640,
    image_lines: 496,
    line_height: 1,
    // slowrx modespec.c:299-310 — PD240 LineTime = 1000e-3,
    // PixelTime = 0.382e-3, SyncTime = 20e-3, PorchTime = 2.08e-3.
    line_seconds: 1.000,
    sync_seconds: 0.020,
    porch_seconds: 0.002_08,
    pixel_seconds: 0.000_382,
    septr_seconds: 0.0, // modespec.c: SeptrTime = 0e-3 for PD-family
    channel_layout: ChannelLayout::PdYcbcr,
    sync_position: SyncPosition::LineStart,
    color_encoding: ColorEncoding::Yuv, // modespec.c:310 ColorEnc = YUV
};

const ROBOT24: ModeSpec = ModeSpec {
    mode: SstvMode::Robot24,
    short_name: "robot24",
    name: "Robot 24",
    vis_code: 0x04,
    line_pixels: 320,
    image_lines: 240,
    line_height: 1,
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
    color_encoding: ColorEncoding::Yuv, // modespec.c:167 ColorEnc = YUV
};

const ROBOT36: ModeSpec = ModeSpec {
    mode: SstvMode::Robot36,
    short_name: "robot36",
    name: "Robot 36",
    vis_code: 0x08,
    line_pixels: 320,
    image_lines: 240,
    line_height: 1,
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
    color_encoding: ColorEncoding::Yuv, // modespec.c:154 ColorEnc = YUV
};

const ROBOT72: ModeSpec = ModeSpec {
    mode: SstvMode::Robot72,
    short_name: "robot72",
    name: "Robot 72",
    vis_code: 0x0C,
    line_pixels: 320,
    image_lines: 240,
    line_height: 1,
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
    color_encoding: ColorEncoding::Yuv, // modespec.c:141 ColorEnc = YUV
};

const SCOTTIE1: ModeSpec = ModeSpec {
    mode: SstvMode::Scottie1,
    short_name: "scottie1",
    name: "Scottie 1",
    vis_code: 0x3C,
    line_pixels: 320,
    image_lines: 256,
    line_height: 1,
    // slowrx modespec.c:91-102 — S1 LineTime = 428.38e-3,
    // PixelTime = 0.4320e-3, SyncTime = 9e-3, PorchTime = 1.5e-3,
    // SeptrTime = 1.5e-3.
    line_seconds: 0.428_38,
    sync_seconds: 0.009,
    porch_seconds: 0.001_5,
    pixel_seconds: 0.000_432_0,
    septr_seconds: 0.001_5,
    channel_layout: ChannelLayout::RgbSequential,
    sync_position: SyncPosition::Scottie,
    color_encoding: ColorEncoding::Gbr, // modespec.c:102 ColorEnc = GBR
};

const SCOTTIE2: ModeSpec = ModeSpec {
    mode: SstvMode::Scottie2,
    short_name: "scottie2",
    name: "Scottie 2",
    vis_code: 0x38,
    line_pixels: 320,
    image_lines: 256,
    line_height: 1,
    // slowrx modespec.c:104-115 — S2 LineTime = 277.692e-3,
    // PixelTime = 0.2752e-3, SyncTime = 9e-3, PorchTime = 1.5e-3,
    // SeptrTime = 1.5e-3.
    line_seconds: 0.277_692,
    sync_seconds: 0.009,
    porch_seconds: 0.001_5,
    pixel_seconds: 0.000_275_2,
    septr_seconds: 0.001_5,
    channel_layout: ChannelLayout::RgbSequential,
    sync_position: SyncPosition::Scottie,
    color_encoding: ColorEncoding::Gbr, // modespec.c:115 ColorEnc = GBR
};

const SCOTTIE_DX: ModeSpec = ModeSpec {
    mode: SstvMode::ScottieDx,
    short_name: "scottiedx",
    name: "Scottie DX",
    vis_code: 0x4C,
    line_pixels: 320,
    image_lines: 256,
    line_height: 1,
    // slowrx modespec.c:117-128 — SDX LineTime = 1050.3e-3,
    // PixelTime = 1.08053e-3, SyncTime = 9e-3, PorchTime = 1.5e-3,
    // SeptrTime = 1.5e-3.
    line_seconds: 1.050_3,
    sync_seconds: 0.009,
    porch_seconds: 0.001_5,
    pixel_seconds: 0.001_080_53,
    septr_seconds: 0.001_5,
    channel_layout: ChannelLayout::RgbSequential,
    sync_position: SyncPosition::Scottie,
    color_encoding: ColorEncoding::Gbr, // modespec.c:128 ColorEnc = GBR
};

/// Robot 24 B/W. slowrx `modespec.c:169-180`.
const ROBOT24_BW: ModeSpec = ModeSpec {
    mode: SstvMode::Robot24Bw,
    short_name: "robot24bw",
    name: "Robot 24 B/W",
    vis_code: 0x0A,
    line_pixels: 320,
    image_lines: 240,
    line_height: 1,
    // slowrx modespec.c:169-180 — R24BW LineTime = 100e-3,
    // PixelTime = 0.291e-3, SyncTime = 7e-3, PorchTime = 0e-3,
    // SeptrTime = 0e-3.
    line_seconds: 0.100,
    sync_seconds: 0.007,
    porch_seconds: 0.0,
    pixel_seconds: 0.000_291,
    septr_seconds: 0.0,
    channel_layout: ChannelLayout::Monochrome,
    sync_position: SyncPosition::LineStart,
    color_encoding: ColorEncoding::Bw, // modespec.c:180 ColorEnc = BW
};

/// Robot 12 B/W. slowrx `modespec.c:182-193`. Its VIS parity bit is
/// inverted (`vis.c:116`; see `crate::vis::R12BW_VIS_CODE`).
pub(crate) const ROBOT12_BW: ModeSpec = ModeSpec {
    mode: SstvMode::Robot12Bw,
    short_name: "robot12bw",
    name: "Robot 12 B/W",
    vis_code: 0x06,
    line_pixels: 320,
    image_lines: 120,
    line_height: 2,
    // slowrx modespec.c:182-193 — R12BW LineTime = 100e-3,
    // PixelTime = 0.291e-3, SyncTime = 7e-3, PorchTime = 0e-3,
    // SeptrTime = 0e-3. Identical line timing to R24BW, half the lines.
    line_seconds: 0.100,
    sync_seconds: 0.007,
    porch_seconds: 0.0,
    pixel_seconds: 0.000_291,
    septr_seconds: 0.0,
    channel_layout: ChannelLayout::Monochrome,
    sync_position: SyncPosition::LineStart,
    color_encoding: ColorEncoding::Bw, // modespec.c:193 ColorEnc = BW
};

/// Robot 8 B/W. slowrx `modespec.c:195-206`.
const ROBOT8_BW: ModeSpec = ModeSpec {
    mode: SstvMode::Robot8Bw,
    short_name: "robot8bw",
    name: "Robot 8 B/W",
    vis_code: 0x02,
    line_pixels: 320,
    image_lines: 120,
    line_height: 2,
    // slowrx modespec.c:195-206 — R8BW LineTime = 66.9e-3,
    // PixelTime = 0.1871875e-3, SyncTime = 7e-3, PorchTime = 0e-3,
    // SeptrTime = 0e-3.
    line_seconds: 0.066_9,
    sync_seconds: 0.007,
    porch_seconds: 0.0,
    pixel_seconds: 0.000_187_187_5,
    septr_seconds: 0.0,
    channel_layout: ChannelLayout::Monochrome,
    sync_position: SyncPosition::LineStart,
    color_encoding: ColorEncoding::Bw, // modespec.c:206 ColorEnc = BW
};

/// Martin 1. slowrx `modespec.c:39-50`.
const MARTIN1: ModeSpec = ModeSpec {
    mode: SstvMode::Martin1,
    short_name: "martin1",
    name: "Martin 1",
    vis_code: 0x2C,
    line_pixels: 320,
    image_lines: 256,
    line_height: 1,
    // slowrx modespec.c:39-50 — M1 LineTime = 446.446e-3,
    // PixelTime = 0.4576e-3, SyncTime = 4.862e-3,
    // PorchTime = 0.572e-3, SeptrTime = 0.572e-3.
    line_seconds: 0.446_446,
    sync_seconds: 0.004_862,
    porch_seconds: 0.000_572,
    pixel_seconds: 0.000_457_6,
    septr_seconds: 0.000_572,
    channel_layout: ChannelLayout::RgbSequential,
    sync_position: SyncPosition::LineStart,
    color_encoding: ColorEncoding::Gbr, // modespec.c:50 ColorEnc = GBR
};

/// Martin 2. slowrx `modespec.c:52-63`.
const MARTIN2: ModeSpec = ModeSpec {
    mode: SstvMode::Martin2,
    short_name: "martin2",
    name: "Martin 2",
    vis_code: 0x28,
    line_pixels: 320,
    image_lines: 256,
    line_height: 1,
    // slowrx modespec.c:52-63 — M2 LineTime = 226.7986e-3,
    // PixelTime = 0.2288e-3, SyncTime = 4.862e-3,
    // PorchTime = 0.572e-3, SeptrTime = 0.572e-3.
    line_seconds: 0.226_798_6,
    sync_seconds: 0.004_862,
    porch_seconds: 0.000_572,
    pixel_seconds: 0.000_228_8,
    septr_seconds: 0.000_572,
    channel_layout: ChannelLayout::RgbSequential,
    sync_position: SyncPosition::LineStart,
    color_encoding: ColorEncoding::Gbr, // modespec.c:63 ColorEnc = GBR
};

/// Wraase SC-2 120. slowrx `modespec.c:208-219`.
const WRAASE120: ModeSpec = ModeSpec {
    mode: SstvMode::Wraase120,
    short_name: "wraase120",
    name: "Wraase SC-2 120",
    vis_code: 0x3F,
    line_pixels: 320,
    image_lines: 256,
    line_height: 1,
    // slowrx modespec.c:208-219 — W2120 LineTime = 475.530018e-3,
    // PixelTime = 0.489039081e-3, SyncTime = 5.5225e-3,
    // PorchTime = 0.5e-3, SeptrTime = 0e-3.
    line_seconds: 0.475_530_018,
    sync_seconds: 0.005_522_5,
    porch_seconds: 0.000_5,
    pixel_seconds: 0.000_489_039_081,
    septr_seconds: 0.0,
    channel_layout: ChannelLayout::RgbSequential,
    sync_position: SyncPosition::LineStart,
    color_encoding: ColorEncoding::Rgb, // modespec.c:219 ColorEnc = RGB
};

/// Wraase SC-2 180. slowrx `modespec.c:221-232`.
const WRAASE180: ModeSpec = ModeSpec {
    mode: SstvMode::Wraase180,
    short_name: "wraase180",
    name: "Wraase SC-2 180",
    vis_code: 0x37,
    line_pixels: 320,
    image_lines: 256,
    line_height: 1,
    // slowrx modespec.c:221-232 — W2180 LineTime = 711.0225e-3,
    // PixelTime = 0.734532e-3, SyncTime = 5.5225e-3,
    // PorchTime = 0.5e-3, SeptrTime = 0e-3.
    line_seconds: 0.711_022_5,
    sync_seconds: 0.005_522_5,
    porch_seconds: 0.000_5,
    pixel_seconds: 0.000_734_532,
    septr_seconds: 0.0,
    channel_layout: ChannelLayout::RgbSequential,
    sync_position: SyncPosition::LineStart,
    color_encoding: ColorEncoding::Rgb, // modespec.c:232 ColorEnc = RGB
};

/// All implemented mode specs. Single source of truth — [`lookup`] is
/// derived from this; [`for_mode`] keeps its exhaustive match so
/// adding a `SstvMode` variant without a `const ModeSpec` (and a
/// matching arm in `for_mode`) is a compile error, by design.
///
/// The F8 round-trip test (`all_specs_roundtrip`) verifies every
/// entry's `(mode, vis_code, short_name, name)` quadruple is unique
/// and that `lookup` and `for_mode` agree with the table.
pub(crate) const ALL_SPECS: [ModeSpec; 16] = [
    PD120, PD180, PD240, ROBOT24, ROBOT36, ROBOT72, ROBOT24_BW, ROBOT12_BW, ROBOT8_BW, SCOTTIE1,
    SCOTTIE2, SCOTTIE_DX, MARTIN1, MARTIN2, WRAASE120, WRAASE180,
];

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
            SstvMode::Robot24Bw,
            SstvMode::Robot12Bw,
            SstvMode::Robot8Bw,
            SstvMode::Wraase120,
            SstvMode::Wraase180,
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

    #[test]
    fn martin1_modespec() {
        let spec = for_mode(SstvMode::Martin1);
        assert_eq!(spec.mode, SstvMode::Martin1);
        assert_eq!(spec.vis_code, 0x2C);
        assert_eq!(spec.line_pixels, 320);
        assert_eq!(spec.image_lines, 256);
        assert_eq!(spec.channel_layout, ChannelLayout::RgbSequential);
        assert_eq!(spec.sync_position, SyncPosition::LineStart);
        assert!((spec.pixel_seconds - 0.000_457_6).abs() < 1e-9);
        assert!((spec.line_seconds - 0.446_446).abs() < 1e-9);
    }

    #[test]
    fn martin2_modespec() {
        let spec = for_mode(SstvMode::Martin2);
        assert_eq!(spec.mode, SstvMode::Martin2);
        assert_eq!(spec.vis_code, 0x28);
        assert!((spec.pixel_seconds - 0.000_228_8).abs() < 1e-9);
        assert!((spec.line_seconds - 0.226_798_6).abs() < 1e-9);
        assert_eq!(spec.channel_layout, ChannelLayout::RgbSequential);
        assert_eq!(spec.sync_position, SyncPosition::LineStart);
    }

    #[test]
    fn martin_vis_codes_resolve() {
        assert_eq!(lookup(0x2C).expect("M1").mode, SstvMode::Martin1);
        assert_eq!(lookup(0x28).expect("M2").mode, SstvMode::Martin2);
    }

    /// The timing every Robot B/W mode shares (slowrx `modespec.c:169-206`):
    /// 7 ms sync, no porch or separator, 320 px, one luminance channel per
    /// line-start-synced radio line.
    fn assert_robot_bw_common(spec: ModeSpec) {
        assert_eq!(spec.line_pixels, 320, "{:?}", spec.mode);
        assert_eq!(spec.sync_seconds, 0.007, "{:?}", spec.mode);
        assert_eq!(spec.porch_seconds, 0.0, "{:?}", spec.mode);
        assert_eq!(spec.septr_seconds, 0.0, "{:?}", spec.mode);
        assert_eq!(spec.channel_layout, ChannelLayout::Monochrome);
        assert_eq!(spec.sync_position, SyncPosition::LineStart);
    }

    #[test]
    fn robot24bw_modespec() {
        let spec = for_mode(SstvMode::Robot24Bw);
        assert_robot_bw_common(spec);
        assert_eq!(spec.mode, SstvMode::Robot24Bw);
        assert_eq!(spec.vis_code, 0x0A);
        assert_eq!(spec.short_name, "robot24bw");
        assert_eq!(spec.name, "Robot 24 B/W");
        assert_eq!(spec.image_lines, 240);
        assert_eq!(spec.line_height, 1);
        assert_eq!(spec.line_seconds, 0.100);
        assert_eq!(spec.pixel_seconds, 0.000_291);
    }

    #[test]
    fn robot12bw_modespec() {
        let spec = for_mode(SstvMode::Robot12Bw);
        assert_robot_bw_common(spec);
        assert_eq!(spec.mode, SstvMode::Robot12Bw);
        assert_eq!(spec.vis_code, 0x06);
        assert_eq!(spec.short_name, "robot12bw");
        assert_eq!(spec.name, "Robot 12 B/W");
        assert_eq!(spec.image_lines, 120);
        assert_eq!(spec.line_height, 2);
        assert_eq!(spec.line_seconds, 0.100);
        assert_eq!(spec.pixel_seconds, 0.000_291);
    }

    #[test]
    fn robot8bw_modespec() {
        let spec = for_mode(SstvMode::Robot8Bw);
        assert_robot_bw_common(spec);
        assert_eq!(spec.mode, SstvMode::Robot8Bw);
        assert_eq!(spec.vis_code, 0x02);
        assert_eq!(spec.short_name, "robot8bw");
        assert_eq!(spec.name, "Robot 8 B/W");
        assert_eq!(spec.image_lines, 120);
        assert_eq!(spec.line_height, 2);
        assert_eq!(spec.line_seconds, 0.066_9);
        assert_eq!(spec.pixel_seconds, 0.000_187_187_5);
    }

    /// slowrx `VISmap` (`modespec.c:376`): `0x02` R8BW, `0x06` R12BW,
    /// `0x0A` R24BW.
    #[test]
    fn robot_bw_vis_codes_resolve() {
        assert_eq!(lookup(0x02).expect("R8BW").mode, SstvMode::Robot8Bw);
        assert_eq!(lookup(0x06).expect("R12BW").mode, SstvMode::Robot12Bw);
        assert_eq!(lookup(0x0A).expect("R24BW").mode, SstvMode::Robot24Bw);
    }

    /// The timing both Wraase SC-2 modes share (slowrx `modespec.c:208-232`):
    /// 5.5225 ms sync, 0.5 ms porch, no separator, 320 × 256 at
    /// `LineHeight` 1, three RGB channels per line-start-synced radio line.
    fn assert_wraase_common(spec: ModeSpec) {
        assert_eq!(spec.line_pixels, 320, "{:?}", spec.mode);
        assert_eq!(spec.image_lines, 256, "{:?}", spec.mode);
        assert_eq!(spec.line_height, 1, "{:?}", spec.mode);
        assert_eq!(spec.sync_seconds, 5.5225e-3, "{:?}", spec.mode);
        assert_eq!(spec.porch_seconds, 0.5e-3, "{:?}", spec.mode);
        assert_eq!(spec.septr_seconds, 0.0, "{:?}", spec.mode);
        assert_eq!(spec.channel_layout, ChannelLayout::RgbSequential);
        assert_eq!(spec.sync_position, SyncPosition::LineStart);
        assert_eq!(spec.color_encoding, ColorEncoding::Rgb);
    }

    /// slowrx `modespec.c:208-219` (`W2120`), compared against its literals.
    #[test]
    fn wraase120_modespec() {
        let spec = for_mode(SstvMode::Wraase120);
        assert_wraase_common(spec);
        assert_eq!(spec.mode, SstvMode::Wraase120);
        assert_eq!(spec.vis_code, 0x3F);
        assert_eq!(spec.short_name, "wraase120");
        assert_eq!(spec.name, "Wraase SC-2 120");
        assert_eq!(spec.pixel_seconds, 0.489_039_081e-3);
        assert_eq!(spec.line_seconds, 475.530_018e-3);
    }

    /// slowrx `modespec.c:221-232` (`W2180`), compared against its literals.
    #[test]
    fn wraase180_modespec() {
        let spec = for_mode(SstvMode::Wraase180);
        assert_wraase_common(spec);
        assert_eq!(spec.mode, SstvMode::Wraase180);
        assert_eq!(spec.vis_code, 0x37);
        assert_eq!(spec.short_name, "wraase180");
        assert_eq!(spec.name, "Wraase SC-2 180");
        assert_eq!(spec.pixel_seconds, 0.734_532e-3);
        assert_eq!(spec.line_seconds, 711.022_5e-3);
    }

    /// slowrx `VISmap` row `0x30` (`modespec.c:379`): `0x37` W2180,
    /// `0x3F` W2120.
    #[test]
    fn wraase_vis_codes_resolve() {
        assert_eq!(lookup(0x37).expect("W2180").mode, SstvMode::Wraase180);
        assert_eq!(lookup(0x3F).expect("W2120").mode, SstvMode::Wraase120);
    }

    /// Every spec carries its slowrx `ColorEnc` (`modespec.c`, one line per
    /// mode). Listed per mode, not `_`, so a new mode must state its own.
    #[test]
    fn color_encoding_matches_slowrx() {
        for spec in ALL_SPECS {
            let expected = match spec.mode {
                // PD-120 / 180 / 240: modespec.c:271, 297, 310.
                // R24 / R36 / R72: modespec.c:167, 154, 141.
                SstvMode::Pd120
                | SstvMode::Pd180
                | SstvMode::Pd240
                | SstvMode::Robot24
                | SstvMode::Robot36
                | SstvMode::Robot72 => ColorEncoding::Yuv,
                // R24BW / R12BW / R8BW: modespec.c:180, 193, 206.
                SstvMode::Robot24Bw | SstvMode::Robot12Bw | SstvMode::Robot8Bw => ColorEncoding::Bw,
                // S1 / S2 / SDX: modespec.c:102, 115, 128. M1 / M2: :50, :63.
                SstvMode::Scottie1
                | SstvMode::Scottie2
                | SstvMode::ScottieDx
                | SstvMode::Martin1
                | SstvMode::Martin2 => ColorEncoding::Gbr,
                // W2120 / W2180: modespec.c:219, 232.
                SstvMode::Wraase120 | SstvMode::Wraase180 => ColorEncoding::Rgb,
            };
            assert_eq!(spec.color_encoding, expected, "{:?}", spec.mode);
        }
    }

    /// The colour encoding and the channel layout describe the same lines:
    /// the RGB-sequential decoder sees only `Gbr` / `Rgb`, the luma-chroma
    /// layouts only `Yuv`, the one-channel layout only `Bw`.
    #[test]
    fn color_encoding_agrees_with_channel_layout() {
        for spec in ALL_SPECS {
            let ok = match spec.channel_layout {
                ChannelLayout::RgbSequential => {
                    matches!(spec.color_encoding, ColorEncoding::Gbr | ColorEncoding::Rgb)
                }
                ChannelLayout::PdYcbcr | ChannelLayout::RobotYuv => {
                    spec.color_encoding == ColorEncoding::Yuv
                }
                ChannelLayout::Monochrome => spec.color_encoding == ColorEncoding::Bw,
            };
            assert!(ok, "{:?}: {:?}", spec.mode, spec.color_encoding);
        }
    }

    /// slowrx's pixel switch (`video.c:432-455`), from a row's three stored
    /// channels (`Image[tx][y][0..3]`, in transmit order) to the RGB pixel.
    #[test]
    fn color_encoding_to_rgb_follows_slowrx() {
        let stored = [10_u8, 120, 230];
        // RGB: p[i] = Image[i] (video.c:434-437).
        assert_eq!(ColorEncoding::Rgb.to_rgb(stored), [10, 120, 230]);
        // GBR: p[0] = Image[2], p[1] = Image[0], p[2] = Image[1]
        // (video.c:440-443) — Martin / Scottie send G, B, R.
        assert_eq!(ColorEncoding::Gbr.to_rgb(stored), [230, 10, 120]);
        // YUV: the video.c:446-450 conversion.
        assert_eq!(
            ColorEncoding::Yuv.to_rgb(stored),
            crate::demod::ycbcr_to_rgb(10, 120, 230)
        );
        // BW: p[0] = p[1] = p[2] = Image[0] (video.c:453).
        assert_eq!(ColorEncoding::Bw.to_rgb(stored), [10, 10, 10]);
    }

    /// `LineHeight` is 2 only for the two 120-line B/W modes; every other
    /// mode shows each decoded row once (slowrx `modespec.c`).
    #[test]
    fn line_height_is_two_only_for_r12bw_and_r8bw() {
        for spec in ALL_SPECS {
            let expected = match spec.mode {
                SstvMode::Robot12Bw | SstvMode::Robot8Bw => 2,
                _ => 1,
            };
            assert_eq!(spec.line_height, expected, "{:?}", spec.mode);
        }
    }

    /// `radio_lines` halves PD's `image_lines` with integer division, so
    /// every PD mode must have an even image height (audit A5).
    #[test]
    fn pd_radio_lines_cover_every_image_row() {
        for spec in ALL_SPECS {
            let rows_per_radio_line = match spec.channel_layout {
                ChannelLayout::PdYcbcr => 2,
                ChannelLayout::RobotYuv
                | ChannelLayout::RgbSequential
                | ChannelLayout::Monochrome => 1,
            };
            assert_eq!(
                spec.radio_lines() * rows_per_radio_line,
                spec.image_lines,
                "{:?}",
                spec.mode
            );
        }
    }

    #[test]
    fn skip_correction_seconds_zero_for_line_start_modes() {
        for mode in [
            SstvMode::Pd120,
            SstvMode::Pd240,
            SstvMode::Pd180,
            SstvMode::Robot24,
            SstvMode::Robot36,
            SstvMode::Robot72,
            SstvMode::Robot24Bw,
            SstvMode::Robot12Bw,
            SstvMode::Robot8Bw,
            SstvMode::Martin1,
            SstvMode::Martin2,
            SstvMode::Wraase120,
            SstvMode::Wraase180,
        ] {
            let spec = for_mode(mode);
            assert_eq!(
                spec.skip_correction_seconds(),
                0.0,
                "{mode:?} expected 0.0 skip correction"
            );
        }
    }

    #[test]
    fn skip_correction_seconds_scottie_formula() {
        for mode in [SstvMode::Scottie1, SstvMode::Scottie2, SstvMode::ScottieDx] {
            let spec = for_mode(mode);
            let expected =
                -f64::from(spec.line_pixels) * spec.pixel_seconds / 2.0 + 2.0 * spec.porch_seconds;
            assert!(
                (spec.skip_correction_seconds() - expected).abs() < 1e-12,
                "{mode:?} got {} expected {expected}",
                spec.skip_correction_seconds()
            );
            assert!(
                spec.skip_correction_seconds() < 0.0,
                "{mode:?} Scottie correction should be negative"
            );
        }
    }

    /// F8 (#91). Every entry in `ALL_SPECS` round-trips cleanly
    /// through `lookup` (VIS code → spec) and `for_mode` (mode →
    /// spec); the table has exactly 16 unique modes, 16 unique VIS
    /// codes, 16 unique `short_names`; every `name` and `short_name`
    /// is non-empty.
    ///
    /// Subsumes the per-mode `vis_code_resolves` tests as a
    /// structural invariant. The individual per-mode tests stay as
    /// fast-failing regression guards with descriptive names.
    #[test]
    fn all_specs_roundtrip() {
        use std::collections::HashSet;

        let modes: HashSet<_> = ALL_SPECS.iter().map(|s| s.mode).collect();
        assert_eq!(
            modes.len(),
            ALL_SPECS.len(),
            "ALL_SPECS has duplicate modes"
        );

        let vis: HashSet<_> = ALL_SPECS.iter().map(|s| s.vis_code).collect();
        assert_eq!(
            vis.len(),
            ALL_SPECS.len(),
            "ALL_SPECS has duplicate VIS codes"
        );

        let short_names: HashSet<_> = ALL_SPECS.iter().map(|s| s.short_name).collect();
        assert_eq!(
            short_names.len(),
            ALL_SPECS.len(),
            "ALL_SPECS has duplicate short_names"
        );

        let names: HashSet<_> = ALL_SPECS.iter().map(|s| s.name).collect();
        assert_eq!(
            names.len(),
            ALL_SPECS.len(),
            "ALL_SPECS has duplicate `name`s"
        );

        for spec in ALL_SPECS.iter().copied() {
            assert_eq!(
                lookup(spec.vis_code),
                Some(spec),
                "lookup({:#04x}) did not return ALL_SPECS entry for {:?}",
                spec.vis_code,
                spec.mode
            );
            assert_eq!(
                for_mode(spec.mode),
                spec,
                "for_mode({:?}) did not match ALL_SPECS entry",
                spec.mode
            );
            assert!(
                !spec.short_name.is_empty(),
                "{:?}: short_name empty",
                spec.mode
            );
            assert!(!spec.name.is_empty(), "{:?}: name empty", spec.mode);
        }
    }
}
