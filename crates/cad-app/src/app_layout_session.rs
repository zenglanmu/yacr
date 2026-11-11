//! The session-only half of the layout / viewport batch: `VPMAX` / `VPMIN`.
//!
//! `VPMAX` enters a **maximized layout-viewport editing state** and `VPMIN`
//! leaves it. The state is real session state, not a display flag:
//!
//! 1. The pre-`VPMAX` view (camera, 2D/3D mode, plan work plane) **and** the
//!    active drawing space are captured as a [`ViewSnapshot`] and stored in the
//!    session's bounded named-view store under the reserved name
//!    [`VPMAX_RESTORE_VIEW`]. That store is the only session-owned slot that
//!    can hold a camera plus a space (`SessionState` has no per-command camera
//!    field, and this lane may not add one), so the restore state is a real
//!    snapshot rather than a second boolean.
//! 2. The viewport camera is moved to the layout viewport's own model window —
//!    the paper clip rectangle mapped through the viewport's exact
//!    paper→model transform — so the maximized state shows the model extent the
//!    viewport displays, filling the display instead of the sheet.
//! 3. The session marker ([`crate::LayoutViewportRef`]) records which layout
//!    viewport is being edited, which is what a host reads to scope in-place
//!    interaction.
//!
//! `VPMIN` restores the captured camera, mode, work plane and space **exactly**
//! (bit-for-bit, via [`ViewSnapshot::restore`]), consumes the reserved snapshot
//! and clears the marker. Nothing here writes the drawing: no revision bump, no
//! `ChangeSet`, no `cad-history` entry, no global layer change.
//!
//! ## Declared boundaries
//!
//! * The reserved name `*VPMAX-RESTORE*` is visible in `VIEW LIST` while a
//!   layout viewport is maximized, because it lives in the same store. That is
//!   deliberate: the restore state is not hidden from the user. `VIEW DELETE`
//!   of that name destroys the restore state, and `VPMIN` then refuses by name
//!   (`vpmin.restore_state_missing`) instead of inventing a camera. The store is
//!   bounded (32 views), so a full store evicts its oldest view to make room —
//!   the existing `VIEW` rule, not a second policy.
//! * Only one layout viewport can be maximized at a time. `VPMAX` while another
//!   layout's viewport is maximized is refused (`vpmax.already_maximized`);
//!   `VPMIN` first.
//! * `VPMAX` / `VPMIN` are **not** `ZOOM PREVIOUS` steps: the maximized state
//!   owns its own restore snapshot, so a `ZOOM PREVIOUS` can never leave the
//!   marker and the camera disagreeing.
//! * No host renders the maximized state differently yet: the camera, the
//!   marker and the restore snapshot are real, the pixel behaviour is not
//!   verified.

use super::*;

use cad_representation::{viewport_transform, ViewportState};

use crate::app_layout::paper_writes::unchanged_outcome;
use crate::app_layout::{layout_viewports, paper_writes::target_viewport_index};
use crate::camera::{Camera, Projection, MIN_ORTHO_SCALE};
use crate::ViewSnapshot;

/// The reserved named-view slot that holds the pre-`VPMAX` view.
///
/// It is written by `VPMAX` and consumed by `VPMIN` (and removed when a
/// `LAYOUT DELETE`/`LAYOUTMERGE` invalidates the maximized state).
pub(crate) const VPMAX_RESTORE_VIEW: &str = "*VPMAX-RESTORE*";

impl Application {
    /// `VPMAX`: enter the maximized editing state of the active layout viewport.
    pub(crate) fn vp_max(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "vpmax.invalid_payload: VPMAX takes no payload".into(),
            ));
        }
        let layout_id = match &session.active_space {
            SpaceId::Paper(layout_id) => *layout_id,
            _ => {
                return Err(CadError::Unsupported(
                    "vpmax.paper_space_required: VPMAX needs an active paper layout".into(),
                ))
            }
        };
        let document_id = command.document;
        let viewports = layout_viewports(self.drawing(document_id)?, layout_id)?;
        let index = target_viewport_index(session, layout_id, None, &viewports, "vpmax")?;
        let target = LayoutViewportRef {
            layout: layout_id,
            index,
        };
        if session.maximized_viewport() == Some(target) {
            return Ok(unchanged_outcome(
                "vpmax.already_maximized",
                format!(
                    "VPMAX: layout {} viewport {index} is already maximized; VPMIN restores the \
                     pre-maximize view",
                    layout_id.0
                ),
            ));
        }
        if let Some(other) = session.maximized_viewport() {
            return Err(CadError::Unsupported(format!(
                "vpmax.already_maximized: layout {} viewport {} is already maximized; run VPMIN \
                 before maximizing another layout viewport",
                other.layout.0, other.index
            )));
        }
        // The model window the viewport shows, from the exact paper→model
        // transform the display uses. An unsupported viewport is refused by
        // name rather than framed with a guessed ratio.
        let transform = match viewport_transform(&viewports[index]) {
            ViewportState::Supported(transform) => *transform,
            ViewportState::Unsupported(reason) => {
                return Err(CadError::Unsupported(format!(
                    "vpmax.unsupported_viewport: layout {} viewport {index} cannot be resolved: \
                     {reason}",
                    layout_id.0
                )))
            }
        };
        let (paper_min, paper_max) = transform.paper_bounds();
        let model_min = transform.paper_to_model(Point3 {
            x: paper_min[0],
            y: paper_min[1],
            z: 0.0,
        });
        let model_max = transform.paper_to_model(Point3 {
            x: paper_max[0],
            y: paper_max[1],
            z: 0.0,
        });
        let width = model_max.x - model_min.x;
        let height = model_max.y - model_min.y;
        if !model_min.x.is_finite()
            || !model_min.y.is_finite()
            || !model_max.x.is_finite()
            || !model_max.y.is_finite()
            || width <= 0.0
            || height <= 0.0
        {
            return Err(CadError::InvalidInput(format!(
                "vpmax.degenerate_viewport: layout {} viewport {index} shows no positive model \
                 window (paper rectangle {} x {})",
                layout_id.0,
                paper_max[0] - paper_min[0],
                paper_max[1] - paper_min[1]
            )));
        }
        let space = session.active_space.clone();
        let viewport = self
            .workspace
            .viewports
            .get_mut(&command.viewport)
            .ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "vpmax.unknown_viewport: this session owns no viewport {:?}",
                    command.viewport
                ))
            })?;
        let [logical_width, logical_height] = viewport.logical_size;
        if !logical_width.is_finite()
            || !logical_height.is_finite()
            || logical_width <= 0.0
            || logical_height <= 0.0
        {
            return Err(CadError::Unsupported(format!(
                "vpmax.unknown_view_metrics: viewport {:?} has no usable logical size \
                 ({logical_width} x {logical_height}); VPMAX cannot frame the maximized view",
                command.viewport
            )));
        }
        let world_per_px = (width / logical_width)
            .max(height / logical_height)
            .max(MIN_ORTHO_SCALE);
        let centre = [
            (model_min.x + model_max.x) * 0.5,
            (model_min.y + model_max.y) * 0.5,
        ];
        let camera = Camera {
            eye: Point3 {
                x: centre[0],
                y: centre[1],
                z: viewport.camera.eye.z,
            },
            target: Point3 {
                x: centre[0],
                y: centre[1],
                z: 0.0,
            },
            up: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            projection: Projection::Orthographic {
                scale: world_per_px,
            },
        };
        camera.validate().map_err(|error| {
            CadError::Invariant(format!(
                "VPMAX: the computed maximized camera is not valid: {error:?}"
            ))
        })?;
        // Capture the view **before** the camera moves, then store it as the
        // documented restore state.
        let saved = ViewSnapshot::capture(viewport);
        session.view_states.save(VPMAX_RESTORE_VIEW, saved, space);
        viewport.camera = camera;
        viewport.view_mode = ViewMode2d3d::TwoD { saved: camera };
        session.set_maximized_viewport(target);
        session.generation += 1;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "vpmax.maximized".into(),
                message: format!(
                    "VPMAX: maximized layout {} viewport {index} and framed its model window \
                     ({}, {})-({}, {}) at {} world unit(s) per pixel; the pre-maximize view and \
                     space are stored as {:?} and VPMIN restores them exactly",
                    layout_id.0,
                    model_min.x,
                    model_min.y,
                    model_max.x,
                    model_max.y,
                    world_per_px,
                    VPMAX_RESTORE_VIEW
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `VPMIN`: leave the maximized editing state and restore the captured view.
    pub(crate) fn vp_min(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "vpmin.invalid_payload: VPMIN takes no payload".into(),
            ));
        }
        let Some(target) = session.maximized_viewport() else {
            // A reserved snapshot without a marker is stale state (a refused or
            // interrupted VPMAX); drop it so it cannot linger in `VIEW LIST`
            // while reporting the honest no-op.
            let removed = session.view_states.remove(VPMAX_RESTORE_VIEW);
            return Ok(unchanged_outcome(
                "vpmin.nothing_to_restore",
                if removed {
                    format!(
                        "VPMIN: no layout viewport is maximized; a stale {:?} restore snapshot \
                         was discarded and the view was left unchanged",
                        VPMAX_RESTORE_VIEW
                    )
                } else {
                    "VPMIN: no layout viewport is maximized; nothing changed".into()
                },
            ));
        };
        let Some(saved) = session.view_states.resolve(VPMAX_RESTORE_VIEW).cloned() else {
            return Err(CadError::Unsupported(format!(
                "vpmin.restore_state_missing: the pre-VPMAX snapshot {:?} of layout {} viewport \
                 {} is gone (it was deleted from the session view store), so the exact camera \
                 cannot be restored; the maximized state was left unchanged",
                VPMAX_RESTORE_VIEW, target.layout.0, target.index
            )));
        };
        let document_id = command.document;
        crate::space::validate_space_id(self.drawing(document_id)?, &saved.space).map_err(
            |_| {
                CadError::Unsupported(format!(
                    "vpmin.restore_space_missing: the saved space {:?} no longer exists in this \
                     drawing, so VPMIN left the maximized state unchanged",
                    saved.space
                ))
            },
        )?;
        let viewport = self
            .workspace
            .viewports
            .get_mut(&command.viewport)
            .ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "vpmin.unknown_viewport: this session owns no viewport {:?}",
                    command.viewport
                ))
            })?;
        saved.view.restore(viewport);
        session.active_space = saved.space;
        session.view_states.remove(VPMAX_RESTORE_VIEW);
        session.clear_maximized_viewport();
        session.generation += 1;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "vpmin.restored".into(),
                message: format!(
                    "VPMIN: restored the pre-VPMAX camera, 2D/3D mode, work plane and space of \
                     layout {} viewport {} exactly",
                    target.layout.0, target.index
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
