//! The `OOPS` snapshot types and their session accessors.
//!
//! Split out of `session_state.rs` to keep that module inside the code-size
//! limit. The snapshot is session state: a new `SessionState` is built for every
//! document (`HostController`), and OOPS additionally refuses a snapshot whose
//! document is not the command's document.

use cad_domain::DocumentId;

use super::SessionState;

/// The entities removed by the **last ERASE**, captured so `OOPS` can restore
/// them.
///
/// This is session state, never database state: a new `SessionState` is built
/// for every document (`HostController`), and OOPS additionally refuses a
/// snapshot whose `document` is not the command's document, so a restore can
/// never cross documents. The snapshot is written by ERASE **after** its delete
/// transaction commits and consumed by the first OOPS; `CUTCLIP`/`CUTBASE`
/// deliberately do not write it (their delete is a cut, not an ERASE).
#[derive(Debug, Clone)]
pub struct EraseSnapshot {
    /// The document the entities were erased from.
    pub document: DocumentId,
    /// The erased rows plus the display state the delete pruned.
    pub entries: Vec<EraseEntry>,
}

/// One erased entity and the display state that belonged to it.
#[derive(Debug, Clone)]
pub struct EraseEntry {
    pub entity: cad_db::DbEntity,
    pub display: cad_db::EntityDisplayState,
}

impl SessionState {}
