use super::*;

/// The active viewport's camera.
fn camera(app: &Application) -> Camera {
    app.workspace.viewports[&ViewportId(1)].camera
}

/// The active viewport's orthographic world-units-per-pixel scale.
fn ortho_scale(app: &Application) -> f64 {
    match camera(app).projection {
        Projection::Orthographic { scale } => scale,
        Projection::Perspective { .. } => panic!("expected the 2D plan projection"),
    }
}

fn line_entity(id: u128, space: SpaceId, start: Point3, end: Point3) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Line { start, end },
        draw_order: 0,
    }
}

/// Two drawable sheets plus model geometry. Model bounds are 400×300; sheet 1
/// has a viewport rectangle 0..100 × 0..50 **and** a paper entity extending it
/// to 300×100, so the paper fit proves both sources are framed.
fn application_with_spaces() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(line_entity(
        1,
        SpaceId::Model,
        point(0.0, 0.0),
        point(400.0, 300.0),
    ))
    .unwrap();
    let sheet_viewport = || PaperViewport {
        clip: vec![
            point(0.0, 0.0),
            point(100.0, 0.0),
            point(100.0, 50.0),
            point(0.0, 50.0),
        ],
        model_to_paper: Transform3::scale(100.0),
        completeness: Completeness::Complete,
        layer_overrides: std::collections::BTreeMap::new(),
    };
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: vec![sheet_viewport()],
    })
    .unwrap();
    b.insert_entity(line_entity(
        2,
        SpaceId::Paper(LayoutId(1)),
        point(200.0, 0.0),
        point(300.0, 100.0),
    ))
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(2),
        name: "Sheet2".into(),
        viewports: vec![sheet_viewport()],
    })
    .unwrap();

    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([0u8; 32]),
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

/// One drawable-but-empty sheet whose stored plot size is unusable (zero), so
/// neither paper geometry nor a paper-size fallback can frame it.
fn application_with_unframable_layout() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "EmptySheet".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.set_plot_settings(cad_db::PlotSettingsRecord {
        layout: LayoutId(1),
        paper_size_name: "unusable".into(),
        paper_width: 0.0,
        paper_height: 0.0,
        margins: cad_db::PlotMargins::default(),
        rotation: cad_db::PlotRotation::None,
        scale_numerator: 1.0,
        scale_denominator: 1.0,
        plot_type: cad_db::PlotType::Layout,
        paper_units: cad_db::PlotPaperUnits::Millimeters,
        provenance: cad_db::PlotProvenance::Imported,
    })
    .unwrap();

    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([0u8; 32]),
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
fn switch_space_fits_the_target_space_then_model_again() {
    let (mut app, mut session) = application_with_spaces();
    // Model: 400×300 framed in 800×600 with the FIT margin.
    let model_scale = 0.5 * FIT_MARGIN;
    assert!((ortho_scale(&app) - 1.0).abs() < 1e-12);

    app.execute(
        &mut session,
        command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Paper(LayoutId(1))),
        ),
    )
    .unwrap();
    assert_eq!(session.active_space, SpaceId::Paper(LayoutId(1)));
    let paper = camera(&app);
    // Paper bounds: viewport 0..100×0..50 plus the paper entity 200..300×0..100
    // => 300×100, centred at (150, 50).
    assert!((paper.target.x - 150.0).abs() < 1e-9, "{paper:?}");
    assert!((paper.target.y - 50.0).abs() < 1e-9, "{paper:?}");
    assert!(
        (ortho_scale(&app) - 0.375 * FIT_MARGIN).abs() < 1e-12,
        "paper scale {} != {}",
        ortho_scale(&app),
        0.375 * FIT_MARGIN
    );

    app.execute(
        &mut session,
        command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Model),
        ),
    )
    .unwrap();
    assert_eq!(session.active_space, SpaceId::Model);
    let model = camera(&app);
    assert!((model.target.x - 200.0).abs() < 1e-9, "{model:?}");
    assert!((model.target.y - 150.0).abs() < 1e-9, "{model:?}");
    assert!(
        (ortho_scale(&app) - model_scale).abs() < 1e-12,
        "model scale {} != {model_scale}",
        ortho_scale(&app)
    );
}

#[test]
fn switch_space_to_an_unframable_layout_reports_fit_empty_and_keeps_the_camera() {
    let (mut app, mut session) = application_with_unframable_layout();
    let before = camera(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::SwitchSpace,
                CommandPayload::Space(SpaceId::Paper(LayoutId(1))),
            ),
        )
        .unwrap();
    assert_eq!(session.active_space, SpaceId::Paper(LayoutId(1)));
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "layout.fit_empty"),
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(
        camera(&app),
        before,
        "an empty sheet must not move the camera"
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
