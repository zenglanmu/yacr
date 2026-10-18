use super::*;

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
