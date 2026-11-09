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
//!   one combined revision. `LAYOUTWIZARD` creates the layout and, when the
//!   optional paper setup is supplied, writes it as one authored
//!   `PLOTSETTINGS` row in the same revision and undo step
//!   (`layoutwizard.paper_applied`).
//! * `VPORTS` creates a declared simple `rows * cols` grid; `VPCLIP RECT` and
//!   `VPSCALE` edit the **first** viewport because this build has no
//!   per-viewport selection model. That is a declared narrowing, not a
//!   fabricated second viewport authority.
//! * `LAYVPI <layer>[,<layer>...]` freezes real per-viewport layer overrides
//!   ([`cad_db::PaperViewport::layer_overrides`]) in every viewport of the
//!   active paper layout, as one revision and one undo step
//!   (`layvpi.frozen`). It never writes the global layer table. Because this
//!   build has no viewport selection, "all viewports of the active layout" is
//!   the declared scope; an already-frozen state is a no-op
//!   (`layvpi.unchanged`, no transaction), a layout without a viewport is
//!   `layvpi.no_viewport`.
//! * `VPMAX` / `VPMIN` set and clear one session-only maximized-viewport marker
//!   naming the active paper layout and its first viewport.
//!
//! # The remaining hard boundary
//!
//! Polygonal `VPCLIP` is not modeled. Imported `VPLAYER` data is not parsed:
//! imported viewports start with no overrides and stay `Partial` when the source
//! carried per-viewport layer state. Page setup is written by the typed
//! `PAGESETUP`/`PSETUPIN` forms in [`crate::command_id::extended`]; `MVIEW`
//! appends one rectangular paper viewport and is documented there.
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
