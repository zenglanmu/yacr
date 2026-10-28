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

use crate::grips::GripId;
use crate::{AppMode, BackendChoice, MeasurementToolKind, SelectionMode, StandardView};

/// The two straight-line picks of a CHAMFER/FILLET command (ADR 0016 decisions
/// B/C): the two picked refs in pick order and the raw click point that selects
/// each pick side.
///
/// Boxed inside [`CommandPayload::Chamfer`]/`Fillet` so the command enum stays
/// small (the same reason [`CommandPayload::Geometry`] boxes its geometry).
pub struct TwoLineCommand {
    pub first: SelectionRef,
    pub second: SelectionRef,
    pub pick_first: Point3,
    pub pick_second: Point3,
}

/// The input of a DIMRADIUS/DIMDIAMETER command (ADR 0017 decision C, P5d): the
/// picked arc/circle ref and the dimension-line location.
///
/// Boxed inside [`CommandPayload::DimRadial`] so the command enum stays small (the
/// same reason [`TwoLineCommand`] is boxed). Which of the two commands it is comes
/// from the [`CommandId`], so the payload carries no kind field (a second source
/// of truth).
pub struct RadialDimensionCommand {
    pub target: SelectionRef,
    pub location: Point3,
}

/// The two supported DIMANGULAR constructions (ADR 0017 decision C, P5e).
///
/// * [`Self::TwoLine`] — the interactive form: two picked straight lines plus the
///   dimension-arc location. The vertex is the intersection of the two infinite
///   lines (ADR 0017 U-8, pinned against the committed QCAD fixtures).
/// * [`Self::ThreePoint`] — the command-level form: a vertex, the two side
///   points and the dimension-arc location. It has no interactive capture yet:
///   AutoCAD's first prompt ("select a line **or** specify a vertex") cannot be
///   expressed by the fixed-step capture state machine. This is declared in the
///   capability notes, not hidden.
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

/// The axis a DIMORDINATE measures (ADR 0017 decision C/E, P5e).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrdinateAxis {
    /// X ordinate: group 70 bit `0x40` set.
    X,
    /// Y ordinate: group 70 bit `0x40` clear.
    Y,
}

/// The input of a DIMORDINATE command (ADR 0017 decision C, P5e): the feature
/// point, the leader endpoint and an optional explicit axis.
///
/// `axis = None` is the interactive form and applies the documented auto rule —
/// "if the Y difference is greater, measure the X ordinate" — comparing the
/// feature and leader points. Autodesk does not state the equal case; our
/// declared reading is **Y** (the documented rule's "otherwise" branch).
#[derive(Debug, Clone, PartialEq)]
pub struct OrdinateDimensionCommand {
    pub feature: Point3,
    pub leader: Point3,
    pub axis: Option<OrdinateAxis>,
}

/// The input of a DIMBASELINE/DIMCONTINUE command (ADR 0017 decision C, P5f):
/// the picked linear base dimension plus the N picked points, in pick order.
///
/// Boxed inside [`CommandPayload::DimChain`] so the command enum stays small (the
/// same reason [`TwoLineCommand`] is boxed). Which of the two commands it is comes
/// from the [`CommandId`], so the payload carries no kind field (a second source
/// of truth).
///
/// The base is the ref resolved **at pick time**; the command re-reads it, so a
/// base that was deleted (or replaced) since the pick is refused by name and
/// nothing is written. The entity stores **no** base identity and no row index
/// (the ADR's non-associative boundary): each created dimension is an ordinary,
/// independent `DIMENSION`.
pub struct DimensionChainCommand {
    pub base: SelectionRef,
    /// The picked points in pick order; the command refuses an empty list.
    pub points: Vec<Point3>,
}

/// The input of a `GRIPSTRETCH` command: **one** handle drag on **one** entity.
///
/// `id` is the handle resolved at pick time (which entity, which kind of handle,
/// which one — see [`crate::grips::GripId`]) and `to` the world point the drag
/// resolved to. Boxed inside [`CommandPayload::GripStretch`] so the payload enum
/// stays small (the same reason [`TwoLineCommand`] and [`DimensionChainCommand`]
/// are boxed).
///
/// `id` is a caller-held reference into geometry that may have moved or been
/// deleted since it was read, so the command re-reads the entity and re-validates
/// the handle against the **current** geometry instead of trusting it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GripStretchCommand {
    pub id: GripId,
    pub to: Point3,
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
    /// Structured zoom: a positive factor plus the cursor in logical pixels that
    /// must stay anchored. Distinct from the legacy `Points`-payload zoom that
    /// takes no cursor.
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
    /// **2D orthographic only**: in a 3D observation the command refuses by name
    /// (`zoom.window.requires_2d`) rather than faking a perspective fit.
    /// View-only (allowed in Viewer mode): no database write, no revision bump,
    /// no `cad-history` entry, and no `session.generation` bump.
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
    /// ARRAYPOLAR the selected refs around `center` (ADR 0016 decision D).
    ///
    /// `center` is the captured array centre; the item count, fill angle and
    /// rotate-items flag are the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ArrayItems`] … `ArrayRotateItems`).
    /// The source refs are retained; every copy is a fresh entity in one
    /// transaction. Associativity is NOT IMPLEMENTED.
    ArrayPolar {
        refs: Vec<SelectionRef>,
        center: Point3,
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
    /// `layer` addresses the row by **name** with the same rule as
    /// [`CommandPayload::RenameLayer`] (exact match first, then a unique
    /// case-insensitive one; unknown and ambiguous names are distinct refusals).
    /// `key` is one of [`crate::layer_property::SUPPORTED_KEYS`] and `value`
    /// follows that key's documented grammar ([`crate::layer_property`], which
    /// also fixes the units: `lineweight` is **1/100 mm**, `transparency` is
    /// **percent transparent**). `plotstyle` is a known key that is **refused as
    /// not implemented**, and `name` / `visible` are refused because they belong
    /// to `RENAMELAYER` and to the derived `visible == !(off || frozen)` rule.
    ///
    /// One attribute per command: the value is parsed, resolved against the
    /// drawing's own tables (a linetype name must exist) and applied to a
    /// **candidate** attribute set before anything is published, so a refused
    /// value changes no attribute, bumps no revision and records no undo entry
    /// (layer edits are **not** undoable — same as create/rename/delete).
    ///
    /// `off` / `frozen` are the only keys with a behaviour beyond storage: the
    /// database recomputes `Layer::visible` from the pair in the same mutation,
    /// so turning one on hides the layer through the existing visibility path and
    /// clearing the last of the two shows it again. `locked`, `plottable`, the
    /// colour, the linetype, the lineweight and the transparency are stored only.
    SetLayerProperty {
        layer: String,
        key: String,
        value: String,
    },
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
    /// A fully formed semantic geometry, used by `CreateLine` as the
    /// programmatic alternative to a point list.
    Geometry(Box<SemanticGeometry>),
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
    /// **Not carried, because not modelled** — the same four facets the importer
    /// marks `Partial` under `MTEXT_UNMODELLED`: line-spacing style
    /// (`text.line_spacing_style_unsupported`), columns
    /// (`text.columns_unsupported`), background fill
    /// (`text.background_unsupported`) and drawing direction
    /// (`text.drawing_direction_unsupported`). There is deliberately no field for
    /// any of them, so no caller can believe one is honoured.
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
    /// `GRIPSTRETCH`: move **one** handle of **one** entity to a world point
    /// (the payload path of a grip drag; see [`crate::grips`] for the model).
    ///
    /// This is **the payload of a pointer gesture**: the gesture itself lives in
    /// `cad-host` (`input/grip_drag.rs`) — a press on a marker of the selection
    /// arms a drag, the moved marker is the preview, and the release commits one
    /// `GripStretch`. The command is defined independently so the write path, its
    /// refusals and its undo granularity stay pinned without a pointer, a marker
    /// or a session.
    ///
    /// `id` names the dragged handle and `to` the world point it is dragged to.
    /// The host resolves `to` in the plan view (x/y from the viewport's
    /// `screen_to_world`, the dragged handle's own `z`) and then runs it through
    /// the same drafting constraint the capture path uses
    /// (`SessionState::constrained_point`: grid `SNAP` first, `ORTHO` second
    /// against the handle's original position), so the previewed marker and the
    /// committed point are one coordinate. The command stores all three
    /// coordinates as given — nothing is projected onto a work plane here. The
    /// handler
    /// re-reads `id.entity`, refuses a missing entity (`grip.target_missing`) and
    /// a non-model-space one (`property.not_addressable`), then re-validates the
    /// handle against the current geometry through
    /// [`crate::grips::apply_grip`]. On success it stages exactly one
    /// `update_entity` in one transaction / one undo step; on **any** refusal it
    /// opens no transaction at all, so there is no empty undo entry and no
    /// revision bump (a zero displacement is refused, not accepted as a no-op —
    /// the host turns that refusal into its own no-op). A non-finite `to` is
    /// refused (`grip.non_finite_target`) before the drawing is read. Work-only.
    ///
    /// Supported handles are the model's whole surface: a `LINE` endpoint and a
    /// `POINT` position. Every other geometry kind refuses with
    /// `grip.unsupported_kind`; this never routes through
    /// [`CommandId::StretchEntities`] (crossing-window stretch).
    GripStretch(Box<GripStretchCommand>),
}
