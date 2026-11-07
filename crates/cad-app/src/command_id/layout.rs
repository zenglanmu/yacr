//! The 2026-11-07 layout / viewport batch.
//!
//! These selectors cover the **read** half of AutoCAD's paper-space surface and
//! explicitly refuse every write the database cannot publish.
//!
//! # The hard boundary
//!
//! `cad-db` exposes the layout table and each `PaperViewport` as **read-only**
//! accessors after import ([`cad_db::DrawingDatabase::layout`],
//! [`cad_db::DrawingDatabase::layouts`]). There is no layout insert, rename,
//! delete, page-setup, viewport insert, clip or per-viewport layer method — the
//! builder is the only writer. This build therefore does **not** mutate a clone
//! of the drawing (that would leave the session, renderer and undo history on
//! stale data). Every command that would need such a write returns a stable
//! `Unsupported` refusal naming the missing capability:
//!
//! * `layout.write_unsupported` — `LAYOUT NEW|RENAME|DELETE`;
//! * `layoutmerge.write_unsupported` — entity moves plus layout delete;
//! * `layoutwizard.write_unsupported` — layout creation plus page setup;
//! * `vports.write_unsupported` — `PaperViewport` creation;
//! * `vpclip.viewport_selection_unsupported` — there is no selected-viewport
//!   state and no clip write;
//! * `vpscale.write_unsupported` — no viewport scale write;
//! * `layvpi.viewport_layer_state_unsupported` — no per-viewport layer state.
//!
//! # What really works
//!
//! * `LAYOUT LIST` reads the authoritative layout descriptors.
//! * `LAYOUT SET <name>` switches the session's active space through the one
//!   `SwitchSpace` path (session state only: no database write, no undo entry).
//! * `VPMAX` / `VPMIN` set and clear one session-only maximized-viewport marker.
//!   The marker names the active paper layout and its **first** viewport
//!   (index 0), because this build has no per-viewport selection model; that is
//!   a declared narrowing, not a fabricated second viewport authority. The
//!   marker is published by [`crate::SessionState::maximized_viewport`] for a
//!   host to render; it writes no drawing data.
//!
//! `-VPORTS` is a spelling alias of `VPORTS`, not a second command: it
//! normalizes to the same [`crate::CommandId::Vports`] and the same refusal.
//!
//! # Name grammar
//!
//! Layout names are trimmed; empty, multi-line, longer than 255 characters, or
//! containing an AutoCAD-illegal character (`<>/\":;?*|=` and backtick) is
//! `layout.invalid_name`. Duplicate names compare case-insensitively
//! (`layout.duplicate_name`); resolution is exact first, then a **unique**
//! case-insensitive match (`layout.unknown` / `layout.ambiguous`). The typed
//! `RENAME <old> <new>` and `LAYOUTMERGE <source> <target>` forms split at the
//! first whitespace, so those two *left-hand* names cannot contain a space; the
//! single-name forms (`NEW`/`DELETE`/`SET`/`LAYOUTWIZARD`) take the whole
//! remainder and therefore do accept spaces. This is a grammar boundary of the
//! command line, documented rather than guessed around.

use crate::LayoutPayload;

/// The semantic input a `CommandId` expects: one [`LayoutPayload`].
///
/// The concrete variant is carried by [`crate::CommandPayload::Layout`]; this
/// alias keeps the intent obvious at call sites without adding a second enum.
pub type LayoutCommand = LayoutPayload;
