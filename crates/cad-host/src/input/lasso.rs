//! The STRETCH freehand **lasso** gesture (the polygon/lasso crossing window).
//!
//! A lasso is the one crossing-window capture that is a *drag* rather than clicks:
//! the press starts the trace while the STRETCH capture waits for its ring, every
//! move appends a sample, and the release hands the whole path to the capture as a
//! closed ring ([`cad_app::SessionState::set_draw_window_ring`]). It reuses the
//! single pointer pipeline and its arbitration — the press claims the contact
//! before any box/pan decision, and a cancel or a multi-touch takeover drops it —
//! so there is no second pointer path, and an aborted trace writes nothing.
//!
//! Split out of `pointer.rs` to keep that module inside the code-size limit; the
//! methods stay inherent on [`HostCore`].

use super::*;

/// Minimum size of the closed lasso ring, in logical pixels, on **both** axes.
///
/// A "lasso" that never grew past a click is a slip, not a ring: the capture
/// refuses it by name (`stretch.window.lasso_degenerate`) and nothing is
/// captured, so an accidental click in lasso mode cannot commit an empty window.
const STRETCH_LASSO_MIN_EXTENT_LOGICAL_PX: f64 = 4.0;

impl HostCore {
    /// Whether the active STRETCH capture is waiting for a **freehand lasso** ring.
    ///
    /// Read from the capture's own state ([`cad_app::SessionState::draw_window_step`]),
    /// never inferred from the tool key: only a lasso-mode STRETCH capture with an
    /// open ring claims the drag, so the rectangle window and every other tool keep
    /// the ordinary box/pan arbitration.
    pub(super) fn stretch_lasso_waiting(&self) -> bool {
        matches!(
            self.controller.borrow().session.draw_window_step(),
            Some(cad_app::StretchWindowStep::LassoTrace)
        )
    }

    /// Append one pointer position to the live lasso path.
    pub(super) fn track_lasso(&self, point: [f64; 2]) {
        if let Some(gesture) = self.navigation.lasso_gesture.borrow_mut().as_mut() {
            gesture.push_sample(point);
        }
    }

    /// Drop a live lasso without handing it to the capture.
    ///
    /// One entry for every non-commit terminal — a cancel, a gesture clear, a
    /// multi-touch takeover, a capture/modal pan owning the pointer. Pure state:
    /// the ring was never captured, so the STRETCH capture still waits for its
    /// window and nothing is written.
    pub(super) fn clear_lasso_gesture(&self) {
        *self.navigation.lasso_gesture.borrow_mut() = None;
    }

    /// The live lasso trail as the existing transient overlay input, or `None`.
    ///
    /// The overlay input is the same one the box-selection rubber band uses
    /// ([`BoxSelectionPreview`]): a list of world-space segments plus the
    /// WINDOW/CROSSING colour, tessellated here because this crate is the one that
    /// sees screen pixels. Feeding the trail through it means the user sees the
    /// ring they are tracing without a second preview input, a second renderer
    /// path or a pixel length guessed in the renderer. A lasso is always a
    /// **CROSSING** window, so it is painted with that colour, and a path with no
    /// segment yet yields `None` (an explicit "nothing to draw", never an empty
    /// fabricated band).
    pub(super) fn lasso_preview(&self) -> Option<BoxSelectionPreview> {
        let gesture = self.navigation.lasso_gesture.borrow();
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
            crossing: true,
        })
    }

    /// Hand a released lasso's traced ring to the active STRETCH capture.
    ///
    /// The path is in canvas **logical** pixels (the pointer convention), so it is
    /// projected through the active viewport's own `screen_to_world` — the same
    /// mapping the box preview and the polygon command's logical vertices use, so
    /// there is no second projection. A ring that is thinner than
    /// [`STRETCH_LASSO_MIN_EXTENT_LOGICAL_PX`] on an axis (a click, not a drag) is
    /// **refused by name and stored nowhere**: the capture keeps waiting, exactly
    /// like a cancelled drag. The capture's own ring validation owns the rest
    /// (vertex count, area, non-finite points) and a refusal is surfaced through
    /// the normal status channel.
    pub(super) fn finish_lasso_gesture(&self, lasso: LassoGesture) -> CadResult<()> {
        let (min, max) = lasso.path.iter().fold(
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
            || width < STRETCH_LASSO_MIN_EXTENT_LOGICAL_PX
            || height < STRETCH_LASSO_MIN_EXTENT_LOGICAL_PX
        {
            let _ = self.set_external_status(
                "stretch.window.lasso_degenerate: the drag did not trace a lasso window (it is \
                 thinner than the minimum extent on an axis); the capture still waits for one",
            );
            return Ok(());
        }
        let world = {
            let c = self.controller.borrow();
            let viewport = c
                .application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .ok_or(CadError::Cancelled)?;
            let size = viewport.logical_size;
            let mut points = Vec::with_capacity(lasso.path.len());
            for logical in &lasso.path {
                match viewport.screen_to_world(*logical, size) {
                    Some(world) => points.push(world),
                    // A perspective/3D view has no plan mapping: the capture layer
                    // refuses the ring (STRETCH is a 2D command), and the refusal is
                    // never turned into a fabricated window.
                    None => {
                        let _ = self.set_external_status(
                            "stretch.window.requires_plan_view: a lasso window needs the 2D plan \
                             view; nothing was captured",
                        );
                        return Ok(());
                    }
                }
            }
            points
        };
        match self
            .controller
            .borrow_mut()
            .session
            .set_draw_window_ring(world)
        {
            Ok(()) => {
                self.full_dirty.set(true);
                Ok(())
            }
            Err(error) => {
                let _ = self.set_external_status(crate::error_text(&error));
                Ok(())
            }
        }
    }
}
