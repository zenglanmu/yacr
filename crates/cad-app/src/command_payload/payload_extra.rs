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
