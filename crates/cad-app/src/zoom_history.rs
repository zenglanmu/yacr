//! The bounded **per-viewport** view history behind `ZOOM PREVIOUS`, AutoCAD's
//! "restore the previous view".
//!
//! A view is session state ([`ViewSnapshot`]: the camera, the 2D/3D observation
//! mode and the plan work plane). It never writes the DWG, never bumps the
//! database revision and is never a `cad-history` entry — so `ZOOM PREVIOUS`
//! cannot use the undo stack and needs its own stack of view snapshots, which
//! lives here. One stack per [`Viewport`], because two viewports frame two
//! different cameras and "the previous view" is a statement about one of them.
//!
//! ## What is recorded, and when
//! Exactly one rule: **before** a **discrete** view change, the pre-change view
//! is pushed. The discrete change sites are the ones the user names a view with:
//!
//! * `FIT` / `ZOOM ALL` / `ZOOM EXTENTS` ([`crate::CommandId::FitDrawing`]),
//! * `ZOOM <factor>` and the `ZOOMIN` / `ZOOMOUT` button steps,
//! * `ZOOM WINDOW` ([`crate::CommandPayload::ZoomWindow`]),
//! * `STANDARDVIEW`, `SWITCH2D3D`, `PROJECTION` and `RESETVIEW`.
//!
//! A change that frames nothing (`FIT` of an empty drawing) or that is refused
//! (an out-of-grammar factor, a 3D `ZOOM WINDOW`, a degenerate rectangle) pushes
//! **nothing**: otherwise `ZOOM PREVIOUS` would burn presses on phantom steps
//! that restore the view the user is already looking at.
//!
//! ## Explicit deviation from AutoCAD: discrete views only
//! **Pans, wheel zooms and orbit drags never push a step.** AutoCAD's `ZOOM
//! Previous` walks back through the zoom/pan history it accumulated
//! interactively; this build records only the *discrete* view commands listed
//! above. Continuous navigation (a middle-drag pan, a wheel notch, a 3D orbit
//! drag) is one command per pointer event, so recording it would turn a single
//! gesture into hundreds of steps and `ZOOM PREVIOUS` into "undo one pixel of
//! my drag". This is a deliberate divergence, not parity: an interactive pan
//! does not become restorable, and the history may therefore be older than
//! AutoCAD's would be.
//!
//! ## Bound
//! [`ZOOM_HISTORY_CAP`] = **16** snapshots, the same bound and the same
//! ring-buffer rule as [`crate::LAYER_OVERRIDE_HISTORY_CAP`]: one snapshot is a
//! copyable camera + a mode tag, so the memory is fixed, and when the stack is
//! full the **oldest** entry is dropped. There is no persistence: a new
//! document / session rebuild starts with an empty history.

use std::collections::VecDeque;

use cad_domain::WorkPlane;

use crate::camera::Camera;
use crate::{ViewMode2d3d, Viewport};

/// How many previous views `ZOOM PREVIOUS` can walk back through.
///
/// See the module docs for why the history is bounded and why the oldest entry is
/// the one dropped.
pub const ZOOM_HISTORY_CAP: usize = 16;

/// One restorable view: exactly what a discrete view change replaces.
///
/// `camera` and `view_mode` are restored verbatim (the mode carries the saved
/// 2D camera a 3D excursion must return to, so restoring the camera alone would
/// leave a 3D view that claims to be 2D). `work_plane` is carried because
/// `RESETVIEW` also resets the plan work plane; without it a `ZOOM PREVIOUS`
/// after a reset would restore the camera of a different plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSnapshot {
    pub camera: Camera,
    pub view_mode: ViewMode2d3d,
    pub work_plane: WorkPlane,
}

impl ViewSnapshot {
    /// The view a viewport is showing right now.
    pub fn capture(viewport: &Viewport) -> Self {
        ViewSnapshot {
            camera: viewport.camera,
            view_mode: viewport.view_mode,
            work_plane: viewport.work_plane,
        }
    }

    /// Apply this view back onto `viewport`.
    ///
    /// Pure assignment of the three recorded values: no derivation, no clamp and
    /// no re-framing, so the restored view is bit-for-bit the recorded one.
    pub fn restore(&self, viewport: &mut Viewport) {
        viewport.camera = self.camera;
        viewport.view_mode = self.view_mode;
        viewport.work_plane = self.work_plane;
    }
}

/// A bounded, oldest-dropped stack of view snapshots for `ZOOM PREVIOUS`.
///
/// The type is a plain value: no database, no history entry, no I/O. It is
/// deliberately the only owner of the snapshot order, so the cap rule exists
/// once.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ZoomHistory {
    snapshots: VecDeque<ViewSnapshot>,
}

impl ZoomHistory {
    /// An empty history (no previous view to restore).
    pub fn new() -> Self {
        Self::default()
    }

    /// The maximum number of snapshots retained.
    pub fn cap(&self) -> usize {
        ZOOM_HISTORY_CAP
    }

    /// How many snapshots are currently retained.
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Whether there is no snapshot to restore.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Record one pre-change view, dropping the oldest entry when full.
    ///
    /// The caller pushes only when a discrete view change really happens (see
    /// the module docs), so no phantom steps are recorded.
    pub fn push(&mut self, snapshot: ViewSnapshot) {
        if self.snapshots.len() >= ZOOM_HISTORY_CAP {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
    }

    /// Remove and return the **newest** snapshot, or `None` when the history is
    /// empty (a `ZOOM PREVIOUS` with nothing to restore).
    pub fn pop(&mut self) -> Option<ViewSnapshot> {
        self.snapshots.pop_back()
    }

    /// The retained snapshots, oldest first (read-only; for contracts).
    pub fn snapshots(&self) -> impl Iterator<Item = &ViewSnapshot> + '_ {
        self.snapshots.iter()
    }
}

impl Viewport {
    /// Record the viewport's current view as one `ZOOM PREVIOUS` step.
    ///
    /// Called by the command layer **before** a discrete view change; see the
    /// module docs for the push rule.
    pub(crate) fn push_view_snapshot(&mut self) {
        let snapshot = ViewSnapshot::capture(self);
        self.zoom_history.push(snapshot);
    }

    /// The number of `ZOOM PREVIOUS` steps recorded for this viewport.
    ///
    /// Session state only; reading it never mutates anything.
    pub fn zoom_history_len(&self) -> usize {
        self.zoom_history.len()
    }

    /// Drop the newest `ZOOM PREVIOUS` step without restoring it.
    ///
    /// The **one** caller is the host's continuous wheel zoom
    /// (`HostCore`'s `Scroll` handling): a wheel notch is the same `ZoomAt`
    /// command as the discrete `ZOOMIN`/`ZOOMOUT` steps, and the application
    /// cannot tell them apart, so the host states the divergence where it maps a
    /// scroll to a zoom — it discards exactly the one step that command recorded.
    /// A wheel notch is therefore never restorable by `ZOOM PREVIOUS` (see the
    /// module docs' declared divergence). Returns whether a step was dropped.
    pub fn discard_last_view_snapshot(&mut self) -> bool {
        self.zoom_history.pop().is_some()
    }
}
