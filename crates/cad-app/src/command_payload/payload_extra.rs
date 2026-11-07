//! Payload structs split out of `command_payload.rs` to keep that file inside the
//! code-size limit. The enum still owns the variants; this module owns the data.

use cad_domain::{Point3, SelectionRef};

use crate::grips::GripId;

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
    /// Apply a saved snapshot (layer-table write; not undoable).
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
    /// `DIMTEDIT MOVE|ROTATE|RESET`.
    TextEdit(DimTextEditCommand),
    /// `DIMEDIT TEXT <override>|RESET`.
    Edit(DimEditCommand),
    /// `DIMOVERRIDE <variable> <value>`.
    Override(DimOverrideCommand),
    /// `DIMJOGLINE ADD <x> <y>|REMOVE`.
    JogLine(DimJogLineCommand),
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DimTextEditCommand {
    /// Move the measurement text to this drawing-plane X/Y (Z stays on the
    /// dimension's own plane).
    Move { x: f64, y: f64 },
    /// Set the text rotation to these degrees (no normalisation; `0` returns to
    /// the derived rotation).
    Rotate { degrees: f64 },
    /// Restore the derived text placement/rotation (the group 11/53 sentinels).
    Reset,
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
