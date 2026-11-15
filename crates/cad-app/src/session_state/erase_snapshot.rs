//! The `OOPS` snapshot types and their session accessors.
//!
//! Split out of `session_state.rs` to keep that module inside the code-size
//! limit. The snapshot is session state: a new `SessionState` is built for every
//! document (`HostController`), and OOPS additionally refuses a snapshot whose
//! document is not the command's document.

use cad_domain::{DocumentId, EntityId};

use super::SessionState;

/// The entities removed by the **last ERASE** (or by `BLOCK`), captured so `OOPS`
/// can restore them.
///
/// This is session state, never database state: a new `SessionState` is built
/// for every document (`HostController`), and OOPS additionally refuses a
/// snapshot whose `document` is not the command's document, so a restore can
/// never cross documents. The snapshot is written by ERASE **after** its delete
/// transaction commits and by `BLOCK` after its members move into the definition;
/// `CUTCLIP`/`CUTBASE` deliberately do not write it (their delete is a cut, not
/// an ERASE), and `WBLOCK` has no delete path in this build.
#[derive(Debug, Clone)]
pub struct EraseSnapshot {
    /// The document the entities were erased from.
    pub document: DocumentId,
    /// The erased rows plus the display state the delete pruned.
    pub entries: Vec<EraseEntry>,
    /// The ids the **last OOPS** restored from this snapshot.
    ///
    /// OOPS keeps its snapshot instead of consuming it: while every id here is
    /// still in the drawing the restore is still in place, so a second OOPS is a
    /// named `oops.already_restored` no-op; once they are gone (an UNDO of the
    /// restore, or a later ERASE of the copies — which replaces this snapshot
    /// anyway) OOPS restores the same set again. Empty before the first restore.
    pub restored: Vec<EntityId>,
}

/// One erased entity and the display state that belonged to it.
#[derive(Debug, Clone)]
pub struct EraseEntry {
    pub entity: cad_db::DbEntity,
    pub display: cad_db::EntityDisplayState,
}

impl SessionState {}
