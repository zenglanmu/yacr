//! Command dispatch and view/camera commands for [`Application`].
use super::*;
use crate::app_zoom::FitResult;

impl Application {
    pub fn new() -> Self {
        Application {
            workspace: Workspace::default(),
            history: BTreeMap::new(),
            measurement: MeasurementEngine::default(),
            query: QueryService::new(),
            vendor_representations: None,
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
        if let Some(outcome) = self.dispatch_extended(session, &command)? {
            return Ok(outcome);
        }
        match command.id {
            CommandId::SelectAll => self.select_all(session, &command),
            CommandId::SelectSimilar => self.select_similar(session, &command),
            CommandId::SelectInBox => self.select_in_box(session, &command),
            id if crate::select_filter::batch(id) => self.dispatch_batch(session, id, &command),
            id if crate::app_drawing::annot_batch::batch(id) => {
                self.annot_batch(session, id, &command)
            }
            id if crate::app_extras::is_extras(id) => {
                self.extras_command(session, id, &command)
            }
            // Read-only inquiry: no transaction/revision/undo; FS mutates selection.
            CommandId::List => self.list_objects(session, &command),
            CommandId::Status => self.drawing_status(session, &command),
            CommandId::Fs => self.selection_touching(session, &command),
            // The 2026-11-07 utility batch (writes vs read-only reports).
            CommandId::Purge | CommandId::Rename | CommandId::MRedo | CommandId::Multiple => {
                self.utility_command(session, &command)
            }
            CommandId::SetVar | CommandId::Time | CommandId::DbList | CommandId::TreeStat => {
                self.utility_report(session, &command)
            }
            // 2026-11-07 dimension batch: read-only reports and explicit no-ops.
            CommandId::DimStyle => self.dimstyle_command(session, &command),
            CommandId::DimRegen => self.dim_regen(&command),
            CommandId::DimDisassociate => self.dim_disassociate(&command),
            // MASSPROP / MEASUREGEOM RADIUS are report-only (Viewer-legal).
            CommandId::MassProp => self.mass_prop(session, &command),
            CommandId::MeasureGeom => self.measure_geom(session, &command),
            CommandId::FitDrawing => self.fit_drawing(session, &command),
            CommandId::RestoreLayers => {
                // Drop temporary overrides; the drawing's layer flags remain the
                // authority (F03). A change is one `LAYERP` step pushed before the clear.
                if !session.layer_overrides.is_empty() {
                    session.push_layer_override_snapshot();
                    session.layer_overrides.clear();
                }
                Ok(CommandOutcome::none())
            }
            // `LAYERP`: pop the newest layer-override snapshot and apply it (it
            // never pushes); an empty history is a named no-op.
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
            // Layer **selection** commands (see `app_drawing::layer_commands`).
            CommandId::LayCur => self.layer_selection_command(session, &command),
            CommandId::LayMCur => self.layer_selection_command(session, &command),
            CommandId::LayFrz => self.layer_selection_command(session, &command),
            CommandId::LayThw => self.layer_selection_command(session, &command),
            CommandId::LayLck => self.layer_selection_command(session, &command),
            CommandId::LayUlk => self.layer_selection_command(session, &command),
            CommandId::SetByLayer => self.layer_selection_command(session, &command),
            // POINT display style writes (`PDMODE`/`PDSIZE`): never undoable.
            CommandId::SetPointDisplayMode | CommandId::SetPointDisplaySize => {
                self.point_display_command(&command)
            }
            // The 2026-11-07 layer batch (`app_drawing::layer_extended`).
            CommandId::LayMCh
            | CommandId::LayMrg
            | CommandId::LayDel
            | CommandId::LayerState
            | CommandId::LayerPmode
            | CommandId::SetLinetypeScale
            | CommandId::LayWalk => self.layer_extended_command(session, &command),
            CommandId::SwitchSpace => {
                if let CommandPayload::Space(space) = &command.payload {
                    self.switch_space_command(session, &command, space.clone())
                } else {
                    Err(CadError::InvalidInput(
                        "SwitchSpace needs a space payload".into(),
                    ))
                }
            }
            // `MODEL` (AutoCAD 2026) is the payload-free spelling of
            // `SPACE MODEL`: one session-state switch, never a second space path.
            CommandId::ModelSpace => {
                if !matches!(command.payload, CommandPayload::None) {
                    return Err(CadError::InvalidInput("MODEL needs no payload".into()));
                }
                self.switch_space_command(session, &command, SpaceId::Model)
            }
            // The 2026-11-07 layout/viewport batch. Read/list, the space switch
            // and the table/viewport writes are real; `LAYVPI` stays a named
            // refusal because this build has no per-viewport layer state.
            CommandId::Layout
            | CommandId::LayoutMerge
            | CommandId::LayoutWizard
            | CommandId::Vports
            | CommandId::VpClip
            | CommandId::VpScale
            | CommandId::LayVpi => self.layout_command(session, &command),
            // `VPMAX`/`VPMIN` are session display state only (no drawing write).
            CommandId::VpMax => self.vp_max(session, &command),
            CommandId::VpMin => self.vp_min(session, &command),
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
            CommandId::PlanView => {
                // `PLAN` is AutoCAD 2026's "display an orthographic view of the
                // XY plane". v1 is the **top/plan 2D view only**: this build has
                // no UCS model, so AutoCAD's UCS/World/Current orientation
                // choices are NOT IMPLEMENTED and `PLAN` takes no payload. A
                // caller that supplies one is refused by name rather than having
                // its value ignored or mapped onto the world plan.
                if !matches!(command.payload, CommandPayload::None) {
                    return Err(CadError::InvalidInput("PLAN needs no payload".into()));
                }
                self.plan_view(session, &command)
            }
            // The 2026-11-07 view/space batch: every arm is session view state
            // only (no database write, revision bump or undo entry), so Viewer
            // mode admits all of them. `VPOINT` moves the camera, never the model.
            CommandId::Redraw => self.redraw(session, &command, false),
            CommandId::RedrawAll => self.redraw(session, &command, true),
            CommandId::Regen => self.regen(session, &command, false),
            CommandId::RegenAll => self.regen(session, &command, true),
            CommandId::View => self.view_command(session, &command),
            CommandId::ViewGo => self.view_go(session, &command),
            CommandId::ViewBack => self.view_back(session, &command),
            CommandId::ViewForward => self.view_forward(session, &command),
            CommandId::VPoint => self.vpoint(session, &command),
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
            CommandId::Zoom => self.zoom(session, &command),
            CommandId::OpenDrawing
            | CommandId::NewDrawing
            | CommandId::CloseDrawing
            | CommandId::PlotDrawing
            | CommandId::SaveDrawingAs
            | CommandId::CancelLoading => Err(CadError::Unsupported(
                "file open, new drawing, close drawing, plot export, save-as and cancel are \
                 performed by the platform host, not the application"
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
            // AUDIT is the detect-only drawing audit: it reads every stored entity
            // through `cad_db::validate_entity` and repairs nothing.
            CommandId::AuditDrawing => self.audit_drawing(session, &command),
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
            | CommandId::CreateCircle2P
            | CommandId::CreateCircle3P
            | CommandId::CreateCircleDiameter
            | CommandId::CreateArc
            | CommandId::CreateXLine
            | CommandId::CreateRay
            | CommandId::CreatePolyline
            | CommandId::Wipeout
            | CommandId::CreateRectangle
            | CommandId::CreateEllipse
            | CommandId::CreatePolygon
            | CommandId::CreatePolygonCircumscribed
            | CommandId::CreatePoint
            | CommandId::CreateSpline
            | CommandId::DimLinear
            | CommandId::DimAligned
            | CommandId::DimRotated
            | CommandId::DimSpace
            | CommandId::DimTextEdit
            | CommandId::DimEdit
            | CommandId::DimOverride
            | CommandId::QDim
            | CommandId::DimJogLine
            | CommandId::DimBreak
            | CommandId::DimRadius
            | CommandId::DimDiameter
            | CommandId::DimCenter
            | CommandId::DimAngular
            | CommandId::DimOrdinate
            | CommandId::DimBaseline
            | CommandId::DimContinue
            | CommandId::Hatch
            | CommandId::HatchPoint
            | CommandId::Boundary
            | CommandId::HatchGenerateBoundary
            | CommandId::HatchToBack
            | CommandId::Gradient
            | CommandId::HatchEdit
            | CommandId::MoveEntities
            | CommandId::TrimEntity
            | CommandId::ExtendEntity
            | CommandId::Break
            | CommandId::Divide
            | CommandId::Lengthen
            | CommandId::ScaleText
            | CommandId::Torient
            | CommandId::TCase
            | CommandId::Txt2MText
            | CommandId::TextToFront
            | CommandId::JustifyText
            | CommandId::TextScale
            | CommandId::TextAlignEdges
            | CommandId::TextCount
            | CommandId::TextCircle
            | CommandId::TextMask
            | CommandId::TextUnmask
            | CommandId::ChamferLines
            | CommandId::FilletLines
            | CommandId::ChamferPolyline
            | CommandId::FilletPolyline
            | CommandId::RevisionCloud
            | CommandId::ArrayRectangular
            | CommandId::ArrayPolar
            | CommandId::ArrayPath
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
            | CommandId::PasteBlock
            | CommandId::OffsetEntity
            | CommandId::SetActiveLayer
            | CommandId::SetEntityProperty
            | CommandId::MatchProp
            | CommandId::GripStretch
            | CommandId::Reverse
            | CommandId::Pedit
            | CommandId::Oops
            | CommandId::Overkill
            | CommandId::BreakAtPoint
            | CommandId::CreateText
            | CommandId::CreateMText
            | CommandId::CreateShape
            | CommandId::MLeader
            | CommandId::TextEdit
            | CommandId::InsertBlock
            | CommandId::DefineBlock
            // The 2026-11-07 draw/modify batch.
            | CommandId::Donut
            | CommandId::Solid
            | CommandId::Helix
            | CommandId::Mline
            | CommandId::MlStyle
            | CommandId::Trex
            | CommandId::Extrim
            | CommandId::Mpedit
            | CommandId::DrawOrder => {
                // Every edit runs through one funnel so the session selection is
                // reconciled against the authoritative drawing afterwards: an
                // edit that removed entities (ERASE, a deleting TRIM, CUT) leaves
                // no dangling ref, and an edit that only adds or transforms
                // entities leaves a still-resolving selection unchanged.
                let outcome = self.drawing_command(session, &command)?;
                self.reconcile_selection(session);
                Ok(outcome)
            }
            other => Err(CadError::Invariant(format!("unhandled command {other:?}"))),
        }
    }
}

// The view/camera half of `Application` (2026-11-12): split out so this file
// stays inside the 1000-line code gate. The inherent impl is unchanged.
#[path = "application_view.rs"]
mod view;
