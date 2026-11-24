//! One entry in the session-only in-process clipboard.

/// One entity captured into the in-process clipboard: the row plus the
/// per-entity render/visibility state the row does **not** carry.
#[derive(Debug, Clone)]
pub struct ClipboardEntry {
    pub entity: cad_db::DbEntity,
    pub display: cad_db::EntityDisplayState,
}
