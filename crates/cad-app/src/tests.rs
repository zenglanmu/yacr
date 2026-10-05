//! cad-app contract tests.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer, Layout, PaperViewport};

// Aliases kept local so the tests read without re-importing camera helpers.
fn cam_xy_work_plane(z: f64) -> WorkPlane {
    xy_work_plane(z)
}
fn cad_app_len(v: Point3) -> f64 {
    camera::length3(v)
}
fn cad_app_dot(a: Point3, b: Point3) -> f64 {
    camera::dot3(a, b)
}

fn application_with_document() -> (Application, SessionState) {
    let mut app = Application::new();
    let document_id = DocumentId(1);
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    let drawing = builder.finish().unwrap();
    app.workspace.documents.insert(
        document_id,
        Document {
            id: document_id,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([0u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), document_id, [800.0, 600.0]),
    );
    let session = SessionState::new(document_id, AppMode::Work);
    (app, session)
}

fn command(id: CommandId, payload: CommandPayload) -> Command {
    Command {
        schema_version: 1,
        id,
        document: DocumentId(1),
        viewport: ViewportId(1),
        payload,
    }
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn application_with_layouts() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let corners = || {
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 100.0,
                y: 50.0,
                z: 0.0,
            },
            Point3 {
                x: 10.0,
                y: 20.0,
                z: 0.0,
            },
        ]
    };
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: vec![PaperViewport {
            clip: corners(),
            model_to_paper: Transform3::scale(100.0),
            completeness: Completeness::Complete,
        }],
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(2),
        name: "Broken".into(),
        viewports: vec![PaperViewport {
            clip: corners(),
            model_to_paper: Transform3::scale(100.0),
            completeness: Completeness::Partial(vec!["importer dropped height".into()]),
        }],
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

#[test]
fn stale_document_command_is_rejected() {
    let (mut app, mut session) = application_with_document();
    let mut cmd = command(CommandId::FitDrawing, CommandPayload::None);
    cmd.document = DocumentId(99);
    assert_eq!(
        app.execute(&mut session, cmd).unwrap_err(),
        CadError::StaleResult
    );
}

#[test]
fn measure_distance_is_reported() {
    let (mut app, mut session) = application_with_document();
    let cmd = command(
        CommandId::Measure,
        CommandPayload::Points(vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 3.0,
                y: 4.0,
                z: 0.0,
            },
        ]),
    );
    let outcome = app.execute(&mut session, cmd).unwrap();
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "measure.result"));
}

#[test]
fn layer_override_does_not_touch_the_database() {
    let (mut app, mut session) = application_with_document();
    let before = app
        .workspace
        .documents
        .get(&DocumentId(1))
        .unwrap()
        .drawing
        .revision();
    app.execute(
        &mut session,
        command(
            CommandId::ToggleLayer,
            CommandPayload::Layer(LayerId(3), false),
        ),
    )
    .unwrap();
    assert_eq!(session.layer_overrides.get(LayerId(3)), Some(false));
    assert_eq!(
        app.workspace
            .documents
            .get(&DocumentId(1))
            .unwrap()
            .drawing
            .revision(),
        before
    );
}

#[test]
fn restore_layers_clears_overrides_without_touching_the_database() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::ToggleLayer,
            CommandPayload::Layer(LayerId(0), false),
        ),
    )
    .unwrap();
    assert!(session.layer_overrides.clear_changed());

    let before = app
        .workspace
        .documents
        .get(&DocumentId(1))
        .unwrap()
        .drawing
        .revision();
    app.execute(
        &mut session,
        command(CommandId::RestoreLayers, CommandPayload::None),
    )
    .unwrap();
    assert!(session.layer_overrides.is_empty());
    assert_eq!(
        app.workspace
            .documents
            .get(&DocumentId(1))
            .unwrap()
            .drawing
            .revision(),
        before
    );
}

#[test]
fn select_command_records_selection_and_does_not_mutate_the_drawing() {
    let (mut app, mut session) = application_with_document();
    let revision_before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    // No entity exists in the empty database, but selection is still a pure
    // state update: refs are recorded and the drawing is untouched.
    let refs = vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(7),
        instance: InstancePath::default(),
        sub_element: None,
    }];
    app.execute(
        &mut session,
        command(CommandId::Select, CommandPayload::Selection(refs)),
    )
    .unwrap();
    assert_eq!(session.selection.len(), 1);
    assert!(matches!(session.tool, ToolState::Selecting));
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        revision_before
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn select_with_empty_payload_clears_the_selection() {
    let (mut app, mut session) = application_with_document();
    session.selection.replace([SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    assert_eq!(session.selection.len(), 1);
    app.execute(
        &mut session,
        command(CommandId::Select, CommandPayload::Selection(Vec::new())),
    )
    .unwrap();
    assert!(session.selection.is_empty());
}

#[test]
fn measure_tool_selects_the_algorithm_from_the_active_kind() {
    let (mut app, mut session) = application_with_document();
    // Three points would be inferred as an angle by the stateless path, but
    // an explicit polyline tool must measure length (audit F06).
    app.execute(
        &mut session,
        command(
            CommandId::Measure,
            CommandPayload::MeasureTool(MeasurementToolKind::PolylineLength),
        ),
    )
    .unwrap();
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::Points(vec![point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)]),
            ),
        )
        .unwrap();
    // Open-ended tools capture but do not auto-complete.
    assert!(outcome.measurement.is_none());
    assert_eq!(
        session.measurement_preview().map(|p| p.kind),
        Some(MeasurementToolKind::PolylineLength)
    );
    let confirmed = app
        .execute(
            &mut session,
            command(CommandId::ConfirmMeasurement, CommandPayload::None),
        )
        .unwrap();
    let record = confirmed.measurement.expect("structured record");
    assert_eq!(record.algorithm, MeasurementAlgorithm::PolylineLength);
    assert!((record.value - 7.0).abs() < 1e-9);
    assert!(matches!(session.tool, ToolState::Idle));
}

#[test]
fn measure_tool_captures_points_across_commands_and_auto_completes() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::Measure,
            CommandPayload::MeasureTool(MeasurementToolKind::Distance),
        ),
    )
    .unwrap();
    let first = app
        .execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::Points(vec![point(0.0, 0.0)]),
            ),
        )
        .unwrap();
    assert!(first.measurement.is_none());
    let preview = session.measurement_preview().expect("preview");
    assert_eq!(preview.points.len(), 1);
    assert_eq!(preview.remaining, 1);
    assert!(!preview.ready);

    let second = app
        .execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::Points(vec![point(3.0, 4.0)]),
            ),
        )
        .unwrap();
    let record = second.measurement.expect("auto-completed record");
    assert_eq!(record.algorithm, MeasurementAlgorithm::Distance3d);
    assert!((record.value - 5.0).abs() < 1e-9);
    assert!(session.measurement_preview().is_none());
    assert!(matches!(session.tool, ToolState::Idle));
}

#[test]
fn measure_none_starts_a_distance_tool_without_capturing() {
    let (mut app, mut session) = application_with_document();
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::Measure, CommandPayload::None),
        )
        .unwrap();
    assert!(outcome.measurement.is_none());
    let preview = session.measurement_preview().expect("preview");
    assert_eq!(preview.kind, MeasurementToolKind::Distance);
    assert!(preview.points.is_empty());
}

#[test]
fn measure_area_uses_the_viewport_work_plane() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::Measure,
            CommandPayload::MeasureTool(MeasurementToolKind::Area),
        ),
    )
    .unwrap();
    app.execute(
        &mut session,
        command(
            CommandId::Measure,
            CommandPayload::Points(vec![
                point(0.0, 0.0),
                point(2.0, 0.0),
                point(2.0, 3.0),
                point(0.0, 3.0),
            ]),
        ),
    )
    .unwrap();
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::ConfirmMeasurement, CommandPayload::None),
        )
        .unwrap();
    let record = outcome.measurement.expect("area record");
    assert_eq!(record.algorithm, MeasurementAlgorithm::PlanarPolygonArea);
    assert!((record.value - 6.0).abs() < 1e-9);
    assert!(record.plane.is_some());
}

#[test]
fn set_mode_cancels_an_unconfirmed_tool_and_gates_work_commands() {
    let (mut app, mut session) = application_with_document();
    // Start an (unconfirmed) measurement tool in Work mode.
    app.execute(
        &mut session,
        command(
            CommandId::Measure,
            CommandPayload::MeasureTool(MeasurementToolKind::Area),
        ),
    )
    .unwrap();
    app.execute(
        &mut session,
        command(
            CommandId::Measure,
            CommandPayload::Points(vec![point(0.0, 0.0)]),
        ),
    )
    .unwrap();
    assert!(session.measurement_preview().is_some());

    // Switch to Viewer: the unconfirmed tool is cancelled, never committed.
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::SetMode, CommandPayload::Mode(AppMode::Viewer)),
        )
        .unwrap();
    assert!(outcome.changes.is_none());
    assert!(matches!(session.tool, ToolState::Idle));
    assert!(session.measurement_preview().is_none());
    assert_eq!(session.mode(), AppMode::Viewer);
    assert!(!app.can_undo(&DocumentId(1)));

    // A Work-only command is now refused at the command layer.
    assert_eq!(
        app.execute(
            &mut session,
            command(CommandId::Measure, CommandPayload::None),
        )
        .unwrap_err(),
        CadError::PermissionDenied
    );
    assert_eq!(
        app.execute(
            &mut session,
            command(
                CommandId::CreateLine,
                CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
            ),
        )
        .unwrap_err(),
        CadError::PermissionDenied
    );

    // Switching back restores Work and its commands.
    app.execute(
        &mut session,
        command(CommandId::SetMode, CommandPayload::Mode(AppMode::Work)),
    )
    .unwrap();
    assert_eq!(session.mode(), AppMode::Work);
    app.execute(
        &mut session,
        command(CommandId::Measure, CommandPayload::None),
    )
    .unwrap();
}

#[test]
fn set_mode_requires_a_mode_payload() {
    let (mut app, mut session) = application_with_document();
    assert!(matches!(
        app.execute(
            &mut session,
            command(CommandId::SetMode, CommandPayload::None)
        ),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(session.mode(), AppMode::Work);
}

#[test]
fn cancelled_measurement_produces_zero_transactions() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::Measure,
            CommandPayload::MeasureTool(MeasurementToolKind::Angle),
        ),
    )
    .unwrap();
    app.execute(
        &mut session,
        command(
            CommandId::Measure,
            CommandPayload::Points(vec![point(1.0, 0.0)]),
        ),
    )
    .unwrap();
    let revision_before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::CancelMeasurement, CommandPayload::None),
        )
        .unwrap();
    assert!(outcome.changes.is_none());
    assert!(matches!(session.tool, ToolState::Idle));
    assert!(session.measurement_preview().is_none());
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        revision_before
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn confirm_before_enough_points_is_rejected_without_changes() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::Measure,
            CommandPayload::MeasureTool(MeasurementToolKind::Area),
        ),
    )
    .unwrap();
    app.execute(
        &mut session,
        command(
            CommandId::Measure,
            CommandPayload::Points(vec![point(0.0, 0.0)]),
        ),
    )
    .unwrap();
    assert!(matches!(
        app.execute(
            &mut session,
            command(CommandId::ConfirmMeasurement, CommandPayload::None),
        ),
        Err(CadError::InvalidInput(_))
    ));
    // The in-progress tool and its captured point survive the rejection.
    assert_eq!(
        session.measurement_preview().map(|p| p.points.len()),
        Some(1)
    );
}

#[test]
fn undo_to_empty_still_allows_redo_and_refresh_is_pure() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        ),
    )
    .unwrap();
    assert_eq!(
        app.history_availability(&DocumentId(1)),
        HistoryAvailability {
            can_undo: true,
            can_redo: false
        }
    );

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    // Undo to empty: undo disabled, but redo must stay available (U11).
    let availability = app.history_availability(&DocumentId(1));
    assert!(!availability.can_undo);
    assert!(availability.can_redo);
    assert!(!app.can_undo(&DocumentId(1)));
    assert!(app.can_redo(&DocumentId(1)));

    // A pure refresh repeats the same snapshot and changes nothing.
    let entity_count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    assert_eq!(
        app.history_availability(&DocumentId(1)),
        app.history_availability(&DocumentId(1))
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        entity_count
    );

    app.execute(&mut session, command(CommandId::Redo, CommandPayload::None))
        .unwrap();
    let availability = app.history_availability(&DocumentId(1));
    assert!(availability.can_undo);
    assert!(!availability.can_redo);
}

#[test]
fn screen_center_maps_to_camera_target_and_corners_are_symmetric() {
    let mut viewport = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
    viewport.camera.target = Point3 {
        x: 100.0,
        y: 50.0,
        z: 0.0,
    };
    viewport.camera.projection = Projection::Orthographic { scale: 2.0 };

    let center = viewport.screen_to_world([400.0, 300.0], [800.0, 600.0]);
    assert_eq!(
        center,
        Some(Point3 {
            x: 100.0,
            y: 50.0,
            z: 0.0
        })
    );

    // Top-left is left of and above the centre; world y flips relative to
    // the downward-growing screen y.
    let top_left = viewport
        .screen_to_world([0.0, 0.0], [800.0, 600.0])
        .unwrap();
    assert!((top_left.x - (100.0 - 400.0 * 2.0)).abs() < 1e-9);
    assert!((top_left.y - (50.0 + 300.0 * 2.0)).abs() < 1e-9);
}

#[test]
fn screen_to_world_rejects_degenerate_input() {
    let viewport = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
    assert!(viewport
        .screen_to_world([f64::NAN, 0.0], [800.0, 600.0])
        .is_none());
    assert!(viewport.screen_to_world([0.0, 0.0], [0.0, 600.0]).is_none());
    assert!(viewport
        .screen_to_world([0.0, 0.0], [800.0, -1.0])
        .is_none());
}

#[test]
fn switch_2d3d_and_orbit_are_real_transitions() {
    let (mut app, mut session) = application_with_document();
    // Start in 2D orthographic.
    let before = app.workspace.viewports[&ViewportId(1)].camera;
    assert!(before.projection.is_orthographic());

    // Switch to 3D: perspective, still 3D mode.
    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let vp = &app.workspace.viewports[&ViewportId(1)];
    assert!(!vp.camera.projection.is_orthographic());
    assert_eq!(vp.view_mode.kind(), ProjectionKind::ThreeD);

    // Orbit in 3D succeeds and preserves the eye-target distance.
    let d = app.workspace.viewports[&ViewportId(1)].camera.distance();
    app.execute(
        &mut session,
        command(
            CommandId::Orbit,
            CommandPayload::Orbit {
                yaw: 0.3,
                pitch: 0.2,
            },
        ),
    )
    .unwrap();
    let after_orbit = app.workspace.viewports[&ViewportId(1)].camera.distance();
    assert!((after_orbit - d).abs() < 1e-9);

    // Switch back restores the exact 2D camera.
    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let vp = &app.workspace.viewports[&ViewportId(1)];
    assert_eq!(vp.camera, before);
    assert_eq!(vp.view_mode.kind(), ProjectionKind::TwoD);
}

#[test]
fn orbit_outside_3d_is_rejected_without_changing_the_camera() {
    let (mut app, mut session) = application_with_document();
    let before = app.workspace.viewports[&ViewportId(1)].camera;
    let result = app.execute(
        &mut session,
        command(
            CommandId::Orbit,
            CommandPayload::Orbit {
                yaw: 0.1,
                pitch: 0.1,
            },
        ),
    );
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
    assert_eq!(app.workspace.viewports[&ViewportId(1)].camera, before);
}

#[test]
fn orbit_without_payload_is_rejected() {
    let (mut app, mut session) = application_with_document();
    assert!(matches!(
        app.execute(
            &mut session,
            command(CommandId::Orbit, CommandPayload::None)
        ),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn standard_view_is_a_real_projection_and_plan_returns_to_2d() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::StandardView,
            CommandPayload::StandardView(StandardView::Front),
        ),
    )
    .unwrap();
    let vp = &app.workspace.viewports[&ViewportId(1)];
    assert_eq!(vp.view_mode.kind(), ProjectionKind::ThreeD);
    let basis = vp.camera.view_basis().unwrap();
    assert!(basis.is_orthonormal(1e-9));

    // Top (plan) returns to 2D orthographic.
    app.execute(
        &mut session,
        command(
            CommandId::StandardView,
            CommandPayload::StandardView(StandardView::Top),
        ),
    )
    .unwrap();
    let vp = &app.workspace.viewports[&ViewportId(1)];
    assert_eq!(vp.view_mode.kind(), ProjectionKind::TwoD);
    assert!(vp.camera.projection.is_orthographic());
}

#[test]
fn zoom_at_anchors_the_cursor_in_2d() {
    let (mut app, mut session) = application_with_document();
    let cursor = [200.0, 150.0];
    let size = [800.0, 600.0];
    let world_before = app.workspace.viewports[&ViewportId(1)]
        .camera
        .screen_to_plan_world(cursor, size)
        .unwrap();
    app.execute(
        &mut session,
        command(
            CommandId::Zoom,
            CommandPayload::ZoomAt {
                factor: 2.0,
                cursor,
            },
        ),
    )
    .unwrap();
    let world_after = app.workspace.viewports[&ViewportId(1)]
        .camera
        .screen_to_plan_world(cursor, size)
        .unwrap();
    assert!((world_before.x - world_after.x).abs() < 1e-9);
    assert!((world_before.y - world_after.y).abs() < 1e-9);
    // The scale actually changed.
    assert!(app.workspace.viewports[&ViewportId(1)].world_per_px() < 1.0);
}

#[test]
fn work_plane_is_right_handed_and_orthonormal() {
    let plane = cam_xy_work_plane(0.0);
    assert!((cad_app_len(plane.u) - 1.0).abs() < 1e-12);
    assert!((cad_app_len(plane.v) - 1.0).abs() < 1e-12);
    assert!(cad_app_dot(plane.u, plane.v).abs() < 1e-12);
    let n = camera::cross(plane.u, plane.v);
    assert!(n.z > 0.0, "normal points +Z in the plan view");
}

#[test]
fn validate_space_distinguishes_unknown_from_unsupported() {
    let (app, _) = application_with_layouts();
    let db = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(validate_space(db, SpaceSelection::Model).is_ok());
    assert!(validate_space(db, SpaceSelection::Paper(LayoutId(1))).is_ok());
    assert!(matches!(
        validate_space(db, SpaceSelection::Paper(LayoutId(99))),
        Err(CadError::InvalidInput(_))
    ));
    assert!(matches!(
        validate_space(db, SpaceSelection::Paper(LayoutId(2))),
        Err(CadError::Unsupported(_))
    ));
}

#[test]
fn switch_space_accepts_a_supported_layout_and_model_space() {
    let (mut app, mut session) = application_with_layouts();
    assert_eq!(session.active_space, SpaceId::Model);
    app.execute(
        &mut session,
        command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Paper(LayoutId(1))),
        ),
    )
    .unwrap();
    assert_eq!(session.active_space, SpaceId::Paper(LayoutId(1)));
    app.execute(
        &mut session,
        command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Model),
        ),
    )
    .unwrap();
    assert_eq!(session.active_space, SpaceId::Model);
}

#[test]
fn switch_space_to_unknown_layout_is_rejected_and_keeps_the_space() {
    let (mut app, mut session) = application_with_layouts();
    let result = app.execute(
        &mut session,
        command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Paper(LayoutId(99))),
        ),
    );
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
    assert_eq!(session.active_space, SpaceId::Model);
}

#[test]
fn switch_space_to_unsupported_layout_is_explicitly_unsupported() {
    let (mut app, mut session) = application_with_layouts();
    let result = app.execute(
        &mut session,
        command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Paper(LayoutId(2))),
        ),
    );
    assert!(matches!(result, Err(CadError::Unsupported(_))));
    assert_eq!(session.active_space, SpaceId::Model);
}

#[test]
fn block_space_is_not_a_selectable_drawing_space() {
    let (mut app, mut session) = application_with_layouts();
    let result = app.execute(
        &mut session,
        command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Block(BlockId(1))),
        ),
    );
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
    assert_eq!(session.active_space, SpaceId::Model);
}

#[test]
fn switch_space_does_not_mutate_the_drawing_or_open_a_change_set() {
    let (mut app, mut session) = application_with_layouts();
    let before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::SwitchSpace,
                CommandPayload::Space(SpaceId::Paper(LayoutId(1))),
            ),
        )
        .unwrap();
    // A space switch is session state only: no data change, no transaction.
    assert!(outcome.changes.is_none());
    assert!(outcome.objects.is_empty());
    assert_eq!(session.active_space, SpaceId::Paper(LayoutId(1)));
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        before
    );
}

#[test]
fn paper_space_resolves_to_a_planar_paper_distance() {
    let plane = xy_work_plane(0.0);
    let (algorithm, space) = resolve_measurement_space(
        &SpaceId::Paper(LayoutId(1)),
        plane,
        MeasurementAlgorithm::Distance3d,
    )
    .unwrap();
    assert_eq!(algorithm, MeasurementAlgorithm::Distance2d);
    assert!(matches!(space, MeasurementSpace::Paper(LayoutId(1))));

    // An angle is undefined on the 2D sheet and is refused, not guessed.
    assert!(matches!(
        resolve_measurement_space(
            &SpaceId::Paper(LayoutId(1)),
            plane,
            MeasurementAlgorithm::Angle3Points,
        ),
        Err(CadError::Unsupported(_))
    ));

    // Model space keeps the spatial algorithm and 3D space.
    let (algorithm, space) =
        resolve_measurement_space(&SpaceId::Model, plane, MeasurementAlgorithm::Distance3d)
            .unwrap();
    assert_eq!(algorithm, MeasurementAlgorithm::Distance3d);
    assert!(matches!(space, MeasurementSpace::World3d));
}

#[test]
fn viewport_measurement_space_carries_a_verified_inverse() {
    let (app, _) = application_with_layouts();
    let db = &app.workspace.documents[&DocumentId(1)].drawing;
    let space = viewport_measurement_space(db, LayoutId(1), 0).unwrap();
    match space {
        MeasurementSpace::ViewportModel { layout, inverse } => {
            assert_eq!(layout, LayoutId(1));
            // The paper centre maps to the view's model centre (10,20).
            let model = inverse.apply_point(point(50.0, 25.0));
            assert!((model.x - 10.0).abs() < 1e-9, "got {model:?}");
            assert!((model.y - 20.0).abs() < 1e-9, "got {model:?}");
        }
        _ => panic!("expected a viewport model measurement space"),
    }
}

#[test]
fn unsupported_viewport_disables_model_measurement() {
    let (app, _) = application_with_layouts();
    let db = &app.workspace.documents[&DocumentId(1)].drawing;
    // Layout 2's viewport state cannot be represented → no valid inverse.
    let result = viewport_measurement_space(db, LayoutId(2), 0);
    assert!(
        matches!(result, Err(CadError::Unsupported(_))),
        "{result:?}"
    );
    // A missing viewport index is an explicit input error.
    let result = viewport_measurement_space(db, LayoutId(1), 7);
    assert!(
        matches!(result, Err(CadError::InvalidInput(_))),
        "{result:?}"
    );
}

#[test]
fn paper_distance_and_model_distance_are_distinguishable() {
    let (mut app, mut session) = application_with_layouts();
    // Paper space: a 1-unit paper distance on the sheet.
    session.active_space = SpaceId::Paper(LayoutId(1));
    let paper = app
        .execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
            ),
        )
        .unwrap()
        .measurement
        .expect("paper measurement");
    assert!((paper.value - 1.0).abs() < 1e-12, "got {}", paper.value);

    // The same paper picks inside the 1:100 viewport become model distance.
    let db = app.workspace.documents[&DocumentId(1)].drawing.clone();
    let space = viewport_measurement_space(&db, LayoutId(1), 0).unwrap();
    let request = MeasurementRequest {
        algorithm: MeasurementAlgorithm::Distance2d,
        points: vec![point(0.0, 0.0), point(1.0, 0.0)],
        tapped: Vec::new(),
        space,
        units: UnitContext::drawing_units(),
        source: GeometrySource::UserPoints,
        precision: Precision::Analytic,
    };
    let model = app.measurement.measure(&request).unwrap();
    assert!((model.value - 100.0).abs() < 1e-9, "got {}", model.value);
    assert_ne!(paper.value, model.value);
    // Unknown units label the result honestly.
    assert_eq!(model.units.label(), "drawing units");
}

/// Drawing/edit command contract tests (spec F-EDIT, `docs/drawing-edit.md` §2/§3).
mod drawing {
    use super::*;

    fn p3(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }
    use cad_db::{BlockDefinition, DbEntity, DbObject};

    fn ent(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbEntity".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry,
            draw_order: order,
        }
    }

    /// A drawing with layer 0, plus a block (one line child) and an INSERT.
    fn drawing_app() -> (Application, SessionState) {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: cad_domain::BlockId(0),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(ent(
            100,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            },
            0,
        ))
        .unwrap();
        b.insert_entity(ent(
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(10.0, 0.0),
            },
            1,
        ))
        .unwrap();
        b.insert_entity(ent(
            2,
            SemanticGeometry::Arc {
                center: point(0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                radius: 5.0,
                start: 0.0,
                sweep: 1.0,
            },
            2,
        ))
        .unwrap();
        b.insert_entity(ent(
            3,
            SemanticGeometry::Insert {
                block: cad_domain::BlockId(0),
                transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
            },
            3,
        ))
        .unwrap();
        let drawing = b.finish().unwrap();

        let mut app = Application::new();
        app.workspace.documents.insert(
            DocumentId(1),
            Document {
                id: DocumentId(1),
                drawing: Arc::new(drawing),
                identity: DocumentIdentity::Sha256([1u8; 32]),
                units: UnitContext::drawing_units(),
                resource_keys: Vec::new(),
            },
        );
        app.workspace.viewports.insert(
            ViewportId(1),
            Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
        );
        (app, SessionState::new(DocumentId(1), AppMode::Work))
    }

    fn ref_of(entity: u128) -> SelectionRef {
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(entity),
            instance: InstancePath::default(),
            sub_element: None,
        }
    }

    fn execute(
        app: &mut Application,
        session: &mut SessionState,
        id: CommandId,
        payload: CommandPayload,
    ) -> CadResult<CommandOutcome> {
        app.execute(session, command(id, payload))
    }

    fn revision(app: &Application) -> Revision {
        app.workspace.documents[&DocumentId(1)].drawing.revision()
    }

    #[test]
    fn create_line_inserts_one_entity_and_one_undo_step() {
        let (mut app, mut session) = drawing_app();
        let before = revision(&app);
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(0.0, 0.0), point(3.0, 4.0)]),
        )
        .unwrap();
        assert_eq!(outcome.objects.len(), 1);
        let oid = outcome.objects[0];
        let cs = outcome.changes.expect("change set");
        assert_eq!(cs.before, before);
        assert_eq!(cs.after.0, before.0 + 1);
        assert_eq!(cs.changes.len(), 1);
        let entity = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(oid.0))
            .expect("new entity");
        match &entity.geometry {
            SemanticGeometry::Line { start, end } => {
                assert_eq!(*start, point(0.0, 0.0));
                assert_eq!(*end, point(3.0, 4.0));
            }
            other => panic!("expected a line, got {other:?}"),
        }
        assert_eq!(entity.layer, LayerId(0));
        assert_eq!(entity.space, SpaceId::Model);
        assert_eq!(entity.draw_order, 4, "next draw order after the seed set");
        assert!(app.can_undo(&DocumentId(1)));
        assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

        // Undo removes the entity exactly, redo restores it.
        execute(
            &mut app,
            &mut session,
            CommandId::Undo,
            CommandPayload::None,
        )
        .unwrap();
        assert!(app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(oid.0))
            .is_none());
        assert!(app.can_redo(&DocumentId(1)));
        execute(
            &mut app,
            &mut session,
            CommandId::Redo,
            CommandPayload::None,
        )
        .unwrap();
        assert!(app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(oid.0))
            .is_some());
    }

    #[test]
    fn create_circle_uses_edge_distance_and_rejects_zero_radius() {
        let (mut app, mut session) = drawing_app();
        let before = revision(&app);
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::CreateCircle,
            CommandPayload::Points(vec![point(0.0, 0.0), point(3.0, 4.0)]),
        )
        .unwrap();
        let oid = outcome.objects[0];
        match &app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(oid.0))
            .unwrap()
            .geometry
        {
            SemanticGeometry::Circle { radius, .. } => assert!((radius - 5.0).abs() < 1e-12),
            other => panic!("expected a circle, got {other:?}"),
        }
        assert_eq!(revision(&app).0, before.0 + 1);

        // A zero radius is an explicit error and changes nothing.
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateCircle,
            CommandPayload::Points(vec![point(1.0, 1.0), point(1.0, 1.0)]),
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)));
        assert_eq!(revision(&app), rev, "revision unchanged");
        assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    }

    #[test]
    fn create_on_missing_layer_is_refused_without_change() {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        execute(
            &mut app,
            &mut session,
            CommandId::SetActiveLayer,
            CommandPayload::ActiveLayer(LayerId(99)),
        )
        .unwrap();
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 1.0)]),
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)));
        assert_eq!(revision(&app), rev);
        assert!(!app.can_undo(&DocumentId(1)));
    }

    #[test]
    fn move_lines_updates_geometry_and_is_undoable_in_one_step() {
        let (mut app, mut session) = drawing_app();
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::MoveEntities,
            CommandPayload::Move {
                refs: vec![ref_of(1)],
                delta: p3(2.0, 3.0, 0.0),
            },
        )
        .unwrap();
        let cs = outcome.changes.expect("change set");
        assert_eq!(cs.changes.len(), 1, "one transaction for the selection");
        let entity = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .unwrap();
        match &entity.geometry {
            SemanticGeometry::Line { start, end } => {
                assert_eq!(*start, point(2.0, 3.0));
                assert_eq!(*end, point(12.0, 3.0));
            }
            other => panic!("expected a line, got {other:?}"),
        }
        assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

        execute(
            &mut app,
            &mut session,
            CommandId::Undo,
            CommandPayload::None,
        )
        .unwrap();
        match &app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .unwrap()
            .geometry
        {
            SemanticGeometry::Line { start, end } => {
                assert_eq!(*start, point(0.0, 0.0));
                assert_eq!(*end, point(10.0, 0.0));
            }
            other => panic!("expected a line, got {other:?}"),
        }
    }

    #[test]
    fn move_insert_composes_its_transform() {
        let (mut app, mut session) = drawing_app();
        execute(
            &mut app,
            &mut session,
            CommandId::MoveEntities,
            CommandPayload::Move {
                refs: vec![ref_of(3)],
                delta: p3(1.0, 2.0, 0.0),
            },
        )
        .unwrap();
        let entity = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(3))
            .unwrap();
        match &entity.geometry {
            SemanticGeometry::Insert { transform, .. } => {
                // Original translation (5,5,0) composed with (1,2,0) = (6,7,0).
                let moved = transform.apply_point(p3(0.0, 0.0, 0.0));
                assert!((moved.x - 6.0).abs() < 1e-9, "{moved:?}");
                assert!((moved.y - 7.0).abs() < 1e-9, "{moved:?}");
            }
            other => panic!("expected an insert, got {other:?}"),
        }
    }

    #[test]
    fn move_multiple_entities_is_one_undo_step() {
        let (mut app, mut session) = drawing_app();
        execute(
            &mut app,
            &mut session,
            CommandId::MoveEntities,
            CommandPayload::Move {
                refs: vec![ref_of(1), ref_of(2)],
                delta: p3(1.0, 0.0, 0.0),
            },
        )
        .unwrap();
        assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
        execute(
            &mut app,
            &mut session,
            CommandId::Undo,
            CommandPayload::None,
        )
        .unwrap();
        assert!(app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .is_some());
        assert!(app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(2))
            .is_some());
    }

    #[test]
    fn trim_line_against_line_keeps_the_pick_side() {
        let (mut app, mut session) = drawing_app();
        // Target line entity 1: (0,0)->(10,0). Boundary: a vertical line at x=4.
        let boundary = ent(
            50,
            SemanticGeometry::Line {
                start: point(4.0, -1.0),
                end: point(4.0, 1.0),
            },
            5,
        );
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        for e in app.workspace.documents[&DocumentId(1)].drawing.entities() {
            b.insert_entity(e.clone()).unwrap();
        }
        b.insert_entity(boundary).unwrap();
        app.workspace
            .documents
            .get_mut(&DocumentId(1))
            .unwrap()
            .drawing = Arc::new(b.finish().unwrap());

        // Pick on the right half: keep (4,0)->(10,0).
        execute(
            &mut app,
            &mut session,
            CommandId::TrimEntity,
            CommandPayload::Trim {
                target: ref_of(1),
                boundary: vec![ref_of(50)],
                pick_point: point(8.0, 0.0),
            },
        )
        .unwrap();
        match &app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .unwrap()
            .geometry
        {
            SemanticGeometry::Line { start, end } => {
                assert!((start.x - 4.0).abs() < 1e-6, "{start:?}");
                assert!((end.x - 10.0).abs() < 1e-6, "{end:?}");
            }
            other => panic!("expected a line, got {other:?}"),
        }
    }

    #[test]
    fn trim_deletes_a_line_fully_inside_a_closed_boundary() {
        let (mut app, mut session) = drawing_app();
        // A closed square boundary from (0,0) to (10,10), target is a small
        // line entirely inside it.
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(ent(
            1,
            SemanticGeometry::Line {
                start: point(2.0, 2.0),
                end: point(4.0, 2.0),
            },
            0,
        ))
        .unwrap();
        b.insert_entity(ent(
            60,
            SemanticGeometry::Polyline {
                points: vec![
                    point(0.0, 0.0),
                    point(10.0, 0.0),
                    point(10.0, 10.0),
                    point(0.0, 10.0),
                ],
                bulges: vec![0.0, 0.0, 0.0, 0.0],
                closed: true,
            },
            1,
        ))
        .unwrap();
        app.workspace
            .documents
            .get_mut(&DocumentId(1))
            .unwrap()
            .drawing = Arc::new(b.finish().unwrap());

        execute(
            &mut app,
            &mut session,
            CommandId::TrimEntity,
            CommandPayload::Trim {
                target: ref_of(1),
                boundary: vec![ref_of(60)],
                pick_point: point(3.0, 2.0),
            },
        )
        .unwrap();
        assert!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity(EntityId(1))
                .is_none(),
            "fully contained line is removed"
        );

        // Undo restores the deleted line.
        execute(
            &mut app,
            &mut session,
            CommandId::Undo,
            CommandPayload::None,
        )
        .unwrap();
        assert!(app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .is_some());
    }

    #[test]
    fn trim_arc_target_is_unsupported_and_changes_nothing() {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let depth = app
            .history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::TrimEntity,
            CommandPayload::Trim {
                target: ref_of(2),
                boundary: vec![ref_of(1)],
                pick_point: point(1.0, 0.0),
            },
        )
        .unwrap_err();
        assert!(matches!(err, CadError::Unsupported(_)));
        assert_eq!(revision(&app), rev);
        assert_eq!(
            app.history
                .get(&DocumentId(1))
                .map(|h| h.undo_depth())
                .unwrap_or(0),
            depth
        );
    }

    #[test]
    fn trim_with_no_intersection_is_reported_and_changes_nothing() {
        let (mut app, mut session) = drawing_app();
        // A parallel boundary far away: no cut, no containment.
        let boundary = ent(
            70,
            SemanticGeometry::Line {
                start: point(0.0, 5.0),
                end: point(10.0, 5.0),
            },
            5,
        );
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        for e in app.workspace.documents[&DocumentId(1)].drawing.entities() {
            b.insert_entity(e.clone()).unwrap();
        }
        b.insert_entity(boundary).unwrap();
        app.workspace
            .documents
            .get_mut(&DocumentId(1))
            .unwrap()
            .drawing = Arc::new(b.finish().unwrap());
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::TrimEntity,
            CommandPayload::Trim {
                target: ref_of(1),
                boundary: vec![ref_of(70)],
                pick_point: point(5.0, 0.0),
            },
        )
        .unwrap_err();
        assert!(matches!(err, CadError::Unsupported(_)));
        assert_eq!(revision(&app), rev);
    }

    #[test]
    fn viewer_mode_rejects_all_drawing_commands_without_change() {
        let (mut app, mut session) = drawing_app();
        let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
        let rev = revision(&app);
        let cases = [
            (
                CommandId::CreateLine,
                CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
            ),
            (
                CommandId::CreateCircle,
                CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
            ),
            (
                CommandId::MoveEntities,
                CommandPayload::Move {
                    refs: vec![ref_of(1)],
                    delta: p3(1.0, 0.0, 0.0),
                },
            ),
            (
                CommandId::TrimEntity,
                CommandPayload::Trim {
                    target: ref_of(1),
                    boundary: vec![ref_of(2)],
                    pick_point: point(1.0, 0.0),
                },
            ),
        ];
        for (id, payload) in cases {
            let err = execute(&mut app, &mut viewer, id, payload).unwrap_err();
            assert_eq!(err, CadError::PermissionDenied, "{id:?}");
        }
        assert_eq!(revision(&app), rev);
        assert!(!app.can_undo(&DocumentId(1)));
        // Silence the unused-mut warning on the shared session.
        let _ = &mut session;
    }

    #[test]
    fn drawing_undo_redo_round_trips() {
        let (mut app, mut session) = drawing_app();
        let before = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count();
        // One drawing create, then a second, then undo both in reverse order.
        execute(
            &mut app,
            &mut session,
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 1.0)]),
        )
        .unwrap();
        let oid = app.workspace.documents[&DocumentId(1)]
            .drawing
            .model_space()
            .iter()
            .map(|e| e.id)
            .max()
            .unwrap();
        execute(
            &mut app,
            &mut session,
            CommandId::CreateCircle,
            CommandPayload::Points(vec![point(5.0, 5.0), point(6.0, 5.0)]),
        )
        .unwrap();
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity_count(),
            before + 2
        );

        // Undo the circle first, then the line create.
        execute(
            &mut app,
            &mut session,
            CommandId::Undo,
            CommandPayload::None,
        )
        .unwrap();
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity_count(),
            before + 1
        );
        execute(
            &mut app,
            &mut session,
            CommandId::Undo,
            CommandPayload::None,
        )
        .unwrap();
        assert!(app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(oid)
            .is_none());
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity_count(),
            before
        );

        // Redo restores them in order.
        execute(
            &mut app,
            &mut session,
            CommandId::Redo,
            CommandPayload::None,
        )
        .unwrap();
        assert!(app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(oid)
            .is_some());
        execute(
            &mut app,
            &mut session,
            CommandId::Redo,
            CommandPayload::None,
        )
        .unwrap();
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity_count(),
            before + 2
        );
    }

    #[test]
    fn drawing_commands_are_work_only_in_the_catalogue() {
        for id in [
            CommandId::CreateLine,
            CommandId::CreateCircle,
            CommandId::MoveEntities,
            CommandId::TrimEntity,
            CommandId::SetActiveLayer,
        ] {
            assert!(id.requires_work_mode(), "{id:?} must be Work-only");
        }
    }

    #[test]
    fn two_creates_are_two_transactions_not_one_or_zero() {
        let (mut app, mut session) = drawing_app();
        for _ in 0..2 {
            execute(
                &mut app,
                &mut session,
                CommandId::CreateLine,
                CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
            )
            .unwrap();
        }
        // Each command is exactly one transaction and one undo step: never a
        // merged pair (one step) and never a silent empty success (zero steps).
        assert_eq!(app.history[&DocumentId(1)].undo_depth(), 2);
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .model_space()
                .len(),
            6,
            "the five seed model entities plus two created lines"
        );
    }

    #[test]
    fn empty_move_selection_is_refused_without_change() {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::MoveEntities,
            CommandPayload::Move {
                refs: Vec::new(),
                delta: p3(1.0, 0.0, 0.0),
            },
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)));
        assert_eq!(revision(&app), rev);
        assert!(!app.can_undo(&DocumentId(1)));
    }
}
