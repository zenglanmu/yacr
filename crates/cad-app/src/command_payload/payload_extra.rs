//! Payload structs split out of `command_payload.rs` to keep that file inside the
//! code-size limit. The enum still owns the variants; this module owns the data.

use cad_domain::{CadError, CadResult, LayerId, Point3, SelectionRef};

use crate::grips::GripId;

mod hatch_edit_payload;
mod image;
mod qdim_payload;
mod table;

pub use hatch_edit_payload::*;
pub use image::*;
pub use qdim_payload::*;
pub use table::*;

mod utility;
pub use utility::*;

/// The first OFFSET prompt: a fixed offset distance, or `Through` (the pick
/// point defines the distance, so the offset passes through it).
///
/// `Through` is the **legacy** behaviour of the bare `CommandPayload::Offset`
/// variant: the side point both chooses the side and fixes the distance. A
/// fixed `Distance` keeps the pick as the side selector only.
// The variants are constructed by the option entry points, the typed grammar
// and the tests; `pub(crate)` use keeps the enum out of the host surface while
// `command_payload.rs` is at its line ceiling.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OffsetMode {
    /// A user-supplied constant distance (must be finite and positive).
    Distance(f64),
    /// Derive the distance from each pick point (AutoCAD's `Through` prompt).
    Through,
}

/// The OFFSET `Layer` option: where the new entities land.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetLayer {
    /// The source entity's layer (the bare command's behaviour).
    Source,
    /// The current layer, resolved by the caller at capture time
    /// (`SessionState::active_layer`). The layer must exist.
    Current(LayerId),
}

/// The OFFSET `OFFSETGAPTYPE` option for polyline joins.
///
/// All three documented values are implemented: [`OffsetGapType::Sharp`]
/// (AutoCAD `OFFSETGAPTYPE` 0, the analytic miter join), [`OffsetGapType::Fillet`]
/// (1, a tangent arc of radius equal to the offset distance filling each real
/// gap) and [`OffsetGapType::Chamfer`] (2, a straight segment across each real
/// gap). Where two offset segments already cross (a concave corner) every value
/// keeps the miter, because there is no gap to fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetGapType {
    Sharp,
    Fillet,
    Chamfer,
}

/// The complete OFFSET option set: the payload data behind the documented
/// `[Through/Erase/Layer]` first prompt and the `[Exit/Multiple/Undo]` side
/// point loop.
///
/// `side_points` is the `Multiple` loop: each point creates one offset from the
/// **same** source in the same transaction. There is no interactive prompt loop
/// in this build, so the loop is expressed as a list.
///
/// Wiring: [`crate::CommandPayload::OffsetOptions`] routes to
/// [`crate::Application::offset_entities_with_options`]; the typed `OFFSET`
/// argument lines store the session defaults and the capture map emits this
/// variant when they differ from the legacy defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct OffsetOptions {
    pub refs: Vec<SelectionRef>,
    pub side_points: Vec<Point3>,
    pub mode: OffsetMode,
    /// AutoCAD `Erase`: delete the source after every offset is created, in the
    /// same transaction / undo step.
    pub erase_source: bool,
    pub layer: OffsetLayer,
    pub gap: OffsetGapType,
}

/// The complete MIRROR option set.
///
/// `mirrortext` is AutoCAD's `MIRRTEXT`: `false` keeps a reflected text run
/// readable (the position mirrors, the glyph order does not reverse), `true`
/// would mirror the glyphs themselves.
///
/// Wiring: like [`OffsetOptions`], [`crate::CommandPayload::MirrorOptions`]
/// routes to [`crate::Application::mirror_entities_with_options`]; a bare
/// `Mirror` payload keeps the defaults and delegates to the same entry point.
#[derive(Debug, Clone, PartialEq)]
pub struct MirrorOptions {
    pub refs: Vec<SelectionRef>,
    pub axis_start: Point3,
    pub axis_end: Point3,
    /// `MIRRTEXT`: `false` keeps text readable (the default), `true` is a
    /// named refusal because the flat-text domain cannot mirror glyphs.
    pub mirrortext: bool,
    /// AutoCAD's `Erase source objects? Yes/No`.
    pub erase_source: bool,
}

/// The input of a DIMBASELINE/DIMCONTINUE command (ADR 0017 decision C, P5f):
/// the picked linear base dimension plus the N picked points, in pick order.
///
/// Boxed inside [`crate::CommandPayload::DimChain`] so the command enum stays
/// small (the same reason `TwoLineCommand` is boxed).
/// Which of the two commands it is comes from the [`crate::CommandId`], so the
/// payload carries no kind field (a second source of truth).
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
/// resolved to. Boxed inside [`crate::CommandPayload::GripStretch`] so the
/// payload enum stays small (the same reason [`TwoLineCommand`] and
/// [`DimensionChainCommand`] are boxed).
///
/// `id` is a caller-held reference into geometry that may have moved or been
/// deleted since it was read, so the command re-reads the entity and re-validates
/// the handle against the **current** geometry instead of trusting it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GripStretchCommand {
    pub id: GripId,
    pub to: Point3,
}

/// The named-object kind a `PURGE`/`RENAME` line targets.
///
/// The database exposes delete/rename APIs for **layers only**; the other kinds
/// are modelled so an explicit request can be refused by name instead of being
/// silently ignored or treated as a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedObjectKind {
    Layer,
    Block,
    Style,
    Linetype,
}

/// The target of a `PURGE` line.
///
/// `All` is the bare `PURGE` form: every kind the build can safely purge is
/// attempted and the kinds without a delete API are reported by name. An
/// explicit `Layer`/`Block`/`Style`/`Linetype` narrows the attempt to that kind;
/// a kind with no delete API is an explicit `Unsupported` refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeTarget {
    All,
    Layer,
    Block,
    Style,
    Linetype,
}

/// The input of a typed `RENAME <kind> <old> <new>` line.
///
/// Names are passed through verbatim: the database trims and validates them, so
/// an empty, illegal or duplicate name is refused there rather than repaired
/// here. A name containing whitespace must be quoted on the command line; the
/// `cad-ui-core` parser owns that tokenisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameCommand {
    pub kind: NamedObjectKind,
    pub old: String,
    pub new: String,
}

/// The option of a typed `LAYERSTATE <option>` line: `SAVE <name>`,
/// `RESTORE <name>`, `LIST` or `DELETE <name>` (grammar in `cad-ui-core`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerStateCommand {
    /// Snapshot the layer attributes, active layer and overrides.
    Save(String),
    /// Apply a saved snapshot (layer-table write; records one layer-patch undo
    /// step for the changed rows).
    Restore(String),
    /// Report the saved names; writes nothing.
    List,
    /// Drop one saved snapshot.
    Delete(String),
}

/// The option of a typed `VIEW <option>` / `-VIEW <option>` line:
/// `SAVE <name>`, `RESTORE <name>`, `LIST` or `DELETE <name>`.
///
/// The store is session only (bounded to 32 case-insensitively unique names);
/// nothing here is written to the DWG. The grammar lives in `cad-ui-core`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewCommand {
    /// Capture the current camera, 2D/3D mode, work plane and active space.
    Save(String),
    /// Apply a saved view (the same path as `VIEWGO`).
    Restore(String),
    /// Report the saved names; writes nothing.
    List,
    /// Drop one saved view.
    Delete(String),
}

/// The camera target of a typed `VPOINT` line.
///
/// The six plain words reuse the existing standard-view path (`TOP` is the 2D
/// plan); the vector form names a viewing direction in world coordinates and
/// always produces a 3D observation. A zero or non-finite vector is refused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VPointInput {
    /// `TOP|BOTTOM|FRONT|BACK|LEFT|RIGHT`.
    Standard(crate::StandardView),
    /// `<x> <y> <z>`, the eye direction from the target.
    Vector(Point3),
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

/// The 2026-11-07 dimension-editing batch. Grouped into one
/// [`crate::CommandPayload::Dimension`] variant so the payload enum stays inside
/// the file-size budget; every contract lives in
/// [`crate::command_id::dimension_edit`].
#[derive(Debug, Clone, PartialEq)]
pub enum DimensionBatchPayload {
    /// `DIMSTYLE LIST|CURRENT|SET <name>`.
    Style(DimStyleCommand),
    /// `DIMTEDIT MOVE|LEFT|RIGHT|CENTER|HOME|ANGLE`.
    TextEdit(DimTextEditCommand),
    /// `DIMEDIT TEXT <override>|RESET`.
    Edit(DimEditCommand),
    /// `DIMOVERRIDE <variable> <value>`.
    Override(DimOverrideCommand),
    /// `DIMJOGLINE ADD <x> <y>|REMOVE`.
    JogLine(DimJogLineCommand),
    /// `QDIM <mode> [DATUM x y] [EDIT i…] [SETTINGS ENDPOINTS|INTERSECTIONS]`.
    QDim(QDimCommand),
}

/// The typed option of a `DIMSTYLE` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DimStyleCommand {
    /// Report every stored dimension style (read-only).
    List,
    /// Report the style dimension creation resolves (read-only).
    Current,
    /// Request the creation style; this build has no post-import style-table
    /// write API, so the application refuses it by name.
    Set(String),
}

/// The typed option of a `DIMTEDIT` line.
///
/// The documented AutoCAD 2026 set is `Left`/`Right`/`Center`/`Home`/`Angle`
/// plus the free text location; the former `ROTATE` spelling is the `Angle`
/// option (`DIMEDIT`'s Rotate is a different command). There is no `RESET`:
/// `Home` restores the default text position and `Angle 0` the default
/// orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DimTextEditCommand {
    /// Move the measurement text to this drawing-plane X/Y (Z stays on the
    /// dimension's own plane).
    Move { x: f64, y: f64 },
    /// Justify the text to the start (minimum projection) of its dimension line.
    Left,
    /// Justify the text to the end (maximum projection) of its dimension line.
    Right,
    /// Justify the text to the midpoint of its dimension line.
    Center,
    /// Restore the derived text position (the group 11 sentinel); the rotation
    /// is untouched.
    Home,
    /// Set the text rotation to these degrees about the UCS X axis, keeping the
    /// text centre where it is; `0` restores the derived orientation.
    Angle { degrees: f64 },
}

/// The typed option of a `DIMEDIT` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DimEditCommand {
    /// Store a dimension text override (group 1); `<>` is the measurement
    /// placeholder.
    Text(String),
    /// Drop the text override and render the measurement again.
    Reset,
}

/// The input of a `DIMOVERRIDE <variable> <value>` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DimOverrideCommand {
    pub variable: String,
    /// The trimmed remainder of the line, verbatim (no quote processing).
    pub value: String,
}

/// The typed option of a `DIMJOGLINE` line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DimJogLineCommand {
    Add { x: f64, y: f64 },
    Remove,
}

/// The 2026-11-07 annotation/group batch. Grouped into one
/// [`crate::CommandPayload::Annotation`] variant so the payload enum stays inside
/// the file-size budget; every contract lives in
/// [`crate::command_id::annot_batch`].
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationBatchPayload {
    /// `GROUP CREATE|LIST|DELETE <name>`; bare `GROUP` is `None` and lists.
    Group(GroupCommand),
    /// `UNGROUP <name>`.
    Ungroup(String),
    /// `GROUPEDIT <name> ADD|REMOVE|RENAME <arg>`.
    GroupEdit(GroupEditCommand),
    /// `QLEADER` / `LEADER` typed geometry and text.
    Leader(LeaderCommand),
    /// `TEXTFIT <target width>`.
    TextFit(TextFitCommand),
    /// `HATCHSETORIGIN <x> <y>`.
    HatchSetOrigin(HatchOriginCommand),
    /// The 2026-11 text/draw utilities (`REVCLOUD`, `WIPEOUT`, `TEXTMASK`,
    /// `TXT2MTXT`) grouped here so `CommandPayload` keeps its file-size budget.
    Utility(Box<UtilityPayload>),
}

/// The typed option of a `GROUP` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupCommand {
    /// Capture the current selection under a new session-only name.
    Create(String),
    /// Report every stored name (read-only).
    List,
    /// Remove one named group; the entities are retained.
    Delete(String),
}

/// The typed option of a `GROUPEDIT` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupEditCommand {
    /// Add the current selection, or these typed model entity ids when the
    /// selection is empty.
    Add { name: String, entities: Vec<u128> },
    /// Remove the current selection, or these typed model entity ids.
    Remove { name: String, entities: Vec<u128> },
    /// Rename the group; membership is unchanged.
    Rename { name: String, new_name: String },
}

/// The input of a `QLEADER` / `LEADER` line (both map to one `MULTILEADER`).
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderCommand {
    /// The arrowhead point.
    pub arrow: Point3,
    /// The landing point (where the leader line ends and the text begins).
    pub landing: Point3,
    /// The non-empty, bounded text content.
    pub text: String,
}

/// The input of a `TEXTFIT <target width>` line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextFitCommand {
    /// The requested estimated width in drawing units.
    pub target_width: f64,
}

/// The input of a `HATCHSETORIGIN <x> <y>` line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HatchOriginCommand {
    pub x: f64,
    pub y: f64,
}

/// The typed operation of one `MOCORO` line over the current selection.
///
/// This is the complete, non-interactive operation: there is no in-command
/// option loop or gizmo. `Copy` keeps the sources and inserts translated clones;
/// the other three transform the selection in place. All reuse the existing
/// `COPY`/`MOVE`/`ROTATE`/`SCALE` write paths and therefore their refusal rules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MocoroCommand {
    /// `MOCORO MOVE <dx> <dy>`: translate in place.
    Move { delta: Point3 },
    /// `MOCORO COPY <dx> <dy>`: translated clones, sources retained.
    Copy { delta: Point3 },
    /// `MOCORO ROTATE <basex> <basey> <degrees>`: rotate in place.
    Rotate { base: Point3, degrees: f64 },
    /// `MOCORO SCALE <basex> <basey> <factor>`: scale in place.
    Scale { base: Point3, factor: f64 },
}

/// The typed option of one `LINETYPE` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinetypeCommand {
    /// `LINETYPE LIST`: read-only report of the drawing's linetype table.
    List,
    /// `LINETYPE SET <name>`: write that linetype to every selected entity.
    Set(String),
}

/// The optional property dimensions `SELECTSIMILAR` compares.
///
/// AutoCAD 2026's default similarity set is **Layer + Object type only**; every
/// other modelled dimension is opt-in. Object type is not a toggle: only
/// objects of the same type are considered at all. Transparency is *not* an
/// AutoCAD similarity criterion and has no field here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectSimilarSettings {
    /// Compare the entity's stored layer id (on by default).
    pub layer: bool,
    /// Compare the stored display colour (`EntityColor`).
    pub color: bool,
    /// Compare the stored display linetype (`EntityLineType`).
    pub linetype: bool,
    /// Compare the stored display lineweight (`EntityLineWeight`).
    pub lineweight: bool,
    /// Compare the block name of `INSERT` references (AutoCAD's **Name**).
    pub name: bool,
    /// Compare the text style (TEXT/MTEXT) or dimension style (DIMENSION)
    /// name (AutoCAD's **Object style**).
    pub object_style: bool,
}

impl Default for SelectSimilarSettings {
    fn default() -> Self {
        SelectSimilarSettings {
            layer: true,
            color: false,
            linetype: false,
            lineweight: false,
            name: false,
            object_style: false,
        }
    }
}

/// The per-property `MATCHPROP` Settings subset.
///
/// [`Default`] copies every modelled property, matching AutoCAD's Match
/// Properties defaults. `basic` is the master switch for the Basic group
/// (colour, layer, linetype, linetype scale, lineweight, transparency); the
/// three Special groups are gated separately and only apply when the source and
/// destination are both of the group's object class.
///
/// `linetype_scale` is the entity's own scale factor, which this build stores
/// beside the linetype (`EntityRenderAttributes::linetype_scale`, mirrored
/// inside `EntityLineType::Explicit`); it is a separate toggle because AutoCAD's
/// dialog lists it separately. Plot style and thickness are **not modelled** on
/// an entity in this build, so they have no field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchPropSettings {
    /// The Basic group master switch.
    pub basic: bool,
    pub layer: bool,
    pub color: bool,
    pub linetype: bool,
    pub lineweight: bool,
    pub transparency: bool,
    pub linetype_scale: bool,
    /// Text Special group: copy the TEXT/MTEXT style reference (never height).
    pub text_style: bool,
    /// Dimension Special group: copy the DIMENSION style reference/snapshot
    /// (never overrides).
    pub dimension_style: bool,
    /// Hatch Special group: copy pattern/angle/scale/style (never the origin).
    pub hatch: bool,
}

impl Default for MatchPropSettings {
    fn default() -> Self {
        MatchPropSettings {
            basic: true,
            layer: true,
            color: true,
            linetype: true,
            lineweight: true,
            transparency: true,
            linetype_scale: true,
            text_style: true,
            dimension_style: true,
            hatch: true,
        }
    }
}

/// The `SETBYLAYER` options: AutoCAD's **Include blocks** switch.
///
/// The default is `false`, matching the bare command (only the selected
/// top-level entities are reset). `true` recurses into every selected `INSERT`
/// and rewrites the display attributes of its block members through the same
/// transaction/undo step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SetByLayerSettings {
    pub include_blocks: bool,
}

/// Typed payloads of the selection-based commands and of three commands that
/// share this already-boxed channel because [`crate::CommandPayload`] is at its
/// code-size ceiling (`OVERKILL`, `SSX`, `AUDIT`).
///
/// Each command accepts only its own arm and refuses any other, so the union can
/// never make one command read another's settings. The older
/// `CommandPayload::SelectFilter` form stays the one-shot `PROPERTY VALUE`
/// command line; both are handled by [`crate::select_filter`].
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionToolPayload {
    /// `SELECTSIMILAR` property dimensions.
    SelectSimilar(SelectSimilarSettings),
    /// `MATCHPROP` Settings subset.
    MatchProp(MatchPropSettings),
    /// `SETBYLAYER` options.
    SetByLayer(SetByLayerSettings),
    /// Structured `QSELECT` (dialog semantics without a dialog).
    QSelect(Box<QSelectRequest>),
    /// Structured `FILTER` (logical expression over the shared property set).
    Filter(Box<FilterRequest>),
    /// `OVERKILL` (Delete Duplicate Objects) tolerance/ignore/option set. This is
    /// not a selection *tool*; it rides this already-boxed payload channel
    /// because [`crate::CommandPayload`] is at its code-size ceiling and adding a
    /// top-level variant is not possible without a sibling refactor. The typed
    /// settings are [`crate::OverkillSettings`]; the handler is
    /// `Application::overkill`.
    Overkill(Box<crate::app_drawing::OverkillSettings>),
    /// `SSX` Express-Tool filter edit + apply. Contract:
    /// [`crate::command_id::select_filter`] and [`crate::ssx`].
    Ssx(crate::ssx::SsxRequest),
    /// `AUDIT` answers (the `Fix any errors detected?` prompt and `AUDITCTL`).
    /// Contract: [`crate::app_audit`].
    Audit(crate::app_audit::AuditRequest),
}

/// A property QSELECT/FILTER can compare.
///
/// The set is the **shared modelled set**: object type, layer, colour, linetype,
/// lineweight, linetype scale, transparency, space, plus the text content and
/// the block name where this build models them. AutoCAD does not document
/// `FILTER` and `QSELECT` property sets as identical, so this is a declared
/// common subset rather than a claim of parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionProperty {
    /// The object type (stored DXF type key or geometry alias).
    Type,
    Layer,
    Color,
    Linetype,
    Lineweight,
    /// The entity's own linetype scale (DXF group 48).
    LinetypeScale,
    /// Transparency percentage (`0..=90`, or the symbolic ByLayer/ByBlock).
    Transparency,
    /// The entity's space (`Model` or a paper-layout name).
    Space,
    /// TEXT/MTEXT content, or a MULTILEADER's annotation text.
    Text,
    /// An INSERT's block name.
    Name,
}

/// The comparison operators AutoCAD's Quick Select exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOperator {
    Equals,
    NotEqual,
    GreaterThan,
    LessThan,
    GreaterThanOrEqual,
    LessThanOrEqual,
    /// `*Wildcard Match*`: anchored ASCII-case-insensitive `*`/`?` matching.
    WildcardMatch,
    /// `Select All`: matches every candidate (the value is ignored).
    SelectAll,
}

/// Where the query reads its candidates from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionScope {
    /// Every top-level entity in the current space. The AutoCAD wording is
    /// "entire drawing"; this build declares **current-space** semantics.
    EntireDrawing,
    /// The current selection (only top-level refs are candidates).
    CurrentSelection,
}

/// How the matches change the selection set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionApply {
    /// `Include in New Selection Set`.
    IncludeNew,
    /// `Exclude from New Selection Set`.
    ExcludeNew,
    /// `Append to Current Selection Set`.
    Append,
}

/// One `PROPERTY <operator> VALUE` clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionPredicate {
    pub property: SelectionProperty,
    pub operator: SelectionOperator,
    pub value: String,
}

/// The structured `QSELECT` request: an optional object-type restriction plus
/// one predicate, the candidate scope and the apply mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QSelectRequest {
    /// AutoCAD's "Object type" dropdown; `None` is "Multiple".
    pub object_type: Option<String>,
    pub predicate: SelectionPredicate,
    pub scope: SelectionScope,
    pub apply: SelectionApply,
}

/// The logical connectives of a FILTER expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterLogic {
    /// One or more members.
    And,
    /// One or more members.
    Or,
    /// Exactly two members.
    Xor,
    /// Exactly one member.
    Not,
}

/// A FILTER criterion tree.
///
/// `Group` is the typed form of the dialog's balanced Begin/End pair: the tree
/// cannot be unbalanced by construction, and [`crate::select_filter`] validates
/// the arity rules (AND/OR ≥ 1, XOR exactly 2, NOT exactly 1) before matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterExpression {
    Predicate(SelectionPredicate),
    Group {
        logic: FilterLogic,
        members: Vec<FilterExpression>,
    },
}

/// The structured `FILTER` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterRequest {
    pub expression: FilterExpression,
    pub scope: SelectionScope,
    pub apply: SelectionApply,
}

/// The declared AutoCAD 2026 `PEDIT` v1 option subset used by `MPEDIT`.
///
/// The command line carries the option (`PEDIT CLOSE|OPEN|REVERSE`) because this
/// build has no in-command option prompt. `MPEDIT` applies it to every selected
/// polyline; the full single-polyline option set (JOIN, WIDTH, FIT, SPLINE,
/// DECURVE, …) is [`PeditEditOption`]. Full contract:
/// [`crate::command_id::pedit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeditOption {
    /// `CLOSE` / `C`: close the one selected open polyline (at least 3 vertices).
    Close,
    /// `OPEN` / `O`: open the one selected closed polyline.
    Open,
    /// `REVERSE` / `R`: reverse vertex order and bulges exactly like `REVERSE`.
    Reverse,
}

/// One typed option of the full AutoCAD 2026 `PEDIT` line form.
///
/// This is the command-line argument form (`PEDIT JOIN`, `PEDIT FIT`,
/// `PEDIT SPLINE 32`, …). The current selection is still the input; the first
/// selected entity is the polyline being edited and (for `Join`) the remaining
/// picks are the objects welded into it. Full contract:
/// [`crate::command_id::pedit`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PeditEditOption {
    /// `CLOSE` / `C`: close the polyline.
    Close,
    /// `OPEN` / `O`: open the polyline.
    Open,
    /// `JOIN` / `J`: weld touching LINEs, ARCs and open polylines into the base
    /// polyline (touch-only; no fuzz distance in this build).
    Join,
    /// `WIDTH <w>`: named capability refusal (no per-polyline width field).
    Width(f64),
    /// `EDIT` / `E`: named capability refusal (no vertex-edit sub-menu channel).
    EditVertex,
    /// `FIT` / `F`: replace the segments with a tangent-continuous arc chain
    /// through every vertex (open polylines only).
    Fit,
    /// `SPLINE` / `S`: store the vertices as the control frame of an
    /// approximating clamped B-spline. `precision` is AutoCAD's 0–99 prompt,
    /// validated and recorded but geometrically inert (the result is an exact
    /// B-spline, not a tessellation).
    Spline { precision: u8 },
    /// `DECURVE` / `D`: remove fit/spline data and restore the straight frame.
    Decurve,
    /// `LTYPE GEN ON|OFF`: named capability refusal (no `plinegen` flag in the
    /// entity model).
    LtypeGen { enabled: bool },
    /// `REVERSE` / `R`: reverse vertex order and bulges.
    Reverse,
    /// `UNDO` / `U`: apply the document undo step (this build has no
    /// in-command PEDIT option stack).
    Undo,
}

/// The input of one full `PEDIT <option>` invocation.
///
/// [`convert`](Self::convert) is the `PEDITACCEPT` answer: `true` converts the
/// selected LINE/ARC/SPLINE into a polyline before the option runs; `false`
/// keeps the named `pedit.not_polyline` refusal. This build has no dialog, so
/// the answer is part of the typed command line rather than an interactive
/// prompt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeditEditCommand {
    pub option: PeditEditOption,
    /// The `PEDITACCEPT` answer (convert LINE/ARC/SPLINE, then apply the option).
    pub convert: bool,
}

/// Which JOIN input mode the command line selected.
///
/// `SourceObject` is AutoCAD's default: the **first** picked object's type
/// decides the accepted kinds and the result type. `Multiple` is the explicit
/// `JOIN MULTIPLE` mode where the whole selection is classified together
/// (this is the only mode where a LINE plus an ARC becomes a POLYLINE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinMode {
    SourceObject,
    Multiple,
}

/// The typed options of the AutoCAD 2026 `JOIN` command.
///
/// A bare `JOIN` (`CommandPayload::None`) is
/// `JoinOptions { close: false, mode: SourceObject }`. `close` is AutoCAD's
/// `Close` sub-option (form a CIRCLE/ELLIPSE when the picks close a full turn).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinOptions {
    pub close: bool,
    pub mode: JoinMode,
}

impl Default for JoinOptions {
    fn default() -> Self {
        JoinOptions {
            close: false,
            mode: JoinMode::SourceObject,
        }
    }
}
impl SelectionProperty {
    /// Parse one property word, ASCII case-insensitive. The accepted spellings
    /// include the DXF-style short names (`LTSCALE`) and the dialog labels.
    pub fn parse(text: &str) -> Option<Self> {
        let word: String = text
            .trim()
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '_' && *c != '-')
            .collect();
        if word.eq_ignore_ascii_case("TYPE")
            || word.eq_ignore_ascii_case("OBJECTTYPE")
            || word.eq_ignore_ascii_case("CLASS")
        {
            Some(Self::Type)
        } else if word.eq_ignore_ascii_case("LAYER") {
            Some(Self::Layer)
        } else if word.eq_ignore_ascii_case("COLOR") || word.eq_ignore_ascii_case("COLOUR") {
            Some(Self::Color)
        } else if word.eq_ignore_ascii_case("LINETYPE") {
            Some(Self::Linetype)
        } else if word.eq_ignore_ascii_case("LINEWEIGHT") {
            Some(Self::Lineweight)
        } else if word.eq_ignore_ascii_case("LINETYPESCALE") || word.eq_ignore_ascii_case("LTSCALE")
        {
            Some(Self::LinetypeScale)
        } else if word.eq_ignore_ascii_case("TRANSPARENCY") {
            Some(Self::Transparency)
        } else if word.eq_ignore_ascii_case("SPACE") {
            Some(Self::Space)
        } else if word.eq_ignore_ascii_case("TEXT") || word.eq_ignore_ascii_case("CONTENT") {
            Some(Self::Text)
        } else if word.eq_ignore_ascii_case("NAME") || word.eq_ignore_ascii_case("BLOCKNAME") {
            Some(Self::Name)
        } else {
            None
        }
    }

    /// Whether the property is compared numerically by the relative operators.
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::Lineweight | Self::LinetypeScale | Self::Transparency
        )
    }
}

pub fn validate_predicate(prefix: &str, predicate: &SelectionPredicate) -> CadResult<()> {
    if predicate.operator == SelectionOperator::SelectAll {
        return Ok(());
    }
    if predicate.value.trim().is_empty() {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.empty_value: the filter value must not be empty"
        )));
    }
    if matches!(
        predicate.operator,
        SelectionOperator::GreaterThan
            | SelectionOperator::LessThan
            | SelectionOperator::GreaterThanOrEqual
            | SelectionOperator::LessThanOrEqual
    ) {
        if predicate.property == SelectionProperty::Type {
            return Err(CadError::InvalidInput(format!(
                "{prefix}.relative_non_numeric: the object-type property cannot be compared \
                 with a relative operator"
            )));
        }
        if !predicate.property.is_numeric() {
            return Err(CadError::InvalidInput(format!(
                "{prefix}.relative_non_numeric: {:?} is not a numeric property",
                predicate.property
            )));
        }
        if predicate.value.trim().parse::<f64>().is_err() {
            return Err(CadError::InvalidInput(format!(
                "{prefix}.relative_non_numeric: '{}' is not a number",
                predicate.value
            )));
        }
    }
    Ok(())
}

pub fn validate_expression(prefix: &str, expression: &FilterExpression) -> CadResult<()> {
    match expression {
        FilterExpression::Predicate(predicate) => validate_predicate(prefix, predicate),
        FilterExpression::Group { logic, members } => {
            match logic {
                FilterLogic::And | FilterLogic::Or => {
                    if members.is_empty() {
                        return Err(CadError::InvalidInput(format!(
                            "{prefix}.group_arity: AND/OR needs at least one member"
                        )));
                    }
                }
                FilterLogic::Xor => {
                    if members.len() != 2 {
                        return Err(CadError::InvalidInput(format!(
                            "{prefix}.xor_arity: XOR needs exactly two members, got {}",
                            members.len()
                        )));
                    }
                }
                FilterLogic::Not => {
                    if members.len() != 1 {
                        return Err(CadError::InvalidInput(format!(
                            "{prefix}.not_arity: NOT needs exactly one member, got {}",
                            members.len()
                        )));
                    }
                }
            }
            for member in members {
                validate_expression(prefix, member)?;
            }
            Ok(())
        }
    }
}

/// The input of a STRETCH inside a **crossing polygon** (or a crossing lasso).
///
/// `polygon_logical` is the free-form boundary in **canvas logical pixels**, the
/// same convention as `CommandPayload::Stretch`'s `rect_logical`; a lasso is the
/// same polygon (the capture records its free-form path as this vertex list), so
/// there is exactly one containment rule. The command maps every vertex through
/// the active viewport's orthographic plan view and tests containment with an
/// even-odd **point-in-polygon** predicate on the XY plane (winding-independent).
/// `base`/`delta` are world points; the effective translation is `delta − base`.
///
/// Capture wiring: this build's `draw_tool` capture state machine only has
/// fixed-arity point runs (`PointArity::Exact`) and no Enter-terminated vertex
/// list, so no draw intent produces this payload yet. Hosts/IPC supply it; the
/// exact capture wiring need is reported with the change rather than half-wired.
#[derive(Debug, Clone, PartialEq)]
pub struct StretchPolygonCommand {
    pub polygon_logical: Vec<[f64; 2]>,
    pub base: Point3,
    pub delta: Point3,
}
