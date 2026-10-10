//! Command dispatch and view/camera commands for [`Application`].
use super::*;

impl Application {
    pub fn new() -> Self {
        Application {
            workspace: Workspace::default(),
            history: BTreeMap::new(),
            measurement: MeasurementEngine::default(),
            query: QueryService::new(),
        }
    }

    /// Execute a command. This is the single entry point shared by UI and CLI.
    pub fn execute(
        &mut self,
        session: &mut SessionState,
        command: Command,
    ) -> CadResult<CommandOutcome> {
        session.authorize(command.id)?;
        if session.document != command.document {
            return Err(CadError::StaleResult);
        }
        match command.id {
            CommandId::SelectAll => self.select_all(session, &command),
            CommandId::FitDrawing => self.fit_drawing(session, &command),
            CommandId::RestoreLayers => {
                // Drop temporary overrides; the drawing's own layer flags remain
                // the authority and the database revision is untouched (F03).
                session.layer_overrides.clear();
                Ok(CommandOutcome::none())
            }
            CommandId::ToggleLayer => {
                if let CommandPayload::Layer(id, visible) = command.payload {
                    self.set_layer_visibilities(session, command.document, &[(id, visible)])
                } else {
                    Err(CadError::InvalidInput(
                        "ToggleLayer needs a layer payload".into(),
                    ))
                }
            }
            CommandId::SetLayerVisibilities => match &command.payload {
                CommandPayload::LayerVisibilities(changes) => {
                    self.set_layer_visibilities(session, command.document, changes)
                }
                _ => Err(CadError::InvalidInput(
                    "SetLayerVisibilities needs a layer visibilities payload".into(),
                )),
            },
            CommandId::SwitchSpace => {
                if let CommandPayload::Space(space) = command.payload {
                    // Validate against the *current* drawing before recording the
                    // switch: an unknown or undrawable layout is an explicit
                    // refusal, never an empty "success" that later renders a
                    // blank sheet (audit F04/U03).
                    let drawing = &self
                        .workspace
                        .documents
                        .get(&command.document)
                        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
                        .drawing;
                    validate_space_id(drawing, &space)?;
                    session.active_space = space;
                    Ok(CommandOutcome::none())
                } else {
                    Err(CadError::InvalidInput(
                        "SwitchSpace needs a space payload".into(),
                    ))
                }
            }
            CommandId::Measure => self.measure(session, &command),
            CommandId::ConfirmMeasurement => self.confirm_measurement(session, &command),
            CommandId::CancelMeasurement => {
                // Cancelling is always allowed and never opens a transaction.
                session.cancel_tool()?;
                Ok(CommandOutcome::none())
            }
            CommandId::Undo => self.undo(&command),
            CommandId::Redo => self.redo(&command),
            CommandId::SwitchProjection => {
                // Explicit toggle of the projection kind, preserving the target.
                // 2D/3D mode is updated to stay consistent with the projection.
                let viewport = self.viewport_mut(&command)?;
                match viewport.camera.projection {
                    Projection::Orthographic { .. } => {
                        viewport.camera.projection =
                            Projection::perspective(camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?;
                        let saved = viewport.view_mode.saved_2d_camera();
                        viewport.view_mode = ViewMode2d3d::ThreeD { saved_2d: saved };
                    }
                    Projection::Perspective { .. } => {
                        let saved = viewport.view_mode.saved_2d_camera();
                        viewport.camera = saved;
                        viewport.view_mode = ViewMode2d3d::TwoD { saved };
                    }
                }
                Ok(CommandOutcome::none())
            }
            CommandId::StandardView => {
                if let CommandPayload::StandardView(view) = command.payload {
                    self.apply_standard_view(session, &command, view)?;
                    Ok(CommandOutcome::none())
                } else {
                    Err(CadError::InvalidInput(
                        "StandardView needs a view payload".into(),
                    ))
                }
            }
            CommandId::ResetView => {
                let viewport = self.viewport_mut(&command)?;
                let camera = Camera::top_view_2d();
                viewport.camera = camera;
                viewport.view_mode = ViewMode2d3d::TwoD { saved: camera };
                viewport.work_plane = xy_work_plane(0.0);
                Ok(CommandOutcome::none())
            }
            CommandId::Pan => self.pan(&command),
            CommandId::Zoom => self.zoom(&command),
            CommandId::OpenDrawing
            | CommandId::NewDrawing
            | CommandId::PlotDrawing
            | CommandId::CancelLoading => Err(CadError::Unsupported(
                "file open, new drawing, plot export and cancel are performed by the platform \
                 host, not the application"
                    .into(),
            )),
            CommandId::Select => {
                // Selection is read-only: it stores the picked refs and enters
                // the Selecting tool state, but never writes the DWG (F05).
                session.tool = ToolState::Selecting;
                if let CommandPayload::Selection(refs) = command.payload {
                    session.selection = SelectionSet::from_refs(refs);
                }
                Ok(CommandOutcome::none())
            }
            CommandId::Resources => self.resources_report(&command),
            CommandId::Diagnostics => self.diagnostics_report(&command),
            CommandId::SwitchBackend => {
                if let CommandPayload::Backend(choice) = command.payload {
                    session.backend = choice;
                    Ok(CommandOutcome {
                        objects: Vec::new(),
                        changes: None,
                        diagnostics: vec![Diagnostic {
                            object: None,
                            code: "backend.preference".into(),
                            message: format!("后端偏好：{choice:?}（渲染会话由宿主重建）"),
                        }],
                        measurement: None,
                    })
                } else {
                    Err(CadError::InvalidInput(
                        "SwitchBackend needs a backend payload".into(),
                    ))
                }
            }
            CommandId::Switch2d3d => self.switch_2d3d(session, &command),
            CommandId::Orbit => self.orbit(&command),
            CommandId::SetMode => match command.payload {
                CommandPayload::Mode(mode) => {
                    // The session cancels an unconfirmed tool on switch; this is
                    // the one catalogue of modes, never a UI-only flag (U02).
                    session.switch_mode(mode)?;
                    Ok(CommandOutcome {
                        objects: Vec::new(),
                        changes: None,
                        diagnostics: vec![Diagnostic {
                            object: None,
                            code: "session.mode".into(),
                            message: format!("模式：{mode:?}"),
                        }],
                        measurement: None,
                    })
                }
                _ => Err(CadError::InvalidInput(
                    "SetMode needs a Mode(AppMode) payload".into(),
                )),
            },
            CommandId::CreateLine
            | CommandId::CreateCircle
            | CommandId::MoveEntities
            | CommandId::TrimEntity
            | CommandId::SetActiveLayer => self.drawing_command(session, &command),
        }
    }

    /// Validate the complete batch before publishing a replacement override set.
    /// Drawing data and history are untouched; unrelated overrides are retained.
    fn set_layer_visibilities(
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
        session.layer_overrides = updated;
        Ok(CommandOutcome::none())
    }

    /// Toggle between the 2D plan view and a 3D perspective view.
    ///
    /// Entering 3D promotes the projection to perspective and snapshots the 2D
    /// camera; returning to 2D restores that camera exactly (audit F13). This is
    /// pure camera state — no GPU view or depth buffer is involved here.
    pub(crate) fn switch_2d3d(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let viewport = self.viewport_mut(command)?;
        let message = match viewport.view_mode.kind() {
            ProjectionKind::TwoD => {
                let saved = viewport.camera;
                viewport.set_view_mode(ViewMode2d3d::ThreeD { saved_2d: saved })?;
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
        })
    }

    /// Report the resources the open document references (spec F10/F11).
    pub(crate) fn resources_report(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let keys = if document.resource_keys.is_empty() {
            "无外部资源引用".to_string()
        } else {
            document.resource_keys.join(", ")
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "resources.summary".into(),
                message: format!("资源引用 {}：{keys}", document.resource_keys.len()),
            }],
            measurement: None,
        })
    }

    /// Structured diagnostics summary for the UI panel and CLI (spec §19).
    pub(crate) fn diagnostics_report(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let bounds = document
            .drawing
            .bounds()
            .map(|(min, max)| format!("范围 {:.3},{:.3} → {:.3},{:.3}", min.x, min.y, max.x, max.y))
            .unwrap_or_else(|| "空图纸".to_string());
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "diagnostics.summary".into(),
                    message: format!(
                        "图元 {}，单位 {:?}",
                        document.drawing.entity_count(),
                        document.units.source
                    ),
                },
                Diagnostic {
                    object: None,
                    code: "diagnostics.bounds".into(),
                    message: bounds,
                },
            ],
            measurement: None,
        })
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
        if self.fit_viewport(session, &command.viewport)? {
            return Ok(CommandOutcome::none());
        }
        // Nothing to frame: report an informational diagnostic instead of a
        // generic success. `fit.empty` is the stable code the host resolves to
        // its localized message; the CLI sees the same code, so the machine path
        // is unchanged.
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "fit.empty".into(),
                message: "fit.empty".into(),
            }],
            measurement: None,
        })
    }

    /// Fit a viewport to the current document bounds. Hosts call this after an
    /// import so the first frame is usable without a synthetic command.
    ///
    /// Fitting always returns the viewport to the 2D plan view (audit F13), so a
    /// fit is a deterministic reset-and-frame, not a 3D manipulation.
    ///
    /// Returns `false` when the drawing has no measurable extent: the camera is
    /// left untouched and nothing is framed, so a caller can report that
    /// explicitly instead of claiming success.
    pub fn fit_viewport(
        &mut self,
        session: &mut SessionState,
        viewport_id: &ViewportId,
    ) -> CadResult<bool> {
        let document = self
            .workspace
            .documents
            .get(&session.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let bounds = document.drawing.bounds();
        let viewport = self
            .workspace
            .viewports
            .get_mut(viewport_id)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let Some((min, max)) = bounds else {
            // An empty drawing has nothing to frame. Fitting is a documented
            // no-op (the camera and the session generation are left unchanged),
            // never an error and never a fabricated view. The `false` return lets
            // the caller report it explicitly.
            return Ok(false);
        };
        let cx = (min.x + max.x) * 0.5;
        let cy = (min.y + max.y) * 0.5;
        let ex = (max.x - min.x).max(1e-6);
        let ey = (max.y - min.y).max(1e-6);
        let w = viewport.logical_size[0].max(1.0);
        let h = viewport.logical_size[1].max(1.0);
        let scale = ((ex / w).max(ey / h) * 1.05).max(camera::MIN_ORTHO_SCALE);
        let camera = Camera {
            eye: Point3 {
                x: cx,
                y: cy,
                z: viewport.camera.eye.z,
            },
            target: Point3 {
                x: cx,
                y: cy,
                z: 0.0,
            },
            up: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            projection: Projection::Orthographic { scale },
        };
        viewport.camera = camera;
        viewport.view_mode = ViewMode2d3d::TwoD { saved: camera };
        session.generation += 1;
        Ok(true)
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

    /// Zoom the view.
    ///
    /// Two payloads are accepted for compatibility:
    /// * [`CommandPayload::ZoomAt`] — a factor plus the logical cursor to anchor
    ///   (the real zoom-to-cursor path used by both 2D and 3D).
    /// * [`CommandPayload::Points`] — the legacy factor-only zoom (no cursor),
    ///   retained for direct callers and the CLI.
    pub(crate) fn zoom(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::ZoomAt { factor, cursor } => self.zoom_at(command, *factor, *cursor),
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
                "Zoom needs a factor or a { factor, cursor } payload".into(),
            )),
        }
    }

    /// Apply a zoom factor anchored at a logical cursor in both projections.
    pub(crate) fn zoom_at(
        &mut self,
        command: &Command,
        factor: f64,
        cursor: [f64; 2],
    ) -> CadResult<CommandOutcome> {
        if factor <= 0.0 || !factor.is_finite() {
            return Err(CadError::InvalidInput(
                "Zoom factor must be positive and finite".into(),
            ));
        }
        let viewport = self.viewport_mut(command)?;
        let size = viewport.logical_size;
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
    /// The camera is placed at `target + offset * distance`, where `offset` is
    /// the standard view's unit eye direction; the distance is taken from the
    /// current camera (or the drawing's diagonal the first time). The resulting
    /// basis is right-handed and orthonormal ([`Camera::view_basis`]). The plan
    /// (top) view returns the viewport to 2D mode with an orthographic
    /// projection; every other standard view is a 3D view.
    pub(crate) fn apply_standard_view(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
        view: StandardView,
    ) -> CadResult<()> {
        let viewport = self.viewport_mut(command)?;
        let target = viewport.camera.target;
        let distance = {
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

        if view.is_plan() {
            // The plan view is the canonical 2D view: force orthographic and keep
            // the current scale, then remember the camera for 2D↔3D round trips.
            let scale = match viewport.camera.projection {
                Projection::Orthographic { scale } => scale,
                Projection::Perspective { .. } => camera::DEFAULT_ORTHO_SCALE,
            };
            viewport.camera.projection = Projection::Orthographic { scale };
            viewport.view_mode = ViewMode2d3d::TwoD {
                saved: viewport.camera,
            };
        } else {
            // A non-plan standard view is a 3D observation.
            if viewport.camera.projection.is_orthographic() {
                viewport.camera.projection =
                    Projection::perspective(camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?;
            }
            let saved = viewport.view_mode.saved_2d_camera();
            viewport.view_mode = ViewMode2d3d::ThreeD { saved_2d: saved };
        }
        Ok(())
    }
}
