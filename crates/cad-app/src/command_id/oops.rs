//! Long-form notes for [`crate::CommandId::Oops`] (`OOPS`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant keeps a compact doc and links here. The implementation is
//! `app_drawing::modify_extra::oops_restore`.
//!
//! ## What is restored
//!
//! The session keeps an `session_state::EraseSnapshot` written by ERASE
//! **after** its delete transaction commits: the source document plus, for every
//! erased row, the full `DbEntity` and its `cad_db::EntityDisplayState` (render
//! attributes and visibility membership). `OOPS` reinserts every snapshot entity
//! in **one transaction / one undo step**, then consumes the snapshot.
//!
//! **Declared v1 boundary**: AutoCAD also restores objects removed by
//! `BLOCK`/`WBLOCK`; this build records **ERASE only** (not `BLOCK`, not
//! `CUTCLIP`/`CUTBASE`). A second `OOPS` after the snapshot was consumed is a
//! no-op `oops.nothing` (no error, no write).
//!
//! ## Identity and undo
//!
//! Restored entities receive **fresh entity ids**: the database allocator never
//! reuses an id, so an OOPS restore is a new copy, not the same row back (as in
//! AutoCAD). The row keeps its layer, space, draw order and type key; the
//! import-only source handle is cleared. The undo patch records `before: None`,
//! `after: the stored row` and the restored display state, so UNDO removes the
//! restored copies. **UNDO of OOPS does not recreate the snapshot** — a second
//! OOPS still reports `oops.nothing`; REDO re-inserts the copies.
//!
//! ## Refusals
//!
//! * a supplied payload is `oops.payload` (OOPS is selection-free and
//!   payload-free: it restores the last ERASE, never a selection);
//! * a snapshot from another document is `oops.document` (and such a snapshot is
//!   never stored by `SessionState::set_last_erase` in the first place).
//!
//! Work-only.
