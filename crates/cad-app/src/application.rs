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
            CommandId::SelectInBox => self.select_in_box(session, &command),
            CommandId::FitDrawing => self.fit_drawing(session, &command),
            CommandId::RestoreLayers => {
                // Drop temporary overrides; the drawing's own layer flags remain
                // the authority and the database revision is untouched (F03).
                // A real change is one `LAYERP` step, recorded **before** the
                // clear so `LAYERP` can bring the cleared state back; a restore
                // with nothing to restore pushes nothing.
                if !session.layer_overrides.is_empty() {
                    session.push_layer_override_snapshot();
                    session.layer_overrides.clear();
                }
                Ok(CommandOutcome::none())
            }
            // `LAYERP`: pop the newest layer-override snapshot and apply it. It
            // never pushes (see `crate::layer_history`), so repeated presses walk
            // back monotonically. An empty history is a no-op with a named
            // diagnostic, not an error — the same shape `RestoreLayers` uses for
            // "nothing to restore".
            CommandId::LayerPrevious => {
                // `LAYERP` takes no input at all: a caller that supplies one is
                // refused rather than silently ignored (the selection is not its
                // input either).
                if !matches!(command.payload, CommandPayload::None) {
                    return Err(CadError::InvalidInput(
                        "layers.previous.invalid_payload: LAYERP takes no payload".into(),
                    ));
                }
                match session.layer_history.pop() {
                    Some(snapshot) => {
                        let restored = snapshot.len();
                        session.layer_overrides = snapshot;
                        Ok(CommandOutcome {
                            objects: Vec::new(),
                            changes: None,
                            diagnostics: vec![Diagnostic {
                                object: None,
                                code: "layers.previous".into(),
                                message: format!(
                                    "LAYERP: restored the previous layer state ({restored} \
                                     temporary override(s); session state only)"
                                ),
                            }],
                            measurement: None,
                            point_report: None,
                        })
                    }
                    None => Ok(CommandOutcome {
                        objects: Vec::new(),
                        changes: None,
                        diagnostics: vec![Diagnostic {
                            object: None,
                            code: "layers.previous.empty".into(),
                            message: "LAYERP: there is no previous layer state to restore in this \
                                      session; nothing changed"
                                .into(),
                        }],
                        measurement: None,
                        point_report: None,
                    }),
                }
            }
            // LAYISO is a **session view** operation (temporary overrides only: no
            // revision bump, no history entry, no database write), so it is
            // dispatched here beside the other layer-view commands rather than
            // through the entity-edit funnel.
            CommandId::IsolateLayers => self.isolate_layers(session, &command),
            // LAYOFF is its sibling: same session-override funnel, same "the
            // current layer is never hidden" rule.
            CommandId::LayOff => self.layoff_layers(session, &command),
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
            // Layer **table** writes (create/rename/delete/attribute): real
            // database writes in one revision bump each, never undoable,
            // dispatched ahead of the entity-edit funnel because they mutate no
            // entity.
            CommandId::CreateLayer
            | CommandId::RenameLayer
            | CommandId::DeleteLayer
            | CommandId::SetLayerProperty => self.layer_command(session, &command),
            // The two drawing-header POINT display style writes (`PDMODE <n>` /
            // `PDSIZE <n>`): real database writes in one revision bump each (the
            // display refresh signal), never undoable because `cad-history` has
            // no header patch kind, and no entity change — so they also stay out
            // of the entity-edit funnel.
            CommandId::SetPointDisplayMode | CommandId::SetPointDisplaySize => {
                self.point_display_command(&command)
            }
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
            // `AREA OBJECT`: the exact area of one **selected** closed entity,
            // computed from its stored geometry (never from picked points). A
            // measurement writes nothing, so it opens no transaction and records no
            // undo entry.
            CommandId::AreaObject => self.area_object(session, &command),
            CommandId::ConfirmMeasurement => self.confirm_measurement(session, &command),
            CommandId::CancelMeasurement => {
                // Cancelling is always allowed and never opens a transaction.
                session.cancel_tool()?;
                Ok(CommandOutcome::none())
            }
            CommandId::Undo => {
                // An undo can remove the entity a live selection points at (for
                // example undoing its creation); reconcile so the selection never
                // dangles.
                let outcome = self.undo(&command)?;
                self.reconcile_selection(session);
                Ok(outcome)
            }
            CommandId::Redo => {
                // A redo of a deleting edit removes entities just like the
                // original edit did.
                let outcome = self.redo(&command)?;
                self.reconcile_selection(session);
                Ok(outcome)
            }
            CommandId::SwitchProjection => {
                // Explicit toggle of the projection kind, preserving the target.
                // 2D/3D mode is updated to stay consistent with the projection.
                // Entering 3D frames the model (see `Camera::enter_3d`) instead of
                // inheriting the 2D plan's unit eye offset, which would put the
                // eye inside the model and clip it at the near plane.
                let bounds = self.document_bounds(command.document);
                let viewport = self.viewport_mut(&command)?;
                // A discrete view change: record the pre-switch view first. The
                // `enter_3d` below can still refuse a degenerate camera, in which
                // case the recorded step restores the view already on screen —
                // `ZOOM PREVIOUS` is then a no-op, never a fabricated view.
                viewport.push_view_snapshot();
                match viewport.camera.projection {
                    Projection::Orthographic { .. } => {
                        let saved = viewport.view_mode.saved_2d_camera();
                        viewport.camera =
                            saved.enter_3d(bounds, camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?;
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
                    // A standard view is a **discrete** view change: record the
                    // pre-view state first. The plan view while already in 2D is
                    // a documented no-op (it restores the very camera on screen),
                    // so it records no step — otherwise `ZOOM PREVIOUS` would burn
                    // a press on a view identical to the current one.
                    let plan_noop = view.is_plan() && self.view_state_is_2d(&command.viewport);
                    if !plan_noop {
                        self.push_view_snapshot(&command.viewport)?;
                    }
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
                // A discrete view change: record the pre-reset view first, so
                // `ZOOM PREVIOUS` can undo the reset (camera, mode **and** the
                // plan work plane it also restores).
                viewport.push_view_snapshot();
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
            | CommandId::SaveDrawingAs
            | CommandId::CancelLoading => Err(CadError::Unsupported(
                "file open, new drawing, plot export, save-as and cancel are performed by the \
                 platform host, not the application"
                    .into(),
            )),
            CommandId::Select => {
                // Selection is read-only: it stores the picked refs and enters
                // the Selecting tool state, but never writes the DWG (F05).
                session.begin_selecting();
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
                        point_report: None,
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
                        point_report: None,
                    })
                }
                _ => Err(CadError::InvalidInput(
                    "SetMode needs a Mode(AppMode) payload".into(),
                )),
            },
            CommandId::ToggleHoverPreview => {
                // Display/session state only: no database write, no history and
                // no ChangeSet. The host also clears a stale tint when this
                // transition disables a hover that is already painted.
                let enabled = session.toggle_hover_preview();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "view.hover_preview".into(),
                        message: format!("hover preview: {enabled}"),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            CommandId::ToggleSnap => {
                // Display/session state only (ADR 0014 A.4): no database write,
                // no history and no ChangeSet. The two tool boundaries read this
                // same flag through `HostController::resolve_snap`.
                let enabled = session.toggle_snap();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "view.snap".into(),
                        message: format!("object snap: {enabled}"),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            CommandId::ToggleGrid => {
                // Display/session state only: no database write, no history and
                // no ChangeSet. The host composes this switch with the persisted
                // `view.overlays.grid` preference when it builds the render
                // overlay visibility, so flipping it here never touches the
                // document.
                let enabled = session.toggle_grid();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "view.grid".into(),
                        message: format!("grid: {enabled}"),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            CommandId::ToggleSnapGrid => {
                // Display/session state only: no database write, no history and
                // no ChangeSet. The capture and preview paths read this same flag
                // (`SessionState::capture_point` / the preview projections), so
                // flipping it here never touches the document or the persisted
                // preferences. Grid snap is neither object snap (OSNAP) nor the
                // display grid (GRID).
                let enabled = session.toggle_snap_grid();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "view.snap_grid".into(),
                        message: format!("grid snap: {enabled}"),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            CommandId::SetSnapSpacing => match command.payload {
                CommandPayload::SnapSpacing(spacing) => {
                    // Session/display state only: `set_snap_spacing` validates
                    // (finite and positive) and refuses explicitly rather than
                    // clamping, so a bad value can never reach the quantiser.
                    session.set_snap_spacing(spacing)?;
                    Ok(CommandOutcome {
                        objects: Vec::new(),
                        changes: None,
                        diagnostics: vec![Diagnostic {
                            object: None,
                            code: "view.snap_spacing".into(),
                            message: format!("snap spacing: {spacing}"),
                        }],
                        measurement: None,
                        point_report: None,
                    })
                }
                _ => Err(CadError::InvalidInput(
                    "SNAPUNIT needs a positive spacing, e.g. `SNAPUNIT 10`".into(),
                )),
            },
            CommandId::ToggleOrtho => {
                // Display/session state only: no database write, no history and
                // no ChangeSet. The capture and preview paths read this same flag
                // (`SessionState::capture_point` / the preview projections), so
                // flipping it here never touches the document or the persisted
                // preferences. Grid snap (`SNAP`, F9) is a separate switch.
                let enabled = session.toggle_ortho();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "view.ortho".into(),
                        message: format!("ortho: {enabled}"),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            CommandId::CreateLine
            | CommandId::CreateCircle
            | CommandId::CreateArc
            | CommandId::CreatePolyline
            | CommandId::CreateRectangle
            | CommandId::CreateEllipse
            | CommandId::CreatePolygon
            | CommandId::CreatePoint
            | CommandId::CreateSpline
            | CommandId::DimLinear
            | CommandId::DimAligned
            | CommandId::DimRadius
            | CommandId::DimDiameter
            | CommandId::DimCenter
            | CommandId::DimAngular
            | CommandId::DimOrdinate
            | CommandId::DimBaseline
            | CommandId::DimContinue
            | CommandId::Hatch
            | CommandId::MoveEntities
            | CommandId::TrimEntity
            | CommandId::ExtendEntity
            | CommandId::ChamferLines
            | CommandId::FilletLines
            | CommandId::ChamferPolyline
            | CommandId::FilletPolyline
            | CommandId::ArrayRectangular
            | CommandId::ArrayPolar
            | CommandId::AlignEntities
            | CommandId::StretchEntities
            | CommandId::EraseEntities
            | CommandId::Join
            | CommandId::ExplodeEntities
            | CommandId::CopyEntities
            | CommandId::RotateEntities
            | CommandId::ScaleEntities
            | CommandId::MirrorEntities
            | CommandId::CopyToClipboard
            | CommandId::CutToClipboard
            | CommandId::PasteFromClipboard
            | CommandId::CopyToClipboardBase
            | CommandId::CutToClipboardBase
            | CommandId::PasteOriginalCoordinates
            | CommandId::OffsetEntity
            | CommandId::SetActiveLayer
            | CommandId::SetEntityProperty
            | CommandId::MatchProp
            | CommandId::GripStretch
            | CommandId::CreateText
            | CommandId::CreateMText
            | CommandId::MLeader
            | CommandId::TextEdit => {
                // Every edit runs through one funnel so the session selection is
                // reconciled against the authoritative drawing afterwards: an
                // edit that removed entities (ERASE, a deleting TRIM, CUT) leaves
                // no dangling ref, and an edit that only adds or transforms
                // entities leaves a still-resolving selection unchanged.
                let outcome = self.drawing_command(session, &command)?;
                self.reconcile_selection(session);
                Ok(outcome)
            }
        }
    }

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
            point_report: None,
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
            point_report: None,
        })
    }

    /// Model-space bounds of the open document, if it has any geometry.
    ///
    /// This is the same bounds source [`Self::fit_viewport`] frames in 2D; the
    /// 3D fit uses it for the eye distance and target, `z` included.
    fn document_bounds(&self, document: DocumentId) -> Option<(Point3, Point3)> {
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
            point_report: None,
        })
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
    fn view_state_is_2d(&self, viewport_id: &ViewportId) -> bool {
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

    /// Zoom the view.
    ///
    /// Four payloads are accepted, all view-only:
    /// * [`CommandPayload::ZoomAt`] — a factor plus the logical cursor to anchor
    ///   (the real zoom-to-cursor path used by both 2D and 3D).
    /// * [`CommandPayload::Points`] — the legacy factor-only zoom (no cursor),
    ///   retained for direct callers and the CLI.
    /// * [`CommandPayload::ZoomWindow`] — `ZOOM WINDOW`, the exact two-corner fit
    ///   (2D orthographic only; see [`Application::zoom_window`]).
    /// * [`CommandPayload::ZoomPrevious`] — `ZOOM PREVIOUS`, the discrete view
    ///   stack (see [`Application::zoom_previous`]).
    ///
    /// Every arm is a discrete view change except the two new ones' refusals: the
    /// factors record one `ZOOM PREVIOUS` step in `zoom_at`; the window and the
    /// previous restore record their own.
    pub(crate) fn zoom(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::ZoomAt { factor, cursor } => self.zoom_at(command, *factor, *cursor),
            CommandPayload::ZoomWindow { .. } => self.zoom_window(command),
            CommandPayload::ZoomPrevious => self.zoom_previous(command),
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
                "Zoom needs a factor, a { factor, cursor } payload, a ZOOM WINDOW rectangle or \
                 ZOOM PREVIOUS"
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
                "Zoom factor must be positive and finite".into(),
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
