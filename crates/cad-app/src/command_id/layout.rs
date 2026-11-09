//! The 2026-11-07 layout / viewport batch.
//!
//! These selectors cover AutoCAD's paper-space surface. The table/viewport
//! **writes** are real now that `cad-db` owns sanctioned post-import layout
//! writes: `LAYOUT NEW|RENAME|DELETE`, `LAYOUTMERGE`, `LAYOUTWIZARD`,
//! `VPORTS`/`-VPORTS`, `VPCLIP RECT` and `VPSCALE` each apply through one
//! validated database write and record exactly one undo step.
//!
//! # What really works
//!
//! * `LAYOUT LIST` reads the authoritative layout descriptors.
//! * `LAYOUT SET <name>` switches the session's active space through the one
//!   `SwitchSpace` path (session state only: no database write, no undo entry).
//! * `LAYOUT NEW` / `LAYOUT RENAME` / `LAYOUT DELETE` write the layout table;
//!   delete also cascades the plot settings and leaves the session on a valid
//!   space when the deleted layout was active.
//! * `LAYOUTMERGE` moves every source paper entity and deletes the source row in
//!   one combined revision. `LAYOUTWIZARD` creates the layout but still does
//!   **not** model page setup (`layoutwizard.paper_not_applied`).
//! * `VPORTS` creates a declared simple `rows * cols` grid; `VPCLIP RECT` and
//!   `VPSCALE` edit the **first** viewport because this build has no
//!   per-viewport selection model. That is a declared narrowing, not a
//!   fabricated second viewport authority.
//! * `VPMAX` / `VPMIN` set and clear one session-only maximized-viewport marker
//!   naming the active paper layout and its first viewport.
//!
//! # The remaining hard boundary
//!
//! `LAYVPI` stays a named `Unsupported` refusal: this build has no per-viewport
//! layer state (`layvpi.viewport_layer_state_unsupported`), and it never freezes
//! the layers globally. Polygonal `VPCLIP` and page setup are likewise not
//! modeled.
//!
//! `-VPORTS` is a spelling alias of `VPORTS`, not a second command: it
//! normalizes to the same [`crate::CommandId::Vports`] and the same handler.
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
