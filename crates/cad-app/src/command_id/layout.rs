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
//!   one combined revision. `LAYOUTWIZARD` creates the layout, its optional
//!   authored `PLOTSETTINGS` row (`layoutwizard.paper_applied`) and its optional
//!   equal-cell viewport arrangement (`layoutwizard.arrangement_applied`) in the
//!   same revision and undo step. The arrangement set is bounded
//!   ([`crate::WizardArrangement`]); a name outside it is refused by name
//!   (`layoutwizard.unknown_arrangement`). The wizard's printer/plot-device,
//!   plot-style and title-block steps have no implementation: they are reported
//!   with their own stable code (`layoutwizard.plot_device_not_implemented`,
//!   `layoutwizard.plot_style_not_implemented`,
//!   `layoutwizard.title_block_not_implemented`) and are never applied.
//! * `VPORTS` creates a declared simple `rows * cols` grid.
//! * `VPCLIP RECT`/`VPCLIP` polygon and `VPSCALE` target the **session
//!   `CVPORT` current viewport** of the active paper layout when their payload
//!   carries no index, and the payload's explicit zero-based index otherwise
//!   (the explicit index always wins). `CVPORT` itself is documented in
//!   [`crate::command_id::extras`]; it is session state, so neither write below
//!   touches it.
//! * `LAYVPI <layer>[,<layer>...]` freezes real per-viewport layer overrides
//!   ([`cad_db::PaperViewport::layer_overrides`]) in every viewport of the
//!   active paper layout **except the current one** (AutoCAD's own default,
//!   read from the same `CVPORT` marker), as one revision and one undo step
//!   (`layvpi.frozen`). It never writes the global layer table. The
//!   explicit-index form (`FreezeViewportLayersIn`) freezes one viewport and
//!   wins over the marker. An already-frozen state is a no-op
//!   (`layvpi.unchanged`, no transaction), a layout without a viewport is
//!   `layvpi.no_viewport`, and a single-viewport layout whose only viewport is
//!   the current one has nothing to exclude (`layvpi.no_other_viewport`) —
//!   never a silent "freeze everywhere".
//! * `VPMAX` / `VPMIN` set and clear one session-only maximized-viewport marker
//!   naming the active paper layout and its first viewport. They are **not**
//!   wired to `CVPORT` in this lane (see the module's note below).
//!
//! # Polygonal viewports
//!
//! `VPCLIP` also writes a polygonal boundary (`LayoutPayload::ClipPolygon`): the
//! validated 3+ point simple polygon replaces the **targeted** viewport's clip
//! and `cad-representation` clips that viewport to it — concave boundaries
//! included, for lines, meshes, images and conics alike. There is no
//! "written but not renderable" residual.
//!
//! # Viewport targeting and the `CVPORT` marker
//!
//! `CVPORT [<n>]` ([`crate::command_id::extras`]) stores the active paper
//! layout's current viewport, one **one-based** position per layout, as pure
//! session state (no database write, revision, history entry or undo step). It is
//! the target of the no-index `LAYVPI`/`VPSCALE`/`VPCLIP` forms and is validated
//! against the layout's viewport count by every reader, so a stale marker refuses
//! (`<command>.current_viewport_out_of_range`) instead of retargeting the first
//! viewport. Switching layouts reports that layout's own marker, and a layout
//! with no viewport reports `cvport.no_viewports` rather than a fabricated `1`.
//!
//! `VPMAX`/`VPMIN` still maximize the layout's **first** viewport
//! (`target_viewport_index` in `app_layout_paper`, which falls back to index `0`
//! and is shared with `MVSETUP`/`PSBSCALE`). Following `CVPORT` there is the
//! semantically right next step (AutoCAD maximizes the current viewport), but it
//! changes a helper outside this batch's authority, so it is **declared, not
//! done**: `VPMAX` remains "first viewport (or the already-maximized one)".
//!
//! # The remaining hard boundary
//!
//! Viewport **holes** are not modeled (one boundary per viewport). Imported
//! `VPLAYER` data is not parsed: imported viewports start with no overrides and
//! stay `Partial` when the source carried per-viewport layer state. Page setup is
//! written by the typed `PAGESETUP`/`PSETUPIN` forms in
//! [`crate::command_id::extended`]; `MVIEW` appends one rectangular **or
//! polygonal** paper viewport and is documented there.
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
