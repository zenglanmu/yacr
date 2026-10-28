//! Long-form notes for [`crate::CommandId::Zoom`].
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the variant itself keeps a compact doc and links here. Nothing here is
//! implemented in this module — the camera work lives in `app_zoom.rs` and the
//! host capture in `cad-host`'s pointer routing.
//!
//! ## One command, four payload forms
//!
//! `Zoom` deliberately carries **no** payload of its own shape: the payload
//! selects the form, and no new [`crate::CommandId`] exists for any of them, so
//! the command vocabulary is unchanged.
//!
//! | payload | behaviour | anchor |
//! | --- | --- | --- |
//! | [`crate::CommandPayload::ZoomAt`] | a positive factor | the logical-pixel cursor (wheel, `ZOOMIN`, `ZOOMOUT`, `ZOOM <factor>`) |
//! | [`crate::CommandPayload::Points`] | the legacy factor-only form | the viewport centre |
//! | [`crate::CommandPayload::ZoomWindow`] | `ZOOM WINDOW`: fill the viewport **exactly** with the picked rectangle | the rectangle's own centre |
//! | [`crate::CommandPayload::ZoomPrevious`] | `ZOOM PREVIOUS`: restore this viewport's last **discrete** view | n/a |
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
//!   `ZOOMIN`/`ZOOMOUT`, `ZOOM WINDOW`, `STANDARDVIEW`, `SWITCH2D3D`,
//!   `PROJECTION` and `RESETVIEW` record a snapshot **before** they change the
//!   view. Pans, wheel zooms and orbit drags record nothing — an explicit
//!   divergence from AutoCAD, documented in `crate::zoom_history`, not parity.
//! * **An empty stack is not an error**: the command changes nothing and reports
//!   the `zoom.previous.empty` diagnostic.
//!
//! ## What neither form does
//!
//! No database write, no revision bump, no `cad-history` entry and no
//! `session.generation` bump (that stamp belongs to open/task work, and a plain
//! zoom is not one). Both are view-only, so Viewer mode allows them.
