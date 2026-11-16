//! The small command-payload data types.
//!
//! Split out of `command_payload.rs` (2026-11-12) so both files stay inside the
//! 1000-line code gate. `command_payload.rs` re-exports every item, so the
//! public paths are unchanged.

use cad_domain::{MLeaderLine, MLeaderPathType, Point3, SelectionRef};

/// The two straight-line picks of a CHAMFER/FILLET command (ADR 0016 decisions
/// B/C): the two picked refs in pick order and the raw click point that selects
/// each pick side.
///
/// Boxed inside [`crate::CommandPayload::Chamfer`]/`Fillet` so the command enum stays
/// small (the same reason [`crate::CommandPayload::Geometry`] boxes its geometry).
pub struct TwoLineCommand {
    pub first: SelectionRef,
    pub second: SelectionRef,
    pub pick_first: Point3,
    pub pick_second: Point3,
}

/// The input of a DIMROTATED command (AutoCAD 2026): the two extension-line
/// origins, the dimension-line location and the **rotation** of the dimension
/// line in radians (about `+Z`).
///
/// Boxed inside [`crate::CommandPayload::RotatedLinearDimension`] so the command enum
/// stays small. `rotation` is already a radian value: the session parameter
/// [`crate::draw_tool::ToolParameter::DimRotation`] carries **degrees** and the
/// host converts it exactly once when it builds this payload. Any finite angle is
/// legal — a negative angle is a rotation in the other direction, not an error,
/// and no normalisation is applied.
pub struct RotatedLinearDimensionCommand {
    pub first: Point3,
    pub second: Point3,
    pub location: Point3,
    pub rotation: f64,
}

/// The input of a DIMRADIUS/DIMDIAMETER command (ADR 0017 decision C, P5d): the
/// picked arc/circle ref and the dimension-line location.
///
/// Boxed inside [`crate::CommandPayload::DimRadial`] so the command enum stays small (the
/// same reason [`TwoLineCommand`] is boxed). Which of the two commands it is comes
/// from the [`CommandId`], so the payload carries no kind field (a second source
/// of truth).
pub struct RadialDimensionCommand {
    pub target: SelectionRef,
    pub location: Point3,
}

/// The two supported DIMANGULAR constructions (ADR 0017 decision C, P5e).
///
/// * [`Self::TwoLine`] — two picked straight lines plus the dimension-arc
///   location. The vertex is the intersection of the two infinite lines
///   (ADR 0017 U-8, pinned against the committed QCAD fixtures).
/// * [`Self::ThreePoint`] — a vertex, the two side points and the dimension-arc
///   location. The interactive capture supports it too: DIMANGULAR's first step
///   is the mixed `CaptureStep::PickOrPoint` ("select a line **or** specify a
///   vertex"), and the first capture selects the form. A capture of the kind the
///   chosen form does not take is refused by name
///   (`dimangular.capture.mixed`), never reinterpreted. This variant is the
///   command-layer payload for that same interactive form, not a
///   command-only construction.
///
/// Which variant it is is part of the input, not a second command: both produce
/// one `DIMENSION` entity through the same funnel.
#[derive(Debug, Clone, PartialEq)]
pub enum AngularDimensionInput {
    /// Vertex, first side point, second side point, dimension-arc location.
    ThreePoint {
        vertex: Point3,
        first: Point3,
        second: Point3,
        location: Point3,
    },
    /// Two picked straight lines plus the dimension-arc location.
    TwoLine {
        first: SelectionRef,
        second: SelectionRef,
        location: Point3,
    },
}

/// The five modes of the `TCASE` text-case command (AutoCAD's `-TCASE`
/// `Sentence`/`Lower`/`Upper`/`Title`/`toGgle` prompt, 2026-11-07).
///
/// This is the machine form of the typed argument grammar parsed by
/// `cad_ui_core::session::parse_tcase_argument`; the command layer is the only
/// consumer. The **bare** `TCASE` selector carries no payload at all and the
/// command applies [`Self::Title`], the documented current default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCaseMode {
    /// `UPPER` / `U`: every alphabetic character upper-cased.
    Upper,
    /// `LOWER` / `L`: every alphabetic character lower-cased.
    Lower,
    /// `TITLE` / `T` (and the bare default): the first alphabetic character of
    /// each whitespace-separated word upper-cased, the rest lower-cased.
    Title,
    /// `SENTENCE` / `S`: the first alphabetic character of the whole string
    /// upper-cased, the rest lower-cased.
    Sentence,
    /// `TOGGLE` / `G`: every alphabetic character's case swapped.
    Toggle,
}

/// The option of the AutoCAD 2026 `MEASUREGEOM` inquiry router.
///
/// The typed grammar (`cad_ui_core::session::parse_measuregeom_argument`)
/// admits the full words only; this enum is the machine form. Four options route
/// to the existing measurement tools and never reach the command layer with this
/// payload; `Radius` is the read-only report and `Volume` is the explicit
/// not-implemented refusal. Full contract:
/// [`crate::command_id::measuregeom`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureGeomOption {
    /// `MEASUREGEOM DISTANCE` → the `DIST` tool.
    Distance,
    /// `MEASUREGEOM ANGLE` → the `MEASUREANGLE` tool.
    Angle,
    /// `MEASUREGEOM AREA` → the `AREA` tool.
    Area,
    /// `MEASUREGEOM POLYLINE` → the `MEASUREPOLYLINE` tool.
    Polyline,
    /// `MEASUREGEOM RADIUS`: read-only radius/diameter/circumference report of
    /// exactly one selected `ARC`/`CIRCLE`.
    Radius,
    /// `MEASUREGEOM VOLUME`: **not implemented** (no 3D-solid/volume model), a
    /// named `Unsupported`, never a fabricated number.
    Volume,
}

/// The option of `MLEDIT` / `-MLEDIT`.
///
/// The stored MLINE creation path produces **two independent `LINE`s** (the
/// `SEMANTIC` model has no `AcDbMline` vertex/join record of its own), so the
/// two-LINE subset is what this build can edit:
///
/// * the four clean-up spellings (`MERGE`, `CLOSED`, `CROSS`, `TEE`) all run the
///   one representable clean-up: the endpoint of each selected line nearest the
///   intersection is moved onto the intersection (extends a short line, trims
///   the shorter overhang of a crossing one);
/// * `OPEN` splits both lines at the intersection (the two-LINE model has no
///   multiline width, so the gap is zero-width — the declared residual);
/// * `BREAK` removes the band between two picked points from both selected
///   parallel lines and leaves two open segments per line.
///
/// Caps, miters, joins, fill and the `AcDbMline` vertex list are **not**
/// representable and are named residuals, never fabricated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MlEditMode {
    /// `MLEDIT MERGE <x> <y>`: the closed clean-up spelling.
    Merge,
    /// `MLEDIT CLOSED <x> <y>`: the closed clean-up spelling.
    Closed,
    /// `MLEDIT CROSS <x> <y>`: the closed clean-up spelling.
    Cross,
    /// `MLEDIT TEE <x> <y>`: the closed clean-up spelling.
    Tee,
    /// `MLEDIT OPEN <x> <y>`: split both lines at the crossing.
    Open,
    /// `MLEDIT BREAK <x1> <y1> <x2> <y2>`: remove the picked band from both lines.
    Break,
}

impl MlEditMode {
    /// The upper-case token the typed line used (echoed in diagnostics).
    pub fn token(self) -> &'static str {
        match self {
            Self::Merge => "MERGE",
            Self::Closed => "CLOSED",
            Self::Cross => "CROSS",
            Self::Tee => "TEE",
            Self::Open => "OPEN",
            Self::Break => "BREAK",
        }
    }

    /// Whether the mode is one of the four clean-up spellings.
    pub fn is_cleanup(self) -> bool {
        matches!(self, Self::Merge | Self::Closed | Self::Cross | Self::Tee)
    }
}

/// The input of `MLEDIT` / `-MLEDIT`.
///
/// `first` is the crossing pick for the clean-up/`OPEN` forms; `second` is the
/// second break-window corner and is present only for [`MlEditMode::Break`].
#[derive(Debug, Clone, PartialEq)]
pub struct MlEditCommand {
    pub mode: MlEditMode,
    pub first: Point3,
    pub second: Option<Point3>,
}

/// The alignment of `MLEADERALIGN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MLeaderAlignMode {
    /// Line the annotation text up on the minimum X of the selection.
    Left,
    /// Line the annotation text up on the maximum X of the selection.
    Right,
    /// Line the annotation text up on the mid X of the extreme text positions.
    Center,
}

impl MLeaderAlignMode {
    /// The upper-case token the typed line used (echoed in diagnostics).
    pub fn token(self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Center => "CENTER",
        }
    }
}

/// The input of `MLEADERALIGN`.
///
/// The current selection is the object input; `spacing` is an optional vertical
/// pitch (`Some(d)` re-spaces the aligned texts top-to-bottom at `d`, `None`
/// only aligns X and keeps every current Y).
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderAlignCommand {
    pub mode: MLeaderAlignMode,
    pub spacing: Option<f64>,
}

/// The action of `MLEADEREDIT`.
///
/// The four addressable fields of one leader line in this model are its two
/// per-line overrides (path type `LEADER_LINE 170` behind override word `93` bit
/// `1`, arrowhead size `40` behind bit `16`) and its vertices (written by `ADD`,
/// moved by `MLEADERALIGN`). Every action below writes **one** stored definition,
/// rebuilt through `MultiLeaderGeometry::new`, in one transaction / one undo step.
#[derive(Debug, Clone, PartialEq)]
pub enum MLeaderEditAction {
    /// `MLEADEREDIT ADD <x> <y>`: add one leader line to root 0, from the typed
    /// arrow position to the root's existing dogleg vertex.
    Add { arrow: Point3 },
    /// `MLEADEREDIT REMOVE <index>`: remove one leader line by its 0-based index
    /// in the leader's stored line list.
    Remove { line_index: usize },
    /// `MLEADEREDIT PATHTYPE <index> LINE|SPLINE|INVISIBLE`: set one line's own
    /// path type (`LEADER_LINE 170`) and stamp override word bit `1`, so the line
    /// keeps that path even when the entity/style value differs.
    SetPathType {
        line_index: usize,
        path_type: MLeaderPathType,
    },
    /// `MLEADEREDIT ARROWSIZE <index> <size>`: set one line's own arrowhead size
    /// (`LEADER_LINE 40`) and stamp override word bit `16`. The size must be
    /// finite and strictly positive: this build draws no arrowhead for a
    /// non-positive per-line size, and a stored `0` has no unambiguous meaning in
    /// the format, so it is refused instead of guessed.
    SetArrowheadSize { line_index: usize, size: f64 },
    /// `MLEADEREDIT BYSTYLE <index> PATH|ARROW`: clear one line's override bit
    /// (`1` / `16`) and re-stamp the stored value from the resolved style, so the
    /// entity/style value applies again and the field never keeps a stale value
    /// the style could not reach.
    ClearOverride {
        line_index: usize,
        field: MLeaderLineOverrideField,
    },
}

impl MLeaderEditAction {
    /// The upper-case token the typed line used (echoed in diagnostics).
    pub fn token(&self) -> &'static str {
        match self {
            Self::Add { .. } => "ADD",
            Self::Remove { .. } => "REMOVE",
            Self::SetPathType { .. } => "PATHTYPE",
            Self::SetArrowheadSize { .. } => "ARROWSIZE",
            Self::ClearOverride { .. } => "BYSTYLE",
        }
    }
}

/// The one per-line override word bit an `MLEADEREDIT BYSTYLE` clears.
///
/// These are the only two per-line values this model stores on the line itself
/// (`path_type`, `arrowhead_size`) instead of the opaque `reserved` bag, so they
/// are the two the command can write and therefore also the two it can clear. The
/// word's other bits (line colour `2`, linetype `4`, lineweight `8`) describe
/// source data this build only preserves: those live in `reserved` and have no
/// editable semantic model, so the typed grammar refuses them by name rather than
/// clearing a bit whose value it could not write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MLeaderLineOverrideField {
    /// Override word bit `1`: the line's own path type (`LEADER_LINE 170`).
    PathType,
    /// Override word bit `16`: the line's own arrowhead size (`LEADER_LINE 40`).
    ArrowheadSize,
}

impl MLeaderLineOverrideField {
    /// The DXF `93` override word bit this field owns.
    pub fn bit(self) -> u32 {
        match self {
            Self::PathType => MLeaderLine::PATH_TYPE,
            Self::ArrowheadSize => MLeaderLine::ARROWHEAD_SIZE,
        }
    }

    /// The upper-case token the typed line used (echoed in diagnostics).
    pub fn token(self) -> &'static str {
        match self {
            Self::PathType => "PATH",
            Self::ArrowheadSize => "ARROW",
        }
    }
}

/// The input of `MLEADEREDIT`.
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderEditCommand {
    pub action: MLeaderEditAction,
}

/// The annotation text attachment pair a created multileader stores.
///
/// Both codes are the stored ones — `point` is the horizontal side (entity `179`
/// / `CONTEXT_DATA 171`: `1` left, `2` centre, `3` right) and `left` the vertical
/// attachment (entity `173` / `CONTEXT_DATA 174`: `0`..`10`). They travel together
/// because a `MULTILEADER` carries exactly one pair, never a per-side one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MLeaderAttachment {
    pub point: i16,
    pub left: i16,
}

/// The annotation **content kind** a typed `MLEADER CONTENT …` option asks for.
///
/// The stored payload stays minimal on purpose: the kind is the *choice*, while
/// every value (the block name, the frame text) comes from a field the payload
/// already has —
///
/// * [`Self::MText`] is the default and the pre-existing behaviour: the `text`
///   field is the annotation, and an empty one creates a content-less leader;
/// * [`Self::Block`] names the block to insert; the leader's `text` field must be
///   empty (a block-content leader has no annotation string) and the command
///   layer resolves the name against the drawing's block table, refusing an
///   unknown one by name (`mleader.block_missing`);
/// * [`Self::Tolerance`] turns the `text` field into the `|`-separated
///   feature-control frame string (the `TOLERANCE` command's own grammar); the
///   cell height is the resolved `MLEADERSTYLE` text height.
///
/// The two codes DXF stores are entity `172` (`0` none, `1` block, `2` MText,
/// `3` tolerance) and the style default `170`; the command writes the entity code
/// and sets the `0x400` override bit only when the kind differs from the style
/// row's, exactly like the pre-existing text/content-less choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MLeaderContentSpec {
    /// `CONTENT MTEXT` (the default): the `text` field is MText content.
    MText,
    /// `CONTENT BLOCK <name>`: the annotation is an inserted block, resolved by
    /// name against the drawing's block table.
    Block(String),
    /// `CONTENT TOLERANCE`: the `text` field is the feature-control frame string.
    Tolerance,
}

impl MLeaderContentSpec {
    /// The upper-case token the typed line used (echoed in diagnostics).
    pub fn token(&self) -> &'static str {
        match self {
            Self::MText => "MTEXT",
            Self::Block(_) => "BLOCK",
            Self::Tolerance => "TOLERANCE",
        }
    }
}

/// The input of `TOLERANCE`: the feature-control-frame insertion point (the
/// frame's lower-left corner) and the typed frame text.
///
/// The text is split on `|` into compartments; the height is the session
/// `textHeight` typed channel, so it is not a payload field.
#[derive(Debug, Clone, PartialEq)]
pub struct ToleranceCommand {
    pub origin: Point3,
    pub text: String,
}

/// The input of a `SHAPE` command: one glyph of a compiled SHX shape font.
///
/// `code` is the shape number the font's glyph table is keyed by. `code == 0`
/// means "referenced by name": the entity stores `name` and the representation
/// layer reports the honest partial it already reports for an imported
/// name-only `SHAPE` (this build's SHX parser keeps shape numbers, not shape
/// names, so a name cannot be resolved to a glyph here).
///
/// `font` is the resolved SHX shape font key. The typed form fills it from the
/// drawing's `Standard` text style; the command refuses `shape.font_missing`
/// when neither an explicit key nor a style font exists, instead of inventing a
/// typeface.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeCommand {
    /// The shape name as typed (empty when the token was a bare number).
    pub name: String,
    /// The shape code; `0` means the name form.
    pub code: u32,
    pub position: Point3,
    /// Glyph size in world units (finite, strictly positive).
    pub size: f64,
    /// Glyph rotation about `+Z`, in radians.
    pub rotation: f64,
    pub font: Option<String>,
}

/// The input of an `MINSERT` command: a rectangular block array.
///
/// The typed form is `MINSERT <block> <rows> <columns> <row_spacing>
/// <column_spacing> <x> <y>`. The grid rule is exactly ARRAYRECT's declared rule
/// (the cell offset *is* `column · column_spacing` / `row · row_spacing`, so a
/// negative spacing grows down/left and the source cell `(0, 0)` is the block's
/// insertion point), and every cell is one real `SemanticGeometry::Insert` with
/// the importer's `T(insertion) · T(−base_point)` transform.
#[derive(Debug, Clone, PartialEq)]
pub struct MInsertCommand {
    /// The existing block definition's name.
    pub block: String,
    /// Row count (at least one row).
    pub rows: u32,
    /// Column count (at least one column).
    pub columns: u32,
    /// Signed distance between adjacent rows' base points.
    pub row_spacing: f64,
    /// Signed distance between adjacent columns' base points.
    pub column_spacing: f64,
    /// The `(0, 0)` cell's insertion point.
    pub insertion: Point3,
}

/// The input of an `ADDSELECTED <x> <y>` command.
///
/// `position` is the new entity's placement point: the LINE start point, the
/// CIRCLE centre, the POINT position or the TEXT insertion point. The command
/// reads the one selected entity for the type and general properties, so the
/// selection is not part of the payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AddSelectedCommand {
    pub position: Point3,
}
