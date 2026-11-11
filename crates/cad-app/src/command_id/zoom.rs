//! Long-form notes for [`crate::CommandId::Zoom`].
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant itself keeps a compact doc and links here. Nothing here is
//! implemented in this module — the camera work lives in `app_zoom.rs` and the
//! host capture in `cad-host`'s pointer routing.
//!
//! ## One command, five payload forms
//!
//! `Zoom` deliberately carries **no** payload of its own shape: the payload
//! selects the form, and no new [`crate::CommandId`] exists for any of them, so
//! the command vocabulary is unchanged.
//!
//! | payload | behaviour | anchor |
//! | --- | --- | --- |
//! | [`crate::CommandPayload::ZoomAt`] | a positive factor | the logical-pixel cursor (wheel, `ZOOMIN`, `ZOOMOUT`, `ZOOM <factor>`, `ZOOM <n>X`) |
//! | [`crate::CommandPayload::Points`] | the legacy factor-only form | the viewport centre |
//! | [`crate::CommandPayload::ZoomWindow`] | `ZOOM WINDOW`: fill the viewport **exactly** with the picked rectangle | the rectangle's own centre |
//! | [`crate::CommandPayload::ZoomPrevious`] | `ZOOM PREVIOUS`: restore this viewport's last **discrete** view | n/a |
//! | [`crate::CommandPayload::ZoomObject`] | `ZOOM OBJECT`: frame the current **selection** with the `FIT` margin | the selection bounds' centre |
//!
//! ## `ZOOM OBJECT`
//!
//! * **The selection is the input.** There is no payload point and no
//!   interactive object pick; the typed `ZOOM OBJECT` line dispatches one
//!   [`crate::CommandPayload::ZoomObject`] and the application reads
//!   `session.selection`.
//! * **Model space** resolves each ref through the ordinary `INSERT`-expanded
//!   pick items, so a block member is framed at its accumulated transform and a
//!   selected `INSERT` frames every member it owns. **Paper space** only accepts
//!   directly addressable layout entities: an instance-path member has no
//!   independent paper object, so its bounds are never invented.
//! * **2D orthographic only** (`zoom.object.requires_2d`), exactly like
//!   `ZOOM WINDOW`. The frame uses the one rect → camera helper with the
//!   existing `FIT` margin (`1.05`), not a second framing rule.
//! * **Named refusals change nothing**: an empty selection
//!   (`zoom.object.empty_selection`), a foreign/unknown ref
//!   (`zoom.object.foreign_ref`, `zoom.object.target_missing`), a non-addressable
//!   ref (`zoom.object.not_addressable`) and no finite bounds
//!   (`zoom.object.no_bounds`) are all explicit before the camera moves. A real
//!   object zoom records one `ZOOM PREVIOUS` step.
//!
//! ## `ZOOM <n>X` and `ZOOM <ratio>XP`
//!
//! * `nX` (`2X`, `0.5X`) is the **relative factor** spelling of the ordinary
//!   cursor-less zoom: exactly [`crate::CommandPayload::ZoomAt`] anchored at the
//!   viewport centre, so `> 1` zooms in. A non-positive/non-finite factor is
//!   `zoom.factor.invalid`.
//! * `nXP` (`2XP` = `VPSCALE 1:2`, `1/50XP` = `VPSCALE 1:50`) is a **drawing
//!   write**: it is routed through the already Work-only
//!   [`crate::CommandId::VpScale`] path, never through the Viewer-legal `ZOOM`
//!   command. It requires an active paper layout
//!   (`zoom.xp.requires_paper`), records one transaction/undo step, and reports
//!   the same `vpscale.applied` diagnostics. Viewer mode refuses it through the
//!   normal Work-only authorization, not a second check.
//!
//! ## `ZOOM DYNAMIC` is a named gap
//!
//! `ZOOM DYNAMIC` is **NOT IMPLEMENTED**. It is outside the parser grammar and
//! the typed path refuses it with `zoom.dynamic_not_implemented`; it is never
//! approximated by another zoom form. Do not read this table as AutoCAD parity.
//!
//! ## `ZOOM WINDOW`
//!
//! * **Two-corner capture.** The rectangle is a pair of opposite corners in
//!   **canvas logical pixels**, produced by the host's existing box-gesture path
//!   and armed by the typed `ZOOM WINDOW` line. Logical pixels are used
//!   precisely so no drafting constraint (object snap, grid snap, `ORTHO`) can
//!   bend the window the user dragged: a window is a screen rectangle, not a
//!   point entry.
//! * **2D orthographic only.** A window says nothing about a perspective
//!   observation, so a 3D view refuses **by name** (`zoom.window.requires_2d`)
//!   instead of faking a perspective fit.
//! * **Exact fit.** The rectangle fills the viewport with margin `1.0`, unlike
//!   `FIT`'s `1.05`. That is our declared choice (`app_zoom::WINDOW_MARGIN`):
//!   AutoCAD also fills the window, but its residual padding is undocumented, so
//!   claiming parity on a few percent would be a guess. Both margins come from
//!   the one rect → (centre, scale) helper, `max(ex / w, ey / h) * margin`.
//! * **Refusals before any camera change**: a non-finite corner
//!   (`zoom.window.invalid_rect`) and a rectangle with no extent on either axis
//!   (`zoom.window.degenerate_rect`) are `InvalidInput`; the host additionally
//!   drops a drag under 2 logical pixels instead of dispatching it.
//!
//! ## `ZOOM PREVIOUS`
//!
//! * **Per-viewport, bounded** history (`crate::zoom_history`, cap
//!   [`crate::ZOOM_HISTORY_CAP`] = 16, oldest dropped), holding the camera, the
//!   2D/3D mode and the plan work plane, so the restore is exact.
//! * **Discrete views only.** `FIT`/`ZOOM ALL`, a zoom factor,
//!   `ZOOMIN`/`ZOOMOUT`, `ZOOM WINDOW`, `ZOOM OBJECT`, `STANDARDVIEW`,
//!   `SWITCH2D3D`, `PROJECTION` and `RESETVIEW` record a snapshot **before**
//!   they change the view. Pans, wheel zooms and orbit drags record nothing — an
//!   explicit divergence from AutoCAD, documented in `crate::zoom_history`, not
//!   parity.
//! * **An empty stack is not an error**: the command changes nothing and reports
//!   the `zoom.previous.empty` diagnostic.
//!
//! ## What the view forms do not do
//!
//! The view-only forms (`FIT`, ALL/EXTENTS, factors/`nX`, WINDOW, PREVIOUS,
//! OBJECT) write no database row, bump no revision, add no `cad-history` entry
//! and do not bump `session.generation` (that stamp belongs to open/task work,
//! and a plain zoom is not one). They are session view state, so Viewer mode
//! allows them. The one exception in this family is `nXP`, which is a
//! **`VPSCALE` drawing write** and therefore Work-only (see above).
