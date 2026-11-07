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
