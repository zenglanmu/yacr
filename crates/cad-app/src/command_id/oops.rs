//! Long-form notes for [`crate::CommandId::Oops`] (`OOPS`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant keeps a compact doc and links here. The implementation is
//! `app_drawing::modify_extra::oops_restore`.
//!
//! ## What is restored
//!
//! The session keeps an `session_state::EraseSnapshot` written by ERASE
//! **after** its delete transaction commits (`BLOCK` writes it too, after its
//! members move into the new definition): the source document plus, for every
//! removed row, the full `DbEntity` and its `cad_db::EntityDisplayState` (render
//! attributes and visibility membership). OOPS reinserts **every** snapshot
//! entity — the whole last ERASE set, not one row — in **one transaction / one
//! undo step**. `WBLOCK` has no delete path in this build, so there is nothing to
//! record for it; `CUTCLIP`/`CUTBASE` deliberately do not record a snapshot
//! (their delete is a cut, not a removal the user expects OOPS to undo).
//!
//! ## What a second OOPS does
//!
//! The snapshot is **kept**, not consumed, and stores the ids the last restore
//! inserted. So:
//!
//! * immediately after a successful OOPS, a second OOPS is `oops.already_restored`
//!   — a named no-op with no revision bump, no transaction and no undo entry;
//! * after UNDO of the restore the restored ids are gone, so OOPS restores the
//!   same set again (fresh ids, one new undo step). Undo therefore puts the
//!   drawing and OOPS back in agreement instead of silently disarming the command
//!   (the pre-promotion behaviour: undo removed the copies and the snapshot was
//!   gone too, so the next OOPS reported `oops.nothing`).
//!
//! A later ERASE (or BLOCK) replaces the snapshot, exactly like AutoCAD's "last
//! erase" moving forward.
//!
//! ## Identity and undo
//!
//! Restored entities receive **fresh entity ids**: the database allocator never
//! reuses an id, so an OOPS restore is a new copy, not the same row back (as in
//! AutoCAD). The row keeps its layer, space, draw order and type key; the
//! import-only source handle is cleared. The undo patch records `before: None`,
//! `after: the stored row` and the restored display state, so UNDO removes the
//! restored copies and REDO re-inserts them.
//!
//! ## Refusals
//!
//! * a supplied payload is `oops.payload` (OOPS is selection-free and
//!   payload-free: it restores the last ERASE/BLOCK, never a selection);
//! * a snapshot from another document is `oops.document` (and such a snapshot is
//!   never stored by `SessionState::set_last_erase` in the first place).
//!
//! Work-only.
