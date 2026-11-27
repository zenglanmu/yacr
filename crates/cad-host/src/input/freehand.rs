//! The FREEHAND stroke gesture (SketchUp's Freehand tool).
//!
//! A freehand stroke is a **drag**, not clicks: the press starts the trace, every
//! move appends a decimated sample, and the release hands the whole path to the
//! active FREEHAND capture as raw world points. It reuses the single pointer
//! pipeline and its arbitration ([`super::pointer`]) — the press claims the
//! contact before any box/pan decision, a cancel or multi-touch takeover drops it
//! — so there is no second pointer path, and an aborted stroke writes nothing.
//!
//! The sampled type is the lasso's [`LassoGesture`] (same decimation and bound),
//! because both are freehand traces in canvas logical pixels. Unlike the lasso
//! this path is **not** a closed ring: the release feeds the points to the capture
//! and confirms the existing `SKETCH` command, so one stroke is one open polyline
//! in one transaction / one undo step.
//!
//! Split out of `pointer.rs` to keep that module inside the code-size limit; the
//! methods stay inherent on [`HostCore`].

use super::*;

/// Minimum stroke extent, in logical pixels, on **either** axis.
///
/// A "freehand stroke" that never grew past a click is a slip, not a stroke: the
/// capture keeps waiting and the named status says so, so an accidental click in
/// the tool cannot commit a degenerate two-point line.
const FREEHAND_MIN_EXTENT_LOGICAL_PX: f64 = 2.0;

impl HostCore {
    /// Whether the active draw capture is the FREEHAND tool.
    ///
    /// Read from the capture's own kind, never inferred from the pointer: only
    /// the freehand tool claims the drag, so every other tool keeps the ordinary
    /// box/pan arbitration.
    pub(super) fn freehand_waiting(&self) -> bool {
        self.controller.borrow().session.draw_kind() == Some(cad_app::DrawToolKind::Freehand)
    }

    /// Append one pointer position to the live freehand stroke.
    pub(super) fn track_freehand(&self, point: [f64; 2]) {
        if let Some(gesture) = self.navigation.freehand_gesture.borrow_mut().as_mut() {
            gesture.push_sample(point);
        }
    }

    /// Drop a live stroke without committing it.
    ///
    /// Every non-commit terminal (cancel, multi-touch takeover, gesture clear)
    /// uses this: the capture was never fed, so the tool stays active and nothing
    /// is written.
    pub(super) fn clear_freehand_gesture(&self) {
        *self.navigation.freehand_gesture.borrow_mut() = None;
    }

    /// The live stroke as the existing transient overlay input, or `None`.
    ///
    /// The same [`BoxSelectionPreview`] segment list the box/lasso previews use;
    /// a freehand stroke is drawn solid (the WINDOW colour) because it is the
    /// geometry being traced, not a crossing window. A path with no segment yet
    /// yields `None` (an explicit "nothing to draw", never a fabricated band).
    pub(super) fn freehand_preview(&self) -> Option<BoxSelectionPreview> {
        let gesture = self.navigation.freehand_gesture.borrow();
        let gesture = gesture.as_ref()?;
        let segments = {
            let c = self.controller.borrow();
            let viewport = c.application.workspace.viewports.get(&c.viewport_id)?;
            let size = viewport.logical_size;
            let mut segments = Vec::with_capacity(gesture.path.len().saturating_sub(1));
            for pair in gesture.path.windows(2) {
                if let (Some(from), Some(to)) = (
                    viewport.screen_to_world(pair[0], size),
                    viewport.screen_to_world(pair[1], size),
                ) {
                    segments.push([from, to]);
                }
            }
            segments
        };
        if segments.is_empty() {
            return None;
        }
        Some(BoxSelectionPreview {
            segments,
            crossing: false,
        })
    }

    /// Hand a released stroke to the active FREEHAND capture and confirm it.
    ///
    /// The samples are canvas **logical** pixels, so they are projected through
    /// the active viewport's own `screen_to_world` — the same mapping the lasso
    /// and the box preview use. A 3D/perspective view has no unique plan point and
    /// is refused by name (the tool stays active); a stroke thinner than
    /// [`FREEHAND_MIN_EXTENT_LOGICAL_PX`] is a click and is refused the same way.
    /// The capture's own `push_point` bound (and the command's two-point minimum)
    /// are the final authority; the command layer validates the geometry.
    pub(super) fn finish_freehand_gesture(&self, gesture: LassoGesture) -> CadResult<()> {
        let (min, max) = gesture.path.iter().fold(
            ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
            |(min, max), p| {
                (
                    [min[0].min(p[0]), min[1].min(p[1])],
                    [max[0].max(p[0]), max[1].max(p[1])],
                )
            },
        );
        let width = max[0] - min[0];
        let height = max[1] - min[1];
        if !(width.is_finite() && height.is_finite())
            || width < FREEHAND_MIN_EXTENT_LOGICAL_PX
            || height < FREEHAND_MIN_EXTENT_LOGICAL_PX
        {
            let _ = self.set_external_status(
                "sketch.too_few_points: the drag did not trace a stroke (it is thinner than the \
                 minimum extent); the freehand tool still waits for one",
            );
            return Ok(());
        }
        let points = {
            let c = self.controller.borrow();
            let viewport = c
                .application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .ok_or(CadError::Cancelled)?;
            let size = viewport.logical_size;
            let mut points = Vec::with_capacity(gesture.path.len());
            for logical in &gesture.path {
                match viewport.screen_to_world(*logical, size) {
                    Some(world) => points.push(world),
                    None => {
                        let _ = self.set_external_status(
                            "sketch.requires_plan_view: a freehand stroke needs the 2D plan \
                             view; nothing was captured",
                        );
                        return Ok(());
                    }
                }
            }
            points
        };
        {
            let mut controller = self.controller.borrow_mut();
            for point in points {
                controller.capture_freehand_point(point)?;
            }
        }
        // The ordinary confirm funnel: one `SKETCH` command, one transaction and
        // one undo step; a command failure preserves the capture.
        self.confirm_tool()?;
        Ok(())
    }
}
