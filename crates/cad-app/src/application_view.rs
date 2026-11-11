//! The view/camera half of `Application`.
//!
//! Split out of `application.rs` (2026-11-12) so both files stay inside the
//! 1000-line code gate. Every method is the same inherent impl on
//! [`Application`]; no public path changed.

use super::*;

impl Application {
    /// Validate the complete batch before publishing a replacement override set.
    /// Drawing data and history are untouched; unrelated overrides are retained.
    pub(crate) fn set_layer_visibilities(
        &self,
        session: &mut SessionState,
        document: DocumentId,
        changes: &[(LayerId, bool)],
    ) -> CadResult<CommandOutcome> {
        let drawing = &self
            .workspace
            .documents
            .get(&document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing;
        if changes.is_empty() {
            return Err(CadError::InvalidInput(
                "layer visibility batch must not be empty".into(),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for (id, _) in changes {
            if !seen.insert(*id) {
                return Err(CadError::InvalidInput(format!(
                    "duplicate layer id {} in visibility batch",
                    id.0
                )));
            }
            if drawing.layer(*id).is_none() {
                return Err(CadError::InvalidInput(format!("unknown layer {}", id.0)));
            }
        }
        let mut updated = session.layer_overrides.clone();
        for (id, visible) in changes {
            updated.set(*id, *visible);
        }
        // `LAYERP` records the **pre-change** state, and only when the batch really
        // changes the override set: a re-published identical batch is a no-op with
        // no phantom `LAYERP` step. The snapshot goes in before the assignment so
        // `LAYERP` restores the state the user was looking at.
        if updated != session.layer_overrides {
            session.push_layer_override_snapshot();
            session.layer_overrides = updated;
        }
        Ok(CommandOutcome::none())
    }

    /// Toggle between the 2D plan view and a 3D perspective view.
    ///
    /// Entering 3D promotes the projection to perspective, frames the document
    /// bounds ([`Camera::enter_3d`]) and snapshots the 2D camera; returning to
    /// 2D restores that camera exactly (audit F13). This is pure camera state —
    /// no GPU view or depth buffer is involved here.
    pub(crate) fn switch_2d3d(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let bounds = self.document_bounds(command.document);
        let viewport = self.viewport_mut(command)?;
        // A discrete view change in both directions: record the pre-switch view
        // first so `ZOOM PREVIOUS` can bring the observation back.
        viewport.push_view_snapshot();
        let message = match viewport.view_mode.kind() {
            ProjectionKind::TwoD => {
                let saved = viewport.camera;
                viewport.set_view_mode(ViewMode2d3d::ThreeD { saved_2d: saved })?;
                // Frame the model: the plan camera's eye offset says nothing
                // about the model's size (the measured defect carried a
                // one-unit offset), so without this the solid can sit at or
                // behind the near plane and be clipped away.
                viewport.camera = viewport
                    .camera
                    .enter_3d(bounds, camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?;
                "已切换到三维视图（透视）"
            }
            ProjectionKind::ThreeD => {
                let saved = viewport.view_mode.saved_2d_camera();
                viewport.set_view_mode(ViewMode2d3d::TwoD { saved })?;
                "已回到二维俯视图"
            }
        };
        session.generation = session.generation.saturating_add(1);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "view.mode".into(),
                message: message.into(),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// Orbit the current view about its target (spec F13).
    ///
    /// Requires an `Orbit { yaw, pitch }` payload; the camera must be in 3D mode
    /// so an accidental orbit never disturbs the exact 2D view. The near-plane
    /// guard lives in [`Camera::orbit`].
    pub(crate) fn orbit(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Orbit { yaw, pitch } = command.payload else {
            return Err(CadError::InvalidInput(
                "Orbit needs a { yaw, pitch } payload in radians".into(),
            ));
        };
        let viewport = self.viewport_mut(command)?;
        if viewport.view_mode.kind() != ProjectionKind::ThreeD {
            return Err(CadError::InvalidInput(
                "Orbit is only defined in the three-dimensional view".into(),
            ));
        }
        viewport.camera.orbit(yaw, pitch)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "view.orbit".into(),
                message: format!("轨道旋转 yaw={yaw:.4} pitch={pitch:.4}"),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// Model-space bounds of the open document, if it has any geometry.
    ///
    /// This is the same bounds source [`Self::fit_viewport`] frames in 2D; the
    /// 3D fit uses it for the eye distance and target, `z` included.
    pub(crate) fn document_bounds(&self, document: DocumentId) -> Option<(Point3, Point3)> {
        self.workspace
            .documents
            .get(&document)
            .and_then(|document| document.drawing.bounds())
    }

    pub(crate) fn viewport_mut(&mut self, command: &Command) -> CadResult<&mut Viewport> {
        self.workspace
            .viewports
            .get_mut(&command.viewport)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))
    }

    pub(crate) fn fit_drawing(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match self.fit_active_space(session, &command.viewport, true)? {
            FitResult::Framed => Ok(CommandOutcome::none()),
            // Model space has nothing to frame: report an informational
            // diagnostic instead of a generic success. `fit.empty` is the
            // stable code the host resolves to its localized message; the CLI
            // sees the same code, so the machine path is unchanged.
            FitResult::EmptyModel => Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "fit.empty".into(),
                    message: "fit.empty".into(),
                }],
                measurement: None,
                point_report: None,
            }),
            // A paper sheet with no geometry and no usable paper size must not
            // pretend it was framed. The layout id is part of the message so a
            // host can point at the tab the user switched to.
            FitResult::EmptyLayout(layout) => Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "layout.fit_empty".into(),
                    message: format!(
                        "layout {} has no drawable paper geometry and no usable paper size; the \
                         camera is unchanged",
                        layout.0
                    ),
                }],
                measurement: None,
                point_report: None,
            }),
        }
    }

    /// Record the viewport's current view as one `ZOOM PREVIOUS` step.
    ///
    /// Called **before** each discrete view change (FIT, a zoom factor,
    /// `ZOOM WINDOW`, `STANDARDVIEW`, `SWITCH2D3D`, `PROJECTION`, `RESETVIEW`);
    /// see [`crate::zoom_history`] for the push rule and for why continuous
    /// navigation never records a step.
    pub(crate) fn push_view_snapshot(&mut self, viewport_id: &ViewportId) -> CadResult<()> {
        let viewport = self
            .workspace
            .viewports
            .get_mut(viewport_id)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        viewport.push_view_snapshot();
        Ok(())
    }

    /// Whether a viewport is currently showing the 2D plan observation.
    ///
    /// `pub(crate)` because the `PLAN` view command (`app_zoom::plan_view`) makes
    /// the same "already 2D is a no-op" decision the `STANDARDVIEW` arm makes.
    pub(crate) fn view_state_is_2d(&self, viewport_id: &ViewportId) -> bool {
        self.workspace
            .viewports
            .get(viewport_id)
            .is_some_and(|viewport| viewport.view_mode.kind() == ProjectionKind::TwoD)
    }

    /// Pan the view by a world-space delta.
    ///
    /// Only the eye and target move; the projection and the orbit frame are
    /// preserved. The 2D plan pan is the special case where `eye` and `target`
    /// share an `x/y`, so this works unchanged in 3D.
    pub(crate) fn pan(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "Pan needs a world-space delta point".into(),
            ));
        };
        let delta = *points
            .first()
            .ok_or_else(|| CadError::InvalidInput("Pan delta missing".into()))?;
        if !camera::is_finite_point(delta) {
            return Err(CadError::InvalidInput("Pan delta must be finite".into()));
        }
        let viewport = self.viewport_mut(command)?;
        // The delta is a screen-plane world offset; shift both eye and target so
        // the view direction and distance are unchanged (audit F02 large coords).
        let shift = Point3 {
            x: -delta.x,
            y: -delta.y,
            z: 0.0,
        };
        viewport.camera.target = camera::add(viewport.camera.target, shift);
        viewport.camera.eye = camera::add(viewport.camera.eye, shift);
        // Keep a saved 2D camera in sync while in plan mode.
        if let ViewMode2d3d::TwoD { .. } = viewport.view_mode {
            viewport.view_mode = ViewMode2d3d::TwoD {
                saved: viewport.camera,
            };
        }
        Ok(CommandOutcome::none())
    }

    /// Zoom the view: five payloads are accepted, all view-only.
    /// * [`CommandPayload::ZoomAt`] — a factor plus the logical cursor to anchor
    ///   (the real zoom-to-cursor path used by both 2D and 3D, and the typed
    ///   `ZOOM n` / `ZOOM nX` form anchored at the viewport centre).
    /// * [`CommandPayload::Points`] — the legacy factor-only zoom (no cursor),
    ///   retained for direct callers and the CLI.
    /// * [`CommandPayload::ZoomWindow`] — `ZOOM WINDOW`, the exact two-corner fit
    ///   (2D orthographic only; see [`Application::zoom_window`]).
    /// * [`CommandPayload::ZoomPrevious`] — `ZOOM PREVIOUS`, the discrete view
    ///   stack (see [`Application::zoom_previous`]).
    /// * [`CommandPayload::ZoomObject`] — `ZOOM OBJECT`, the selection bounds fit
    ///   (2D orthographic only; see [`Application::zoom_object`]).
    ///
    /// Every arm is a discrete view change except the new ones' refusals: the
    /// factors record one `ZOOM PREVIOUS` step in `zoom_at`; the window, object
    /// and previous restore record their own.
    pub(crate) fn zoom(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::ZoomAt { factor, cursor } => self.zoom_at(command, *factor, *cursor),
            CommandPayload::ZoomWindow { .. } => self.zoom_window(command),
            CommandPayload::ZoomPrevious => self.zoom_previous(command),
            CommandPayload::ZoomObject => self.zoom_object(session, command),
            CommandPayload::Points(points) => {
                let factor = points
                    .first()
                    .map(|p| p.x)
                    .ok_or_else(|| CadError::InvalidInput("Zoom factor missing".into()))?;
                // Centre-of-canvas cursor for the legacy, anchor-free path.
                let size = self.viewport_mut(command)?.logical_size;
                self.zoom_at(command, factor, [size[0] * 0.5, size[1] * 0.5])
            }
            _ => Err(CadError::InvalidInput(
                "Zoom needs a factor, a { factor, cursor } payload, a ZOOM WINDOW rectangle, ZOOM \
                 OBJECT or ZOOM PREVIOUS"
                    .into(),
            )),
        }
    }

    /// Apply a zoom factor anchored at a logical cursor in both projections.
    ///
    /// A real zoom is a **discrete** view change, so the pre-zoom view is
    /// recorded as one `ZOOM PREVIOUS` step ([`crate::zoom_history`]) after the
    /// factor is validated and before the camera moves; a refused factor records
    /// nothing.
    pub(crate) fn zoom_at(
        &mut self,
        command: &Command,
        factor: f64,
        cursor: [f64; 2],
    ) -> CadResult<CommandOutcome> {
        if factor <= 0.0 || !factor.is_finite() {
            return Err(CadError::InvalidInput(
                "zoom.factor.invalid: the zoom factor must be positive and finite".into(),
            ));
        }
        let viewport = self.viewport_mut(command)?;
        let size = viewport.logical_size;
        viewport.push_view_snapshot();
        viewport.camera.zoom_at(factor, cursor, size)?;
        if let ViewMode2d3d::TwoD { .. } = viewport.view_mode {
            viewport.view_mode = ViewMode2d3d::TwoD {
                saved: viewport.camera,
            };
        }
        Ok(CommandOutcome::none())
    }

    /// Apply a named standard view to the current target.
    ///
    /// The plan (top) view is the canonical 2D view: it restores the 2D camera
    /// snapshotted when the 3D observation was entered — the same camera
    /// [`Application::switch_2d3d`] restores — so a standard-view round trip
    /// never loses the 2D framing. While already in 2D that snapshot is the
    /// current camera, so the plan view is a re-orientation no-op.
    ///
    /// Every other standard view is a 3D observation: the camera is placed at
    /// `target + offset * distance`, where `offset` is the standard view's unit
    /// eye direction. Entering 3D **from the plan** derives the distance from the
    /// document bounds ([`Camera::enter_3d`]), exactly like the other 3D entry
    /// points: the plan camera's fixed eye height (1000 world units) says nothing
    /// about the model's size, so a small model would be a speck and a large one
    /// would put the eye inside the geometry. From an existing 3D observation the
    /// current distance is kept — a standard view re-orients, it does not reset
    /// the user's zoom. The resulting basis is right-handed and orthonormal
    /// ([`Camera::view_basis`]).
    pub(crate) fn apply_standard_view(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
        view: StandardView,
    ) -> CadResult<()> {
        let bounds = self.document_bounds(command.document);
        let viewport = self.viewport_mut(command)?;
        if view.is_plan() {
            let saved = viewport.view_mode.saved_2d_camera();
            viewport.camera = saved;
            viewport.view_mode = ViewMode2d3d::TwoD { saved };
            return Ok(());
        }

        let entering_from_plan = matches!(viewport.view_mode.kind(), ProjectionKind::TwoD);
        let target = viewport.camera.target;
        let distance = if entering_from_plan {
            viewport
                .camera
                .enter_3d(bounds, camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?
                .distance()
        } else {
            let d = viewport.camera.distance();
            if d.is_finite() && d > 1e-6 {
                d
            } else {
                1000.0
            }
        };
        let offset = camera::scale3(view.eye_offset(), distance);
        let eye = camera::add(target, offset);
        let camera = Camera {
            eye,
            target,
            up: view.up_hint(),
            projection: viewport.camera.projection,
        };
        camera.validate()?;
        viewport.camera = camera;

        // A non-plan standard view is a 3D observation.
        if viewport.camera.projection.is_orthographic() {
            viewport.camera.projection =
                Projection::perspective(camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?;
        }
        let saved = viewport.view_mode.saved_2d_camera();
        viewport.view_mode = ViewMode2d3d::ThreeD { saved_2d: saved };
        Ok(())
    }
}
