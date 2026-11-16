//! The MULTILEADER text **angle type** (`174` entity / `175` style) and the two
//! entity override bits creation uses.
//!
//! Split out of `multileader.rs` for the code-size gate: this is the DXF mirror
//! plus the mask constants, with no geometry of its own.

/// The annotation text **angle type**: DXF entity `174` (mirrored on the style
/// as `175`, property-override mask bit `0x2000`).
///
/// This group is a *mode*, not a rotation: it says how the drawn text follows
/// the leader, and the three values are exactly the ones AutoCAD stores. A
/// rotation in degrees is a different quantity and has no group here, which is
/// why the typed creation option takes a mode token and refuses a number.
///
/// [`Self::Horizontal`] is the default and the value the writer already emitted
/// before this field existed (acadrust's own default), so every leader written
/// by an earlier build re-imports to the same value and every re-export is
/// byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MLeaderTextAngle {
    /// `174 = 0`: the text is parallel to the last leader line segment.
    ParallelToLastLeaderLine,
    /// `174 = 1`: the text is always horizontal (the writer's default).
    #[default]
    Horizontal,
    /// `174 = 2`: parallel to the last leader line segment, flipped upright when
    /// the segment would draw it upside down.
    Optimized,
}

impl MLeaderTextAngle {
    /// The DXF code of this mode (`174` / `175`).
    pub fn dxf(self) -> i16 {
        match self {
            Self::ParallelToLastLeaderLine => 0,
            Self::Horizontal => 1,
            Self::Optimized => 2,
        }
    }

    /// The canonical typed token of this mode (`PARALLEL` = `0`, `HORIZONTAL` =
    /// `1`, `OPTIMIZED` = `2`).
    pub fn token(self) -> &'static str {
        match self {
            Self::ParallelToLastLeaderLine => "PARALLEL",
            Self::Horizontal => "HORIZONTAL",
            Self::Optimized => "OPTIMIZED",
        }
    }

    /// The mode a typed token names, case-insensitively.
    pub fn from_token(token: &str) -> Option<Self> {
        if token.eq_ignore_ascii_case("PARALLEL") {
            Some(Self::ParallelToLastLeaderLine)
        } else if token.eq_ignore_ascii_case("HORIZONTAL") {
            Some(Self::Horizontal)
        } else if token.eq_ignore_ascii_case("OPTIMIZED") {
            Some(Self::Optimized)
        } else {
            None
        }
    }

    /// The mode a DXF code names, with the same clamp acadrust applies (an
    /// unknown code reads as [`Self::Horizontal`], the file's default).
    pub fn from_dxf(value: i16) -> Self {
        match value {
            0 => Self::ParallelToLastLeaderLine,
            2 => Self::Optimized,
            _ => Self::Horizontal,
        }
    }
}

/// The DXF `90` property-override bit for the content type (`172` over style
/// `170`).
///
/// Creation sets it only when the created content kind differs from the style's
/// declared kind (a content-less leader under an MText style, or the reverse),
/// so the entity's own value is the one a reload resolves.
pub const MASK_CONTENT_TYPE: u32 = 0x400;

/// The DXF `90` property-override bit for the annotation text angle type
/// (entity `174` over style `175`).
///
/// Creation sets it only when
/// [`MLeaderCreateOptions::text_angle`](crate::MLeaderCreateOptions::text_angle)
/// names a mode that differs from the resolved style's, so the requested mode
/// survives a reload instead of re-resolving from `175`.
pub const MASK_TEXT_ANGLE: u32 = 0x2000;
