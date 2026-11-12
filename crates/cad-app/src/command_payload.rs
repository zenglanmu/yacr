//! The typed input payloads of the command funnel.
//!
//! Split out of the crate root to keep `lib.rs` inside the code-size limit. Every
//! item is re-exported by [`crate`] under its original name, so the public paths
//! (`cad_app::CommandPayload`, `cad_app::TwoLineCommand`, …) are unchanged.
//!
//! A payload describes the **input** of exactly one [`crate::CommandId`]; it never
//! carries behaviour. The command layer validates it and either performs the
//! documented write in one transaction or refuses explicitly (see each variant's
//! documentation).

use cad_domain::{
    LayerId, Point3, SelectionRef, SemanticGeometry, SpaceId, TextAlignH, TextAlignV,
};

use crate::{AppMode, BackendChoice, MeasurementToolKind, SelectionMode, StandardView};

mod text_batch;
pub use text_batch::*;

mod payload_extra;
pub use payload_extra::*;

mod basic;
pub use basic::*;

mod style_tables;
pub use style_tables::*;

/// The direction of one `REFSET ADD|REMOVE` working-set edit.
///
/// Session-only: `REFSET` never writes the drawing, it only records which
/// extracted entities (and which extra host objects) the in-place block edit
/// owns. [`crate::CommandId::RefSet`] with a [`crate::CommandPayload::None`]
/// means `Add` (AutoCAD's bare prompt default); the typed `REFSET REMOVE` line
/// reaches this enum through the argument channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefSetOption {
    /// Add the current selection to the working set.
    Add,
    /// Remove the current selection from the working set.
    Remove,
}

/// The disposition of an active in-place block edit (`REFCLOSE SAVE|DISCARD`).
///
/// `Save` writes the working set back into the block definition; `Discard`
/// deletes the extracted model-space clones and leaves the definition untouched.
/// A bare `REFCLOSE` carries no option and is refused by name
/// (`refclose.needs_option`), never defaulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefCloseOption {
    /// Write the working set back into the block definition.
    Save,
    /// Delete the clones and leave the block definition unchanged.
    Discard,
}

pub enum CommandPayload {
    None,
    Points(Vec<Point3>),
    Layer(LayerId, bool),
    /// Distinct existing layer ids and their temporary visibility values.
    /// Empty batches and duplicate ids (including equal values) are invalid.
    LayerVisibilities(Vec<(LayerId, bool)>),
    Space(SpaceId),
    StandardView(StandardView),
    Backend(BackendChoice),
    /// Start (or restart) a measurement tool with an explicit algorithm.
    MeasureTool(MeasurementToolKind),
    /// Orbit the current view: yaw about world +Z, pitch about the view right
    /// axis, both in radians. Preserves the eye-target distance (audit F13).
    Orbit {
        yaw: f64,
        pitch: f64,
    },
    /// Structured zoom: a positive factor plus the logical-pixel cursor that must
    /// stay anchored (distinct from the legacy `Points`-payload zoom, which has none).
    ZoomAt {
        factor: f64,
        cursor: [f64; 2],
    },
    /// `ZOOM WINDOW`: frame the picked two-corner rectangle **exactly** (margin
    /// `1.0`, unlike `FIT`'s `1.05`).
    ///
    /// `rect_logical` is the two opposite corners in **canvas logical pixels**,
    /// the same convention as [`CommandPayload::ZoomAt`] and
    /// [`CommandPayload::BoxSelect`]; the corners may arrive in any order and are
    /// canonicalised to `[min, max]` by the command (one rule, every producer).
    /// A non-finite corner is `zoom.window.invalid_rect` and a rectangle with no
    /// extent on either axis is `zoom.window.degenerate_rect` — both are
    /// `InvalidInput` before any camera change, never a clamped fit.
    ///
    /// **2D orthographic only**: a 3D observation refuses by name
    /// (`zoom.window.requires_2d`); view-only, no write/revision/history/generation.
    ZoomWindow {
        rect_logical: [[f64; 2]; 2],
    },
    /// `ZOOM PREVIOUS`: restore this viewport's last **discrete** view.
    ///
    /// Pops the per-viewport bounded view history (`cad_app::zoom_history`, cap
    /// 16) and restores the camera, the 2D/3D mode and the plan work plane
    /// exactly. Pans, wheel zooms and orbit drags never record a step — an
    /// explicit divergence from AutoCAD, documented in that module.
    ///
    /// An empty history is **not an error**: the command changes nothing and
    /// reports the `zoom.previous.empty` diagnostic. View-only; no database
    /// write, no history entry, no generation bump.
    ZoomPrevious,
    ZoomObject,
    /// Target mode for [`CommandId::SetMode`] (audit U02).
    Mode(AppMode),
    /// The session grid-snap spacing for [`CommandId::SetSnapSpacing`] (the typed
    /// `SNAPUNIT <value>` line). Must be finite and strictly positive; the
    /// session setter validates it and refuses anything else explicitly.
    SnapSpacing(f64),
    /// The raw `$PDMODE` value for [`CommandId::SetPointDisplayMode`] (the typed
    /// `PDMODE <n>` line), stored verbatim in the drawing header. The display
    /// layer interprets the base glyph and the `32`/`64`/`96` shape bits.
    PointDisplayMode(i16),
    /// The raw `$PDSIZE` value for [`CommandId::SetPointDisplaySize`] (the typed
    /// `PDSIZE <n>` line), stored verbatim. Must be finite; the sign and the
    /// magnitude are the display layer's contract (`> 0` absolute drawing units,
    /// `0` viewport-relative, `< 0` a percentage of the viewport height).
    PointDisplaySize(f64),
    /// Replace the current selection with these refs (read-only; never writes
    /// the DWG). An empty list clears the selection.
    Selection(Vec<SelectionRef>),
    /// Select every visible model-space item whose geometry satisfies a
    /// window/crossing rectangle (ADR 0014 B).
    ///
    /// `rect_logical` is two opposite corners in **canvas logical pixels**, the
    /// same convention as [`CommandPayload::ZoomAt`]; the command maps them to
    /// the active viewport's work plane. `crossing == false` is a WINDOW
    /// selection (fully inside only); `crossing == true` is a CROSSING
    /// selection (intersect or contain), i.e. the right-to-left drag semantic.
    /// `mode` is the merge mode applied to the current selection. Read-only: no
    /// ChangeSet, no revision change, no history entry.
    BoxSelect {
        rect_logical: [[f64; 2]; 2],
        crossing: bool,
        mode: SelectionMode,
    },
    /// Translate a set of selected refs by a world delta (MOVE, spec F-EDIT).
    Move {
        refs: Vec<SelectionRef>,
        delta: Point3,
    },
    /// Clone a set of selected refs with fresh ids, translating each copy by a
    /// world delta (COPY, spec F-EDIT). Same interaction shape as `Move`.
    Copy {
        refs: Vec<SelectionRef>,
        delta: Point3,
    },
    /// Rotate a set of selected refs about `base` by `angle` radians in the XY
    /// plane (ROTATE, spec F-EDIT).
    Rotate {
        refs: Vec<SelectionRef>,
        base: Point3,
        angle: f64,
    },
    /// Scale a set of selected refs uniformly about `base` by `factor` (SCALE,
    /// spec F-EDIT). `factor` must be positive and finite.
    Scale {
        refs: Vec<SelectionRef>,
        base: Point3,
        factor: f64,
    },
    /// Mirror a set of selected refs across the line through `axis_start` and
    /// `axis_end`, keeping the originals and adding mirrored copies (MIRROR,
    /// spec F-EDIT). Same interaction shape as `Copy`. `Arc`/`Ellipse` (including
    /// elliptical arcs) are reflected explicitly — the plane normal is negated and
    /// the arc's start angle is re-derived from its mirrored start point
    /// (`app_drawing::mirror_curves`); kinds with no honest reflection (`Text`,
    /// `Shape`, `Opaque`, `MultiLeader`) still refuse the whole command.
    Mirror {
        refs: Vec<SelectionRef>,
        axis_start: Point3,
        axis_end: Point3,
    },
    /// The full MIRROR option set: `MIRRTEXT` and `Erase source objects`
    /// (`crate::app_drawing::drawing_edit::mirror_entities_with_options`).
    ///
    /// The typed `MIRROR TEXT 0|1` / `MIRROR ERASE YES|NO` lines store these
    /// session defaults, and a capture that runs with a non-default set travels
    /// through this variant instead of the legacy [`Self::Mirror`] payload. A
    /// bare `Mirror` payload keeps the documented defaults (readable text, no
    /// erase) and delegates to the same option entry point.
    MirrorOptions(Box<MirrorOptions>),
    /// Offset a single selected LINE, CIRCLE, ARC or LWPOLYLINE (straight or
    /// bulged) by the distance and side defined by `side_point` (OFFSET, spec
    /// F-EDIT). A polyline is offset with miter joins and its vertex
    /// count/order preserved (see `app_drawing::offset_polyline`); an arc
    /// segment offsets concentrically with its centre unchanged (see
    /// `app_drawing::offset_polyline_bulge`). Exactly one source entity is
    /// required; any other geometry kind is an explicit refusal.
    Offset {
        refs: Vec<SelectionRef>,
        side_point: Point3,
    },
    /// The full OFFSET option set: distance / `Through`, `Erase`, `Layer`,
    /// `Multiple` (one side/through point per repetition) and the
    /// `OFFSETGAPTYPE` join (see
    /// [`crate::app_drawing::drawing_edit::offset_entities_with_options`]).
    ///
    /// The typed `OFFSET <distance>` / `OFFSET THROUGH` / `OFFSET ERASE` /
    /// `OFFSET LAYER ...` lines store these session defaults, and a capture that
    /// runs with a non-default set travels through this variant instead of the
    /// legacy [`Self::Offset`] payload. A bare `Offset` payload keeps the legacy
    /// `Through` semantics and delegates to the same option entry point.
    OffsetOptions(Box<OffsetOptions>),
    /// Trim one target against a list of boundary refs, keeping the side that
    /// contains `pick_point` (TRIM, spec F-EDIT). See `docs/drawing-edit.md` §3
    /// for the supported subset.
    Trim {
        target: SelectionRef,
        boundary: Vec<SelectionRef>,
        pick_point: Point3,
    },
    /// Extend one target LINE to the nearest boundary segment beyond the picked
    /// end (EXTEND, spec F-EDIT). Same interaction shape and supported subset as
    /// [`CommandPayload::Trim`], but the target grows instead of shrinking.
    Extend {
        target: SelectionRef,
        boundary: Vec<SelectionRef>,
        pick_point: Point3,
    },
    /// `BREAK` one LINE, ARC, CIRCLE or SPLINE between two picked points, keeping
    /// the complementary piece(s) (Work-only).
    ///
    /// `target` is the entity pick resolved at capture time, and also the raw
    /// click that is the **first** break point (`first`); `second` is the second
    /// break point. Both picks are projected onto the target's own parameter space
    /// by the handler; there is no `F` first-point override and no multiple-break
    /// mode. The source is retained as the first piece and at most one new entity
    /// (the second piece, or a CIRCLE's single ARC complement) is inserted.
    Break {
        target: SelectionRef,
        first: Point3,
        second: Point3,
    },
    /// `BREAKATPOINT`: split the picked object at one point.
    ///
    /// `target` is the ref resolved at pick time and `point` is the second click.
    /// The point is **projected** onto the target (AutoCAD's own behaviour for a
    /// click that is not exactly on the curve). LINE, ARC and an open LWPOLYLINE
    /// are supported; a CIRCLE/closed polyline, a SPLINE and every other kind are
    /// named refusals. The source stays the first piece and the second piece is a
    /// fresh id — one transaction / one undo step. Full contract:
    /// [`crate::command_id::break_at_point`].
    BreakAtPoint {
        target: SelectionRef,
        point: Point3,
    },
    /// `DIVIDE` the one selected curve into equal arc-length parts, placing POINT
    /// marks at the division parameters (Work-only).
    ///
    /// The selection snapshot is the whole input (the capture is zero-point), and
    /// the segment count is the session typed channel
    /// ([`crate::draw_tool::ToolParameter::DivideSegments`]), read by the handler.
    /// The source curve is retained; the marks are new entities on the active
    /// layer in one transaction / one undo step.
    Divide {
        refs: Vec<SelectionRef>,
    },
    /// `LENGTHEN` the one selected curve (Work-only).
    ///
    /// The selection snapshot names the target; the static DE/PER/TOT modes also
    /// accept a one-point `Points([pick])` whose pick chooses the end that moves
    /// (the legacy no-pick payload moves the far end). The mode and amount are
    /// the session typed channel ([`crate::draw_tool::ToolParameter::LengthenMode`] /
    /// [`crate::draw_tool::ToolParameter::LengthenValue`]), read by the handler.
    /// The same entity id is updated in one transaction / one undo step.
    Lengthen {
        refs: Vec<SelectionRef>,
    },
    /// `SCALETEXT` the selected Text/MText entities' glyph height (Work-only).
    ///
    /// The selection snapshot is the whole input (the capture is zero-point); the
    /// factor is the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ScaleTextFactor`]), read by the
    /// handler. Every ref must resolve to a directly addressable model-space
    /// `Text`/`MText`; a mixed or unsupported selection refuses the whole command
    /// before any write, and every changed entity is staged in one transaction /
    /// one undo step.
    ScaleText {
        refs: Vec<SelectionRef>,
    },
    /// `TORIENT` (Work-only): reset the rotation of the selected `Text`/`MText`
    /// entities (the **selection** is the whole input, `CommandPayload::None`
    /// for the bare most-readable form).
    ///
    /// `angle_degrees` is the typed `TORIENT <angle-degrees>` form: the absolute
    /// rotation in degrees (finite), stored in radians. `None` means AutoCAD's
    /// bare command: normalise each rotation into `(-π/2, π/2]` by 180° steps.
    /// Only `rotation` changes; position, height, alignment, style, font, layer,
    /// space and draw order are preserved. Non-text selected entities are
    /// reported as `torient.not_reset` diagnostics and never fail the command.
    Torient {
        angle_degrees: Option<f64>,
    },
    TorientReverse, // TORIENT Reverse: add 180°; typed as TORIENT REVERSE|R.
    /// `TCASE` (Work-only): change the case of the selected text.
    ///
    /// The selection is read from the session at apply time (bare `TCASE` uses
    /// `CommandPayload::None` and applies [`TextCaseMode::Title`]); the typed
    /// argument form carries its mode here. Only visible characters change —
    /// MTEXT formatting escapes (`\P`, `\f…;`, `\H…;`, `\C…;`, `{}`, `\~`,
    /// `\\`) are copied verbatim. Non-text selected entities are reported as
    /// `tcase.not_reset` diagnostics; a MULTILEADER's text is rewritten through
    /// [`cad_domain::MultiLeaderGeometry::with_text`].
    TCase {
        mode: TextCaseMode,
    },
    /// `PEDIT` (Work-only): one declared v1 polyline option.
    ///
    /// The current selection is the input: exactly one directly addressable
    /// model-space LWPOLYLINE. A bare `PEDIT` (`CommandPayload::None`) is refused
    /// with `pedit.needs_option`, because this build has no in-command option
    /// prompt. Full contract: [`crate::command_id::pedit`].
    PeditOption(PeditOption),
    /// `PEDIT` (Work-only): the full typed option line (`JOIN`, `WIDTH`, `FIT`,
    /// `SPLINE <0-99>`, `DECURVE`, `LTYPE GEN`, `UNDO`, …) plus the
    /// `PEDITACCEPT` conversion answer. Full contract:
    /// [`crate::command_id::pedit`].
    PeditEdit(PeditEditCommand),
    /// `JOIN` (Work-only): the typed `Close`/`MULTIPLE` sub-options. A bare
    /// `JOIN` keeps `CommandPayload::None` and means the source-object mode with
    /// no close. Full contract: [`crate::CommandId::Join`].
    Join(JoinOptions),
    /// `HATCHEDIT` (Work-only): one typed option applied to the one selected
    /// model-space hatch.
    ///
    /// The current selection is the input (exactly one directly addressable
    /// model-space `AcDbHatch`). A bare `HATCHEDIT` (`CommandPayload::None`) is
    /// refused with `hatchedit.needs_option`, because this build has no
    /// in-command Hatch Edit dialog. Full contract:
    /// [`crate::command_id::hatchedit`].
    HatchEdit(HatchEditOption),
    /// `BLOCK` (Work-only): define a named block from `refs` at the picked
    /// `base` point.
    ///
    /// `refs` is the selection snapshot taken when
    /// [`crate::draw_tool::DrawToolKind::BlockDefine`] started. Every ref must be
    /// a directly addressable model-space entity in this document; a member
    /// keeps its geometry **bit-for-bit** and only moves into
    /// [`cad_domain::SpaceId::Block`]`(new_id)` — the command never translates
    /// member geometry. The stored metadata is `BlockMetadata { name,
    /// base_point: base }`, and the new reference uses the same
    /// `T(base) · T(−base)` composition as `INSERT`, i.e. the identity.
    ///
    /// `name` is trimmed and validated by the command (non-empty, single line,
    /// at most [`crate::draw_tool::MAX_TOOL_TEXT_CHARS`] characters, no
    /// AutoCAD-illegal character, no exact/case-insensitive duplicate). The
    /// definition row, member space changes and reference insert commit in one
    /// transaction / one undo step. See [`crate::app_drawing::block_define`].
    DefineBlock {
        refs: Vec<SelectionRef>,
        base: Point3,
        name: String,
    },
    /// CHAMFER two straight lines (ADR 0016 decision C). The two picks and their
    /// raw click points travel in one boxed [`TwoLineCommand`] (boxed like
    /// [`CommandPayload::Geometry`] so the command enum stays small); the two
    /// distances are the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ChamferDistanceA`]/`B`), read by the
    /// handler so the session is the single authority for them.
    Chamfer(Box<TwoLineCommand>),
    /// FILLET two straight lines (ADR 0016 decision B). Same shape as
    /// [`CommandPayload::Chamfer`]; the radius is the session typed channel
    /// ([`crate::draw_tool::ToolParameter::FilletRadius`]).
    Fillet(Box<TwoLineCommand>),
    /// ARRAYRECT the selected refs on a rectangular grid (ADR 0016 decision D).
    ///
    /// The row/column counts and spacings are the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ArrayRows`] … `ArrayColumnSpacing`),
    /// read by the handler so the session stays the single authority for them.
    /// The source refs are retained; every copy is a fresh entity in one
    /// transaction. Associativity is NOT IMPLEMENTED.
    ArrayRect {
        refs: Vec<SelectionRef>,
    },
    /// ARRAYPOLAR the selected refs around `center` (ADR 0016 decision D). `center`
    /// is the captured array centre; the item count, fill angle and rotate-items flag
    /// are the session typed channel (`ArrayItems` … `ArrayRotateItems`). The source
    /// refs are retained in one transaction; associativity is NOT IMPLEMENTED.
    ArrayPolar {
        refs: Vec<SelectionRef>,
        center: Point3,
    },
    /// ARRAYPATH: 2D path array v1 (first `refs` entry = path, rest = items;
    /// `count` **copies**, optional `align`). `refs` is the selection snapshot
    /// (the typed path fills it at argument-dispatch time, exactly like
    /// [`Self::ArrayRect`]). `count` is the copies inserted per item; the N source
    /// objects stay, so the array adds `count × items` fresh entities. Position 0
    /// can coincide with a retained source whose anchor equals the path start.
    ArrayPath {
        refs: Vec<SelectionRef>,
        count: usize,
        align: bool,
    },
    /// ALIGN the selected refs from two point pairs (ADR 0016 decision F).
    ///
    /// `refs` is the selection snapshot; the four points are the captured
    /// `source1`/`dest1`/`source2`/`dest2` (AutoCAD's capture order). The
    /// `AlignScale` flag is the session typed channel
    /// ([`crate::draw_tool::ToolParameter::AlignScale`]), read by the handler so
    /// the session stays the single authority for it. The composed transform is
    /// applied **in place** (no copies) in one transaction / one undo step.
    Align {
        refs: Vec<SelectionRef>,
        source1: Point3,
        dest1: Point3,
        source2: Point3,
        dest2: Point3,
    },
    /// STRETCH the entities inside a crossing window (ADR 0016 decision E).
    ///
    /// `rect_logical` is the two opposite corners of the **crossing window** in
    /// **canvas logical pixels** — the same convention as
    /// [`CommandPayload::BoxSelect`] and [`CommandPayload::ZoomAt`]; the command
    /// maps them through the active viewport's orthographic work plane and
    /// recomputes the enclosure classification from the rectangle and the
    /// geometry (it is **never** stored in the selection: single source of
    /// truth). The window is **always treated as CROSSING** (AutoCAD requires a
    /// crossing window; cpolygon/lasso are NOT IMPLEMENTED).
    ///
    /// `base` is the captured base point and `delta` the captured displacement
    /// point, both world coordinates; the effective translation is
    /// `delta − base`. A zero displacement (`delta == base`) is refused rather
    /// than a silent no-op.
    Stretch {
        rect_logical: [[f64; 2]; 2],
        base: Point3,
        delta: Point3,
    },
    /// STRETCH inside a **crossing polygon** (or a free-form lasso, which is the
    /// same containment rule over the same vertex list); see
    /// [`StretchPolygonCommand`].
    StretchPolygon(Box<StretchPolygonCommand>),
    /// Set the session's active layer for new entities. Session state only.
    ActiveLayer(LayerId),
    /// `CREATELAYER`: the new layer's name.
    ///
    /// The name is **not** normalized here: the database trims it and then
    /// refuses an empty/whitespace-only name, an AutoCAD-illegal character and a
    /// case-insensitive duplicate. A blank line therefore fails explicitly instead
    /// of creating a nameless layer.
    CreateLayer(String),
    /// `RENAMELAYER`: the existing layer (addressed by name, exactly like the
    /// layer panel: exact match wins, then a **unique** case-insensitive one) and
    /// its new name.
    RenameLayer {
        layer: String,
        name: String,
    },
    /// `DELETELAYER`: the layer to remove, addressed by name like
    /// [`CommandPayload::RenameLayer`].
    DeleteLayer {
        layer: String,
    },
    /// `SETLAYERPROPERTY`: write **one** attribute of **one** layer table row
    /// (`off`, `frozen`, `locked`, `plottable`, `color`, `linetype`,
    /// `lineweight`, `transparency`).
    ///
    /// `layer` addresses the row by **name** (exact match first, then a unique
    /// case-insensitive one; unknown and ambiguous names are distinct refusals).
    /// `key` is one of [`crate::layer_property::SUPPORTED_KEYS`] and `value`
    /// follows that key's grammar (units fixed there: `lineweight` is **1/100
    /// mm**, `transparency` is **percent transparent**). `plotstyle` is a known
    /// key that is **refused as not implemented**; `name` / `visible` are refused
    /// because they belong to `RENAMELAYER` and to the derived
    /// `visible == !(off || frozen)` rule.
    ///
    /// The value is parsed and applied to a **candidate** attribute set before
    /// anything is published, so a refusal changes nothing and a real write
    /// records exactly one layer-table undo step. `off` / `frozen` additionally
    /// recompute `Layer::visible` in the same mutation; every other key is stored.
    SetLayerProperty {
        layer: String,
        key: String,
        value: String,
    },
    /// The typed `-LAYER <option> …` write line (AutoCAD's command-line layer
    /// editor). See [`DashLayerOps`]; it rides this Work-only carrier because
    /// `ExtendedCommandId::DashLayer` is the read-only report.
    DashLayer(DashLayerOps),
    /// DIMRADIUS/DIMDIAMETER: the picked arc/circle (`target`) plus the
    /// dimension-line location (`location`, ADR 0017 decision C, P5d). The kind
    /// (radius vs diameter) comes from the [`CommandId`]; the handler resolves the
    /// entity, builds the definition through the shared synthesis and inserts one
    /// `AcDbDimension`.
    DimRadial(Box<RadialDimensionCommand>),
    /// DIMCENTER: the picked arc/circle (`target`) for the non-associative centre
    /// mark/centrelines (ADR 0017 decision C/D.7, P5d). The handler resolves the
    /// entity, reads the resolved `DIMCEN` and inserts ordinary LINE geometry
    /// (never a `DIMENSION`); `DIMCEN = 0` inserts nothing and reports it.
    DimCenter(SelectionRef),
    /// DIMANGULAR: the two-line construction (interactive) or the three-point
    /// construction (command-level), ADR 0017 decision C, P5e. The handler builds
    /// an `AcDbDimension` through the shared synthesis and inserts it on the
    /// active layer in one transaction / one undo step.
    DimAngular(Box<AngularDimensionInput>),
    /// DIMORDINATE: the feature point, leader endpoint and optional axis override
    /// (ADR 0017 decision C, P5e). The measurement is the feature's datum-relative
    /// (world-origin-relative) X or Y coordinate magnitude, matching P5b's
    /// ordinate semantics and the acadrust writer's group 42.
    DimOrdinate(Box<OrdinateDimensionCommand>),
    /// DIMBASELINE/DIMCONTINUE: the picked linear base dimension plus the N
    /// picked points, in pick order (ADR 0017 decision C, P5f). The handler
    /// resolves the base, builds one `AcDbDimension` per point through the shared
    /// synthesis and inserts them all in one transaction / one undo step on the
    /// active layer. The kind (baseline stacking vs continuous chain) comes from
    /// the [`CommandId`].
    DimChain(Box<DimensionChainCommand>),
    /// DIMROTATED: the two extension-line origins, the dimension-line location
    /// and the dimension line's rotation in **radians** (AutoCAD 2026). The
    /// handler validates the points, builds one `DimensionKind::Linear { rotation }`
    /// through the shared synthesis and inserts it on the active layer in one
    /// transaction / one undo step. Any finite rotation is accepted (a negative
    /// angle is legal and is not normalised).
    RotatedLinearDimension(Box<RotatedLinearDimensionCommand>),
    DimSpace(Option<f64>), // None = Auto (2 x style height); Some(0) = align; Some(d>0) = spacing.
    /// HATCH the pre-selected boundary entities (ADR 0018 decision D, P6b).
    ///
    /// `refs` is the selection snapshot the tool started with (the tool captures
    /// no points; each selected entity becomes one boundary loop). The pattern
    /// type (solid / user-defined / predefined), the predefined pattern index,
    /// spacing, angle, double flag and style are the session typed channel
    /// ([`crate::draw_tool::ToolParameter::HatchPatternType`] … `HatchStyle`), read
    /// by the handler so the session stays the single authority for them.
    Hatch {
        refs: Vec<SelectionRef>,
    },
    /// HATCHPOINT: create a hatch from **one internal point** (project-specific
    /// selector).
    ///
    /// `seed` is the picked point (finite; model space only in v1). The command
    /// discovers the smallest **whole visible** closed boundary containing it,
    /// includes every nested closed boundary as an island, and fills with the
    /// same session typed parameters HATCH reads
    /// ([`crate::draw_tool::ToolParameter::HatchPatternType`] … `HatchStyle`).
    ///
    /// Declared v1 limits: `LINE`/`ARC` edge chains are **not** auto-discovered
    /// and the current selection is **not** used — only whole visible `Circle`,
    /// full `Ellipse` and closed `Polyline` entities are candidates. Only
    /// boundaries in the seed's `+Z` plane (same elevation) are considered.
    HatchPoint {
        seed: Point3,
    },
    /// BOUNDARY: create one closed polyline per discovered boundary loop from
    /// **one internal point** (project-specific selector, like HATCHPOINT).
    ///
    /// `seed` is the picked internal point (finite; model space only in v1). The
    /// command discovers the smallest visible closed boundary containing it plus
    /// every nested closed boundary, and creates one **non-associative** closed
    /// `AcDbPolyline` per loop on the active layer, in one transaction / one undo
    /// step. The source boundary entities are retained. The current selection is
    /// **not** the input.
    ///
    /// Source-entity limits are HATCHPOINT's: only whole visible `Circle`, full
    /// `Ellipse` and closed `Polyline` entities participate; `LINE`/`ARC` edge
    /// chains, block-instance edges and off-plane geometry are skipped. `-BOUNDARY`
    /// (boundary set, island detection off, region output, object type) is **NOT
    /// IMPLEMENTED** and has no payload field. Full contract:
    /// [`crate::command_id::boundary`].
    BoundaryPoint {
        seed: Point3,
    },
    /// `COPYBASE`: capture the refs into the in-process clipboard together with
    /// the picked base point (a read-only capture: no transaction, no history).
    ///
    /// `refs` is the selection snapshot taken when the tool started (the
    /// [`crate::draw_tool::DrawToolKind::CopyBase`] capture is the one-point
    /// sibling of OFFSET: selection at start, then one free point). `base` is that
    /// captured point, stored as the clipboard's base so
    /// [`CommandPayload::PasteClip`] can translate by `insertion − base`.
    /// Duplicate refs are collapsed; an empty list is an explicit refusal. The
    /// capture is read-only, so the same payload is legal in Viewer mode.
    CopyBase {
        refs: Vec<SelectionRef>,
        base: Point3,
    },
    /// `CUTBASE`: the [`CommandPayload::CopyBase`] capture **plus** the delete of
    /// exactly these refs (never the session selection) in one transaction and one
    /// undo step.
    ///
    /// The clipboard (entries and base) is committed only after the delete
    /// succeeded, so a refused cut leaves the previous clipboard intact. A ref
    /// that does not resolve refuses the whole command **before** anything is
    /// written.
    CutBase {
        refs: Vec<SelectionRef>,
        base: Point3,
    },
    /// `PASTECLIP`: insert fresh-id clones of the clipboard entities translated by
    /// `insertion − base`, where `base` is
    /// [`crate::SessionState`]'s recorded clipboard base and the world origin when
    /// the clipboard has none (`COPYCLIP`/`CUTCLIP`).
    ///
    /// `insertion` is the captured insertion point (finite; one transaction / one
    /// undo step). A non-finite point, an empty clipboard, or a clipboard entry
    /// whose geometry cannot be translated refuses the whole paste before a write —
    /// never a partial insert.
    PasteClip {
        insertion: Point3,
    },
    /// `PASTEBLOCK`: turn the in-process clipboard into a new **auto-named block
    /// definition** and insert one reference at `insertion` (one transaction /
    /// one undo step).
    ///
    /// `insertion` is the captured insertion point (finite). Every clipboard
    /// entry must be a model-space entity; a nested `INSERT` entry is allowed when
    /// its referenced block still exists in the drawing. The new block is named
    /// `PasteBlockN` (first free suffix), member geometry is copied bit-for-bit
    /// into [`SpaceId::Block`]`(new_id)`, and the reference transform is
    /// `T(insertion) · T(−clipboard base)` — the world origin when the clipboard
    /// has no base, exactly like [`CommandPayload::PasteClip`]. The clipboard is
    /// not cleared. Work-only.
    PasteBlock {
        insertion: Point3,
    },
    /// A fully formed semantic geometry, used by `CreateLine` as the
    /// programmatic alternative to a point list.
    Geometry(Box<SemanticGeometry>),
    /// `XLINE`: an infinite construction line reduced to the finite segment
    /// clipped to the drawing's XY bounds; `type_key` stays `AcDbXline` but export
    /// writes a LINE. Full contract: [`crate::command_id::construction_line`].
    XLine {
        base: Point3,
        through: Point3,
    },
    /// `RAY`: the **positive half** of an infinite construction line from `base`
    /// through `through`, stored as the finite segment clipped to the drawing's
    /// XY bounds (or the default 1000-unit box when the drawing is empty).
    ///
    /// Same loss and same refusals as [`CommandPayload::XLine`], with the RAY
    /// names (`ray.invalid_point`, `ray.degenerate_direction`,
    /// `ray.outside_bounds`); the entity's `type_key` stays `AcDbRay` and export
    /// writes a LINE. A RAY whose `+direction` never enters the clip box is
    /// refused rather than drawn as an empty or negative-half line. Work-only.
    Ray {
        base: Point3,
        through: Point3,
    },
    /// INSERT: place a reference to an existing **named block** at one picked
    /// insertion point. The block must already exist in the drawing (this command
    /// never writes the block table); its stored name/base-point metadata and the
    /// `T(insertion) · T(−base_point)` transform are the command layer's input.
    /// `insertion` must be finite; `name` is trimmed, must be non-empty, single
    /// line and at most [`crate::draw_tool::MAX_TOOL_TEXT_CHARS`] characters, and
    /// is resolved exact-first then ASCII case-insensitively against the
    /// drawing's block metadata. Work-only.
    InsertBlock {
        insertion: Point3,
        name: String,
    },
    /// TEXT: one single-line text entity (ADR 0019 F.1, P7a).
    ///
    /// `position` is the captured insertion point, `text` the typed string
    /// (non-empty, at most [`crate::draw_tool::MAX_TOOL_TEXT_CHARS`] characters —
    /// re-validated here so a programmatic caller cannot bypass the capture), and
    /// `height` its glyph height in world units (finite, `> 0`). The capture reads
    /// the height from [`crate::draw_tool::ToolParameter::TextHeight`]; it is a
    /// payload field because the command is the single construction point and the
    /// value must travel with it (never re-read from a session the caller does not
    /// own). The **rotation** deliberately has no payload field: the command reads
    /// it from the session it is handed through the typed channel
    /// [`crate::draw_tool::ToolParameter::TextRotation`] (**degrees**, any finite
    /// value) and stores the radians, so a session at its default stores `0.0`
    /// exactly like the previously fixed unrotated text. The style is the
    /// drawing's `Standard` text style with the style default alignment
    /// (left/baseline).
    ///
    /// Single-line means single-line: a `text` containing `\r`/`\n` is **refused**
    /// (`text.single_line_newline`), because an `AcDbText` does not interpret
    /// `\P` — storing one would render over several lines and could never be
    /// re-edited through `TEXTEDIT`. Use the [`Self::MText`] payload for
    /// multi-line text.
    Text {
        position: Point3,
        text: String,
        height: f64,
    },
    /// MTEXT: one multiline text entity stored as **`AcDbMText`**.
    ///
    /// This is the command core: the interactive `DrawToolKind::MText` tool
    /// (first corner → opposite corner → typed text) builds exactly this payload,
    /// and a direct caller (CLI, host IPC) supplies the whole construction:
    ///
    /// * `text` — the draft string. `\r\n`, `\r` and `\n` are stored as `\P`, the
    ///   form the viewport's MTEXT parser and AutoCAD both interpret, by the one
    ///   newline policy `TEXTEDIT` shares. A later `TEXTEDIT` of this entity
    ///   accepts line breaks again because the entity is `AcDbMText`;
    /// * `position` — insertion point (world, must be finite);
    /// * `width` — the automatic wrap boundary (DXF group `41`
    ///   `rectangle_width`; the **two-corner** model: the caller passes the
    ///   width it captured from the second corner). `None` = no wrap and is
    ///   stored as `None`; `Some(w)` must be finite and `> 0` and is refused
    ///   otherwise (never clamped, never dropped) and sanitized through
    ///   [`cad_domain::wrap_width_from_dxf`] like every other wrap boundary;
    /// * `height` — glyph height in world units (finite, `> 0`);
    /// * `rotation` — rotation in radians (finite; `0.0` = unrotated). The
    ///   interactive capture fills it from
    ///   [`crate::draw_tool::ToolParameter::TextRotation`] (**degrees**, any finite
    ///   value) and converts once; a programmatic caller supplies the radians
    ///   directly, exactly as before this parameter existed;
    /// * `h_align` / `v_align` — horizontal/vertical attachment relative to
    ///   `position`. The interactive capture fills them from
    ///   [`crate::draw_tool::ToolParameter::MTextAttachment`] (AutoCAD's `1..=9`
    ///   codes; `1` = Left/Top, `5` = Center/Middle, `9` = Right/Bottom), so
    ///   `TextAlignV::Baseline` is never produced by the interactive path.
    ///
    /// **Not carried, because not modelled**: line-spacing style, columns,
    /// background fill and drawing direction (the importer's `MTEXT_UNMODELLED`
    /// facets). No field exists for any of them, so none can be believed to be
    /// honoured.
    ///
    /// The style/font resolution is the one `Text` creation uses, and the stored
    /// `type_key` is `AcDbMText` (not `Text`'s `AcDbText`), so this command can
    /// never produce a single-line entity that claims to be multiline.
    MText {
        position: Point3,
        width: Option<f64>,
        text: String,
        height: f64,
        rotation: f64,
        h_align: TextAlignH,
        v_align: TextAlignV,
    },
    /// MLEADER: one straight-line multileader with text content (ADR 0019
    /// decision F.2, P7c).
    ///
    /// `arrow` is the captured arrow position (the first leader vertex `10`),
    /// `landing` the captured landing/content-connection point (the root's
    /// `LEADER{}` group `10`), and `text` the terminal typed string
    /// (non-empty, at most [`crate::draw_tool::MAX_TOOL_TEXT_CHARS`] characters —
    /// re-validated here so a programmatic caller cannot bypass the capture).
    ///
    /// Every style value (dogleg length, landing gap, arrowhead size, text
    /// height, text style, vertical attachment, path type, content kind) is read
    /// from the drawing's `MLeaderStyle` table, so none of them is a payload
    /// field: a caller cannot smuggle a second, contradictory style source in.
    MLeader {
        arrow: Point3,
        landing: Point3,
        text: String,
    },
    /// TEXTEDIT: replace the string of an existing text-bearing entity (ADR 0020
    /// amendment A/B; ADR-0021 D4 generalises it to MTEXT and MULTILEADER text).
    ///
    /// `target` is the ref resolved at pick time (interactive path) or the
    /// current single selection (panel path). The handler re-reads it and refuses
    /// by name when it is gone or not a directly addressable text entity; only
    /// the `text` field changes, through `update_entity` in one transaction /
    /// one undo step. A MULTILEADER is replaced through
    /// `MultiLeaderGeometry::with_text`, which also rebuilds its derived display.
    /// Compound children and block-instance members are not editable (a ref with
    /// an instance path or sub-element is refused). Newline policy (ADR-0021
    /// D4): `AcDbMText` / MULTILEADER drafts store `\r\n`, `\r`, `\n` as `\P`;
    /// an `AcDbText` draft containing them is refused (`textedit.newline_text`).
    /// Every other code passes through verbatim.
    TextEdit {
        target: SelectionRef,
        text: String,
    },
    /// `SETENTITYPROPERTY`: write one property value to **every** currently
    /// selected entity (the properties panel's typed edit surface).
    ///
    /// `key` is one of [`crate::property_value::SUPPORTED_KEYS`] (`layer`,
    /// `color`, `linetype`, `lineweight`, `transparency`); `value` follows that
    /// key's documented grammar ([`crate::property_value`]). `text` is
    /// deliberately **not** accepted here — it has its own `TEXTEDIT` path with
    /// its own newline policy.
    ///
    /// The selection itself travels in neither payload field: the command reads
    /// the session's current selection at apply time, so a stale UI cannot
    /// retarget another object. Every selected ref must be directly addressable
    /// (no instance path, no sub-element); one that is not refuses the whole
    /// command with `property.not_addressable`, and a bad value refuses before
    /// anything is staged. The whole selection is written in **one transaction
    /// / one undo step**; entities that already hold the value are not staged,
    /// and if no entity changes the command records no undo entry at all
    /// (`property.unchanged`).
    EntityProperty {
        key: String,
        value: String,
    },
    /// `GRIPSTRETCH`: move **one** handle of **one** entity to a world point (the
    /// payload of a pointer gesture; the gesture itself lives in `cad-host`).
    ///
    /// The host resolves `to` in the plan view and runs it through the same
    /// drafting constraint the capture path uses, so the previewed marker and the
    /// committed point are one coordinate. The handler re-reads `id.entity`,
    /// refuses a missing entity (`grip.target_missing`) and a non-model-space one
    /// (`property.not_addressable`), then re-validates the handle against the
    /// current geometry. One `update_entity` in one transaction / one undo step;
    /// any refusal opens no transaction and no revision bump. A non-finite `to` is
    /// refused (`grip.non_finite_target`) before the drawing is read. Work-only.
    /// Supported handles: a `LINE` endpoint and a `POINT` position; every other
    /// kind refuses with `grip.unsupported_kind`.
    GripStretch(Box<GripStretchCommand>),
    /// The option of a `MEASUREGEOM <option>` line (only `Radius`/`Volume`; the
    /// four tool options route to their existing `BeginMeasurement` requests).
    MeasureGeom(MeasureGeomOption),
    /// `JUSTIFYTEXT <H> <V>` / `TJUST <H> <V>` (Work-only): set only the
    /// alignment pair of the selected Text/MText. Full contract:
    /// [`crate::command_id::text_batch::JustifyTextCommand`].
    JustifyText(JustifyTextCommand),
    /// `TSCALE <factor>` (Work-only): scale the selected text glyph height / wrap
    /// width with the shared `SCALETEXT` rule. Full contract:
    /// [`crate::command_id::text_batch::TextScaleCommand`].
    TextScale(TextScaleCommand),
    /// `TEXTALIGN <mode>` (Work-only): move the selected text to one common
    /// alignment edge/centre using estimated extents (the 12 AutoCAD 2026 modes
    /// plus the original six single-axis spellings). Full contract:
    /// [`crate::command_id::text_batch::TextAlignCommand`].
    TextAlignEdges(TextAlignCommand),
    /// `TEXTALIGN DISTRIBUTE [X|Y]` (Work-only): even gap distribution; also
    /// carries the three named unsupported spacing options, which are refused
    /// before any transaction. Full contract:
    /// [`crate::command_id::text_batch::TextAlignSpacingCommand`].
    TextAlignSpacing(TextAlignSpacingCommand),
    /// `TCOUNT <start> <step> [prefix] [suffix]` (Work-only): number the selected
    /// text in reading order. Full contract:
    /// [`crate::command_id::text_batch::TCountCommand`].
    TextCount(TCountCommand),
    /// `TCIRCLE CIRCLE|RECTANGLE` (Work-only): draw an estimated-bounds boundary
    /// around each selected text. Full contract:
    /// [`crate::command_id::text_batch::TCircleCommand`].
    TextCircle(TCircleCommand),
    /// The option of a typed `LAYERSTATE <option>` line.
    LayerState(LayerStateCommand),
    /// The target state of `LAYERPMODE ON|OFF` (session-only; gates `LAYERP`
    /// snapshots). Bare `LAYERPMODE` carries `None` and reports.
    LayerPmode(bool),
    /// The value of a typed `LTSCALE <value>` line: finite and strictly positive,
    /// one header revision bump, no undo entry.
    LinetypeScale(f64),
    /// The layer names of a typed `LAYWALK <name>[,<name>...]` line: session
    /// overrides only (plus the active layer), same atomic path as `LAYISO`.
    LayerWalk(Vec<String>),
    /// `QSELECT`/`FILTER`/`GETSEL`; full contract in
    /// [`crate::command_id::select_filter`]. Read-only: only the selection moves.
    SelectFilter {
        property: String,
        value: String,
        mode: SelectionMode,
    },
    /// Typed settings for `SELECTSIMILAR` / `MATCHPROP` / `SETBYLAYER`.
    SelectionTool(Box<SelectionToolPayload>),
    /// `-FIND`/`FIND`; full contract in [`crate::command_id::select_filter`].
    /// `replacement = None` is the report-only form; `-FIND` refuses `Some`.
    TextFind {
        needle: String,
        replacement: Option<String>,
    },
    Purge(PurgeTarget),
    Rename(RenameCommand),
    /// One post-import write on a named style table (`STYLE`, `-STYLE`,
    /// `MLEADERSTYLE`); see [`StyleTableCommand`].
    StyleTable(StyleTableCommand),
    SetVar(String),
    /// `SYSVARMONITOR ADD|REMOVE|LIST <name>`: one session-only monitor action.
    ///
    /// The monitor holds a bounded set of **modelled** `SETVAR` names plus the
    /// value seen at the last query. `LIST` compares each watched value with
    /// that snapshot and reports a change; nothing is written to the drawing.
    /// There is no background notification: a change is reported only when the
    /// monitor is queried.
    SysVarMonitor(SysVarMonitorAction),
    /// `SPACETRANS <distance>`: one model/paper distance conversion.
    ///
    /// The value is a **model-space distance** converted to paper units for the
    /// active layout viewport (and the inverse is reported too). A model-space
    /// session or a layout viewport that cannot be resolved exactly is refused
    /// by name; the conversion never guesses a scale.
    SpaceTrans(f64),
    MRedo(u32),
    Multiple(Option<String>),
    /// The 2026-11-07 draw/modify batch; contracts in `command_id::draw2`.
    Draw2(Box<crate::Draw2Payload>),
    /// `VIEW`/`VIEWGO`/`VPOINT`; contracts in [`crate::app_view`].
    View(ViewCommand),
    ViewGo(String),
    VPoint(VPointInput),
    /// The 2026-11-07 layout/viewport batch; contract:
    /// [`crate::command_id::layout`].
    Layout(Box<crate::LayoutPayload>),
    /// The 2026-11-07 dimension-editing batch (`DIMSTYLE`/`DIMTEDIT`/`DIMEDIT`/
    /// `DIMOVERRIDE`/`DIMJOGLINE`). Grouped in
    /// [`crate::DimensionBatchPayload`]; contract in
    /// `command_id::dimension_edit`.
    Dimension(Box<crate::DimensionBatchPayload>),
    /// The 2026-11-07 annotation/group batch (`GROUP`/`UNGROUP`/`GROUPEDIT`/
    /// `QLEADER`/`LEADER`/`TEXTFIT`/`HATCHSETORIGIN`). Grouped in
    /// [`crate::AnnotationBatchPayload`]; contract in `command_id::annot_batch`.
    Annotation(Box<crate::AnnotationBatchPayload>),
    /// `TRACE`: a closed 4-vertex band from `start` to `end` with `width`.
    /// Stored as one closed `Polyline`; no TRACE entity type exists.
    Trace {
        start: Point3,
        end: Point3,
        width: f64,
    },
    /// `MOCORO`: one complete typed transform of the current selection.
    Mocoro(crate::MocoroCommand),
    /// `MSTRETCH`: the STRETCH planner over several logical crossing windows.
    MStretch {
        rects_logical: Vec<[[f64; 2]; 2]>,
        base: Point3,
        delta: Point3,
    },
    /// `VIEWRES`: the session circle-tessellation percentage (`1..=1000`).
    ViewRes(u32),
    /// `VSCURRENT`: the session visual-style token (`WIREFRAME` only).
    VisualStyle(String),
    /// `LINETYPE`: `LIST` or `SET <name>`.
    Linetype(crate::LinetypeCommand),
    /// `IMAGEATTACH <file> <x> <y> <w> <h> <scale>`: one real `Image` row (Work-only).
    ImageAttach(Box<ImageAttachCommand>),
    /// `IMAGECLIP <minx> <miny> <maxx> <maxy>` / `OFF`: the selected image's clip (Work-only).
    ImageClip(Box<ImageClipCommand>),
    /// `IMAGEADJUST <b> <c> <f> [MONO|COLOR]` / `RESET`: the display transfer of
    /// the one selected image (Work-only); see [`crate::app_drawing::image_edit`].
    ImageAdjust(Box<ImageAdjustCommand>),
    /// `TABLE <rows> <cols> <col_width> <row_height> <x> <y>`: one real
    /// `SemanticGeometry::Table` grid (Work-only); contract in `app_drawing::table_edit`.
    TableCreate(Box<TableCreateCommand>),
    /// `TABLEDIT <row> <col> [text...]`: replace one 1-based cell's text on the
    /// single selected table (Work-only). Contract:
    /// [`crate::app_drawing::table_edit`].
    TableEdit(Box<TableEditCommand>),
    /// `SHAPE <name-or-code> <x> <y> [size] [rotation_deg]`: one real
    /// `SemanticGeometry::Shape` glyph (Work-only). The extent is the shape's
    /// code into the resolved SHX shape font; contract:
    /// [`crate::app_drawing::shape`].
    Shape(Box<ShapeCommand>),
    /// `MINSERT <block> <rows> <columns> <row_spacing> <column_spacing> <x> <y>`:
    /// a rectangular array of INSERT references in one transaction (Work-only).
    MInsert(Box<MInsertCommand>),
    /// `ADDSELECTED <x> <y>`: a new entity of the single selected entity's type
    /// and general properties at the typed point (Work-only).
    AddSelected(Box<AddSelectedCommand>),
    /// `XCLIP RECT|POLY|ON|OFF|DELETE|INVERT|?`: the selected INSERT's
    /// display-only block clip boundary, stored in block-local space (Work-only);
    /// contract in [`crate::app_drawing::xclip_edit`].
    XClip(Box<XClipCommand>),
    /// `MLEDIT` / `-MLEDIT <option> …`: the two-LINE subset of multiline editing
    /// (Work-only). The current selection is the object input: exactly two
    /// `LINE`s. Contract: [`crate::app_drawing::mledit`].
    MlEdit(Box<MlEditCommand>),
    /// `MLEADERALIGN <LEFT|RIGHT|CENTER> [spacing]`: align / space the selected
    /// MULTILEADERs (Work-only). The current selection is the object input.
    /// Contract: [`crate::app_drawing::mleader_edit`].
    MLeaderAlign(Box<MLeaderAlignCommand>),
    /// `MLEADEREDIT ADD <x> <y>` / `MLEADEREDIT REMOVE <index>`: add or remove a
    /// leader line on every selected MULTILEADER (Work-only). Contract:
    /// [`crate::app_drawing::mleader_edit`].
    MLeaderEdit(Box<MLeaderEditCommand>),
    /// `TOLERANCE <x> <y> <text>`: one feature-control frame built from real
    /// `LWPOLYLINE`/`LINE`/`TEXT` entities on the active layer (Work-only). This
    /// build has no single `AcDbFcf` object, so the frame is the declared
    /// separate-entity representation. Contract:
    /// [`crate::app_drawing::tolerance`].
    Tolerance(Box<ToleranceCommand>),
    /// `REFSET ADD|REMOVE`: add or remove the current selection from the active
    /// in-place block edit's working set (Work-only, session-only; no drawing
    /// write). [`crate::CommandPayload::None`] is the bare-selector `Add`.
    /// Contract: [`crate::app_refedit`].
    RefSet(RefSetOption),
    /// `REFCLOSE SAVE|DISCARD`: save the working set back into the block
    /// definition or discard the extracted clones (Work-only). A bare
    /// `REFCLOSE` is `CommandPayload::None` and refused by name
    /// (`refclose.needs_option`). Contract: [`crate::app_refedit`].
    RefClose(RefCloseOption),
}
