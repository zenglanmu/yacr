use super::*;

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
fn measure_none_remains_distance_after_an_area_tool_completed() {
    let (mut app, mut session) = application_with_document();
    session
        .set_measurement_kind(MeasurementToolKind::Area)
        .unwrap();
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
            CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0), point(2.0, 3.0)]),
        ),
    )
    .unwrap();
    let area = app
        .execute(
            &mut session,
            command(CommandId::ConfirmMeasurement, CommandPayload::None),
        )
        .unwrap();
    assert_eq!(
        area.measurement.unwrap().algorithm,
        MeasurementAlgorithm::PlanarPolygonArea
    );

    app.execute(
        &mut session,
        command(CommandId::Measure, CommandPayload::None),
    )
    .unwrap();
    assert_eq!(
        session.measurement_preview().unwrap().kind,
        MeasurementToolKind::Distance
    );
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
