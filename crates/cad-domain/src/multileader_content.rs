//! MULTILEADER annotation **content**: the kind (DXF entity `172`) and the
//! payload each kind carries (block content, tolerance frame).
//!
//! Split out of `multileader.rs` for the code-size gate. There is exactly **one**
//! display derivation for every kind: [`MultiLeaderGeometry::build_display`]
//! calls [`MLeaderContent::display`], and nothing else synthesises a
//! multileader's content geometry.
//!
//! ## What each kind carries
//!
//! | kind | DXF entity `172` | stored payload | DXF payload groups |
//! |---|---|---|---|
//! | [`MLeaderContentKind::None`] | `0` | none | — |
//! | [`MLeaderContentKind::Block`] | `1` | [`MLeaderBlockContent`] | `CONTEXT_DATA 341` (block handle), `14`/`15`/`16` (normal/location/scale), `46` (rotation), `177` (connection); entity `344`, `10`, `43`, `176` |
//! | [`MLeaderContentKind::Text`] | `2` | [`MLeaderText`](crate::MLeaderText) | `CONTEXT_DATA 304`/`12`/`41`/`42` |
//! | [`MLeaderContentKind::Tolerance`] | `3` | [`MLeaderToleranceContent`] | `CONTEXT_DATA 304` (the `\|`-separated frame string), `12`, `41` (cell height) |
//!
//! The MText string and the tolerance string share the `CONTEXT_DATA` text
//! fields: DXF has no separate tolerance payload (the ezdxf group layout and
//! acadrust 0.6.8's `MultiLeaderAnnotContext` both carry exactly one
//! `text_string`/`text_height` pair), so the tolerance kind is
//! `content_type = 3` **plus** the text fields.
//!
//! ## Declared limits
//!
//! * **Block content** draws through the ordinary block-instance path
//!   ([`SemanticGeometry::Insert`]); the block's base point is placed at the
//!   content point, exactly like a block reference, so the provider expands the
//!   one definition it already has. A content block whose handle does not resolve
//!   to a known definition keeps the kind (and is reported by the display)
//!   instead of being silently dropped.
//! * **Tolerance content** draws the same feature-control frame the `TOLERANCE`
//!   command builds ([`tolerance_frame_geometries`] is their one shared rule):
//!   one closed border polyline, one divider line per compartment boundary and
//!   one `TEXT` per compartment. AutoCAD's anchor for a tolerance frame attached
//!   to a leader is **unverified against a vendor file**; this build puts the
//!   frame's lower-left corner on the stored content point (the `TOLERANCE`
//!   command's own convention).
//! * `CONTEXT_DATA 47` (the block-content transform matrix), `14` (block content
//!   normal), `93` (block colour) and the block-attribute records have no model
//!   home here; the importer names the attribute loss and the general
//!   colour/property deviation covers the rest.

use crate::dimension_synthesis::is_finite_point;
use crate::{
    BlockId, CadError, CadResult, Point3, SemanticGeometry, StyleId, TextAlignH, TextAlignV,
    Transform3,
};

// ---------------------------------------------------------------------------
// The kind.
// ---------------------------------------------------------------------------

/// The content **kind** of a multileader (DXF entity `172` / style `170`).
///
/// The numeric codes are the format's: `0` none, `1` block, `2` MText and `3`
/// tolerance (`LeaderContentType` in acadrust 0.6.8 and ezdxf's documented
/// `content_type` table agree). The earlier pre-P7b note that `1` was MText was
/// wrong; the writer, the reader and the style record all use these codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MLeaderContentKind {
    /// `0`: no annotation content (a real AutoCAD case).
    #[default]
    None,
    /// `1`: the content is an inserted block.
    Block,
    /// `2`: the annotation is MText.
    Text,
    /// `3`: the annotation is a feature-control (tolerance) frame.
    Tolerance,
}

impl MLeaderContentKind {
    /// The DXF `172`/`170` code of this kind.
    pub fn dxf(self) -> i16 {
        match self {
            Self::None => 0,
            Self::Block => 1,
            Self::Text => 2,
            Self::Tolerance => 3,
        }
    }

    /// The kind a DXF `172`/`170` code names, when the code is one of the four.
    pub fn from_dxf(code: i16) -> Option<Self> {
        match code {
            0 => Some(Self::None),
            1 => Some(Self::Block),
            2 => Some(Self::Text),
            3 => Some(Self::Tolerance),
            _ => None,
        }
    }

    /// The upper-case token the diagnostics use.
    pub fn token(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Block => "BLOCK",
            Self::Text => "MTEXT",
            Self::Tolerance => "TOLERANCE",
        }
    }
}

// ---------------------------------------------------------------------------
// The content with its payload.
// ---------------------------------------------------------------------------

/// The annotation content of a multileader, with the payload each kind carries.
///
/// The MText string lives in [`MLeaderText`](crate::MLeaderText) (one home, the
/// same one the text-edit channels write); this enum carries only what the other
/// kinds need. `PartialEq` compares the payload too, so a round-trip assertion
/// cannot pass on the kind alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum MLeaderContent {
    /// `172 = 0`: no annotation.
    #[default]
    None,
    /// `172 = 1`: an inserted block.
    Block(MLeaderBlockContent),
    /// `172 = 2`: MText, carried by [`MLeaderText`](crate::MLeaderText).
    Text,
    /// `172 = 3`: a feature-control frame.
    Tolerance(MLeaderToleranceContent),
}

impl MLeaderContent {
    /// The kind of this content (the value entity `172` stores).
    pub fn kind(&self) -> MLeaderContentKind {
        match self {
            Self::None => MLeaderContentKind::None,
            Self::Block(_) => MLeaderContentKind::Block,
            Self::Text => MLeaderContentKind::Text,
            Self::Tolerance(_) => MLeaderContentKind::Tolerance,
        }
    }

    /// The DXF `172` code of this content.
    pub fn dxf(&self) -> i16 {
        self.kind().dxf()
    }

    /// The block payload, when this content is a block.
    pub fn block(&self) -> Option<&MLeaderBlockContent> {
        match self {
            Self::Block(block) => Some(block),
            _ => None,
        }
    }

    /// The tolerance payload, when this content is a tolerance frame.
    pub fn tolerance(&self) -> Option<&MLeaderToleranceContent> {
        match self {
            Self::Tolerance(tolerance) => Some(tolerance),
            _ => None,
        }
    }

    /// Refuse a content definition the display cannot derive, and return the
    /// annotation height the stored text must carry.
    ///
    /// Refusals (named): `mleader.text_required` (MText content with an empty
    /// string), `mleader.block_content_invalid` (a non-finite block location,
    /// rotation or scale, or a collapsing scale factor) and
    /// `mleader.tolerance_text_required` / `mleader.tolerance_height` (an empty
    /// frame, or a height that is not finite and positive).
    pub(crate) fn validate(&self, text: &crate::MLeaderText) -> CadResult<f64> {
        match self {
            Self::None | Self::Text => Ok(text.height),
            Self::Block(block) => {
                block.validate()?;
                Ok(text.height)
            }
            Self::Tolerance(tolerance) => {
                if tolerance.text.trim().is_empty() {
                    return Err(CadError::InvalidInput(
                        "mleader.tolerance_text_required: tolerance content needs a non-empty \
                         frame string"
                            .into(),
                    ));
                }
                if !tolerance.height.is_finite() || tolerance.height <= 0.0 {
                    return Err(CadError::InvalidInput(format!(
                        "mleader.tolerance_height: the tolerance frame height must be finite and \
                         positive, got {}",
                        tolerance.height
                    )));
                }
                Ok(tolerance.height)
            }
        }
    }

    /// The display children this content contributes, plus the honest reasons for
    /// content that cannot be drawn.
    ///
    /// `origin` is the stored content point and `style`/`font` the resolved text
    /// style the kind's text is drawn with.
    pub(crate) fn display(
        &self,
        text: &crate::MLeaderText,
        style: StyleId,
        font: Option<String>,
    ) -> (Vec<SemanticGeometry>, Vec<String>) {
        match self {
            Self::None => (Vec::new(), Vec::new()),
            Self::Text => {
                if text.string.is_empty() {
                    return (Vec::new(), Vec::new());
                }
                // The text child is the geometry owner's `text_geometry` (it
                // honours the text angle type), so this arm only reports
                // presence; see `MultiLeaderGeometry::build_display`.
                (Vec::new(), Vec::new())
            }
            Self::Block(block) => match block.display() {
                Some(insert) => (vec![insert], Vec::new()),
                // Two distinct losses, each named: an unresolved identity cannot
                // draw anything, and a resolved identity without a base point
                // cannot be placed without inventing an origin.
                None if block.block.is_none() => (
                    Vec::new(),
                    vec!["multileader block content handle did not resolve to a block".to_string()],
                ),
                None => (
                    Vec::new(),
                    vec![
                        "multileader block content has no stored base point, so its placement \
                         cannot be derived"
                            .to_string(),
                    ],
                ),
            },
            Self::Tolerance(tolerance) => {
                let cells = tolerance.cells();
                if cells.is_empty() {
                    return (
                        Vec::new(),
                        vec![
                            "multileader tolerance content has no compartment to frame".to_string()
                        ],
                    );
                }
                let frame = tolerance_frame_geometries(
                    text.location,
                    &cells,
                    tolerance.height,
                    style,
                    font,
                )
                .unwrap_or_default();
                (frame, Vec::new())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Block content.
// ---------------------------------------------------------------------------

/// Block content: the leader's annotation is an inserted block (DXF `172 = 1`).
///
/// Both halves of the identity are stored: `block` is the drawing-local
/// [`BlockId`] the display expands (the provider already knows how to expand a
/// block reference), and `name`/`handle` are what the DXF record carries and what
/// a reload resolves again. `block` is `None` for a source whose block handle did
/// not resolve — the content kind is still preserved and the display reports it,
/// instead of silently dropping the annotation.
///
/// ## Placement (`base_point`)
///
/// The content point receives the **block's own base point**, exactly like a
/// block reference: the placement is
/// `T(location) · R(rotation) · S(scale) · T(−base_point)` (see
/// [`Self::transform`]), which is the composition `INSERT` uses
/// (`crates/cad-app/src/app_drawing/insert_block.rs`: `T(insertion) · T(−base)`)
/// and the one the importer builds for an `AcDbBlockReference`
/// (`insert_array_transform`). `base_point` is therefore **stored** — the display
/// and the exporter never re-read the block definition to place the content.
///
/// `base_point` is `None` only when the definition's metadata is unavailable (an
/// import whose block name/handle did not resolve): the display then refuses to
/// draw the block rather than guessing an origin.
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderBlockContent {
    /// The drawing-local block the display expands, when the handle/name resolved.
    pub block: Option<BlockId>,
    /// The block name (DXF `344`/`341` resolve to it in the source document).
    pub name: String,
    /// The source `BLOCK_RECORD` handle, preserved verbatim for round-trip.
    pub handle: Option<u64>,
    /// The definition's insertion base point (`BlockMetadata::base_point`): the
    /// point the placement puts on [`Self::location`]. `None` = unavailable.
    pub base_point: Option<Point3>,
    /// The content point (`CONTEXT_DATA 15`), i.e. where the base point lands.
    pub location: Point3,
    /// The three scale factors (`CONTEXT_DATA 16`, entity `10`).
    pub scale: Point3,
    /// The block rotation in radians (`CONTEXT_DATA 46`, entity `43`).
    pub rotation: f64,
    /// The connection type (`CONTEXT_DATA 177`, entity `176`): `0` block extents,
    /// `1` block base point.
    pub connection: i16,
}

impl MLeaderBlockContent {
    /// A block content placed at `location` with unit scale and no rotation —
    /// the created leader's definition.
    ///
    /// `base_point` is the definition's stored base point (`BlockMetadata`): the
    /// caller resolves it from the drawing's block table, which is what keeps the
    /// placement identical to an `INSERT` of the same block.
    pub fn placed(block: BlockId, name: String, base_point: Point3, location: Point3) -> Self {
        MLeaderBlockContent {
            block: Some(block),
            name,
            handle: None,
            base_point: Some(base_point),
            location,
            scale: Point3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
            rotation: 0.0,
            connection: 0,
        }
    }

    /// Refuse a block payload the display cannot derive (`mleader.block_content_invalid`).
    pub(crate) fn validate(&self) -> CadResult<()> {
        if !is_finite_point(self.location) || !self.rotation.is_finite() {
            return Err(CadError::InvalidInput(
                "mleader.block_content_invalid: the block content location and rotation must be \
                 finite"
                    .into(),
            ));
        }
        if self.base_point.is_some_and(|base| !is_finite_point(base)) {
            return Err(CadError::InvalidInput(
                "mleader.block_content_invalid: the block's base point must be finite".into(),
            ));
        }
        if !is_finite_point(self.scale) {
            return Err(CadError::InvalidInput(
                "mleader.block_content_invalid: the block content scale factors must be finite"
                    .into(),
            ));
        }
        if self.scale.x == 0.0 || self.scale.y == 0.0 || self.scale.z == 0.0 {
            return Err(CadError::InvalidInput(
                "mleader.block_content_invalid: a block content scale factor is zero, which \
                 collapses the block and no DXF INSERT placement can express it"
                    .into(),
            ));
        }
        Ok(())
    }

    /// The placement transform the display expands the block with, when the
    /// definition's base point is known.
    ///
    /// `T(location) · R(rotation) · S(scale) · T(−base_point)`, the same
    /// composition `INSERT` and the importer's `AcDbBlockReference` expansion use:
    /// the block's base point lands on the stored content point.
    /// `transform.apply_point(base_point) == location` holds for every finite
    /// input, and for a zero base point the base-point factor is the identity (so
    /// a zero-base block draws exactly as it did before the compensation).
    pub fn transform(&self) -> Option<Transform3> {
        let base = self.base_point?;
        let (sin, cos) = self.rotation.sin_cos();
        let mut matrix = [[0.0f64; 4]; 4];
        matrix[0][0] = cos * self.scale.x;
        matrix[0][1] = -sin * self.scale.y;
        matrix[1][0] = sin * self.scale.x;
        matrix[1][1] = cos * self.scale.y;
        matrix[2][2] = self.scale.z;
        // `T(location) · R · S · T(−base)`: the rotation/scale act on the
        // base-point offset, then the content point places the result.
        matrix[0][3] =
            self.location.x - (cos * self.scale.x * base.x - sin * self.scale.y * base.y);
        matrix[1][3] =
            self.location.y - (sin * self.scale.x * base.x + cos * self.scale.y * base.y);
        matrix[2][3] = self.location.z - self.scale.z * base.z;
        matrix[3][3] = 1.0;
        Some(Transform3 { matrix })
    }

    /// The one display child of block content: the block instance.
    pub fn display(&self) -> Option<SemanticGeometry> {
        let block = self.block?;
        Some(SemanticGeometry::Insert {
            block,
            transform: self.transform()?,
            attributes: Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tolerance content.
// ---------------------------------------------------------------------------

/// Tolerance content: the leader's annotation is a feature-control frame
/// (DXF `172 = 3`).
///
/// The string is the `|`-separated compartment text the `TOLERANCE` command
/// takes (for example `|0.05|A|B`), stored verbatim so it round-trips; `height`
/// is the cell text height (`CONTEXT_DATA 41`, the same group the MText height
/// uses — the constructor mirrors it onto the stored text height so the two can
/// never disagree).
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderToleranceContent {
    /// The frame text, `|`-separated into compartments, verbatim.
    pub text: String,
    /// The cell text height (`CONTEXT_DATA 41`).
    pub height: f64,
}

impl MLeaderToleranceContent {
    /// The non-empty compartments of the frame string, trimmed.
    ///
    /// An empty compartment is dropped (the `TOLERANCE` command refuses one, a
    /// source file need not be that tidy) and a frame with no compartment at all
    /// draws nothing — the display reports it.
    pub fn cells(&self) -> Vec<String> {
        self.text
            .split('|')
            .map(str::trim)
            .filter(|cell| !cell.is_empty())
            .map(str::to_string)
            .collect()
    }
}

/// The declared char-width estimate of one frame cell text.
///
/// The importer's frame rule and the `TOLERANCE` command's: exact glyph metrics
/// are a representation-layer concern, so the cell width is this estimate.
pub const TOLERANCE_CHAR_WIDTH_FACTOR: f64 = 0.62;

/// The declared row-height factor of one frame row.
pub const TOLERANCE_ROW_HEIGHT_FACTOR: f64 = 1.6;

/// The horizontal padding on each side of a cell, as a fraction of the height.
pub const TOLERANCE_CELL_PADDING_FACTOR: f64 = 0.5;

/// Build the feature-control frame of one tolerance: border + dividers + cells.
///
/// This is **the** frame rule — the `TOLERANCE` command and the MULTILEADER
/// tolerance display both call it, so the two can never draw different frames.
/// `origin` is the frame's lower-left corner; `cells` must each be non-empty.
///
/// Refusal (`tolerance.height`): the derived extent is not finite, which can only
/// happen for a non-finite height or origin.
pub fn tolerance_frame_geometries(
    origin: Point3,
    cells: &[String],
    height: f64,
    style: StyleId,
    font: Option<String>,
) -> CadResult<Vec<SemanticGeometry>> {
    if cells.is_empty() {
        return Ok(Vec::new());
    }
    let row_height = height * TOLERANCE_ROW_HEIGHT_FACTOR;
    let padding = height * TOLERANCE_CELL_PADDING_FACTOR;
    let widths: Vec<f64> = cells
        .iter()
        .map(|cell| {
            let chars = cell.chars().count().max(1) as f64;
            chars * height * TOLERANCE_CHAR_WIDTH_FACTOR + 2.0 * padding
        })
        .collect();
    let total_width: f64 = widths.iter().sum();
    if !total_width.is_finite() || total_width <= 0.0 || !row_height.is_finite() {
        return Err(CadError::InvalidInput(
            "tolerance.height: the frame extent is not finite; check the text height".into(),
        ));
    }
    let z = origin.z;
    let corner = |x: f64, y: f64| Point3 { x, y, z };
    let mut geometries = Vec::with_capacity(1 + cells.len().saturating_sub(1) + cells.len());
    // The border: one closed rectangle.
    geometries.push(SemanticGeometry::Polyline {
        points: vec![
            corner(origin.x, origin.y),
            corner(origin.x + total_width, origin.y),
            corner(origin.x + total_width, origin.y + row_height),
            corner(origin.x, origin.y + row_height),
        ],
        bulges: vec![0.0; 4],
        closed: true,
        width: 0.0,
    });
    // One internal divider per compartment boundary.
    let mut x = origin.x;
    for width in widths.iter().take(cells.len().saturating_sub(1)) {
        x += *width;
        geometries.push(SemanticGeometry::Line {
            start: corner(x, origin.y),
            end: corner(x, origin.y + row_height),
        });
    }
    // One TEXT per compartment: left aligned, vertically centred in the cell.
    let mut cell_left = origin.x;
    for (cell, width) in cells.iter().zip(&widths) {
        geometries.push(SemanticGeometry::Text {
            text: cell.clone(),
            position: corner(cell_left + padding, origin.y + row_height * 0.5),
            style,
            height,
            rotation: 0.0,
            font: font.clone(),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Middle,
            // The cell is already sized from the declared width estimate; an
            // explicit wrap boundary would only risk wrapping a cell that fits.
            wrap_width: None,
            width_factor: crate::DEFAULT_TEXT_WIDTH_FACTOR,
            line_spacing: crate::TextLineSpacing::default(),
        });
        cell_left += *width;
    }
    Ok(geometries)
}

// ---------------------------------------------------------------------------
// The creation request.
// ---------------------------------------------------------------------------

/// The annotation content a **creation** request asks for.
///
/// [`Self::Text`] keeps the pre-existing rule: an empty string creates a
/// content-less leader, a non-empty one MText content. The other two variants are
/// only reachable from the typed `MLEADER CONTENT …` option, and each needs the
/// identity/value the command layer resolved (a block name cannot be resolved to
/// a [`BlockId`] inside `cad-domain`).
#[derive(Debug, Clone, PartialEq)]
pub enum MLeaderCreateContent {
    /// `CONTENT MTEXT` (the default): the annotation text.
    Text(String),
    /// `CONTENT BLOCK <name>`: the resolved block identity, its stored name and
    /// the definition's base point (the point the placement puts on the content
    /// point).
    Block {
        /// The drawing-local block the display expands.
        block: BlockId,
        /// The block name the DXF record carries.
        name: String,
        /// The definition's insertion base point (`BlockMetadata::base_point`), or
        /// `None` when the drawing has no metadata for it — which the creation
        /// refuses (`mleader.block_metadata_missing`) instead of guessing an
        /// origin.
        base_point: Option<Point3>,
    },
    /// `CONTENT TOLERANCE <text>`: the frame string; the height is the resolved
    /// style's text height.
    Tolerance {
        /// The `|`-separated frame text.
        text: String,
    },
}

impl MLeaderCreateContent {
    /// The content kind this request asks for (before the text-is-empty rule).
    pub fn kind(&self) -> MLeaderContentKind {
        match self {
            Self::Text(_) => MLeaderContentKind::Text,
            Self::Block { .. } => MLeaderContentKind::Block,
            Self::Tolerance { .. } => MLeaderContentKind::Tolerance,
        }
    }
}
