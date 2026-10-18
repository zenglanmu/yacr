use super::*;

#[test]
fn rotate_about_a_base_rotates_in_place_and_undoes() {
    let (mut app, mut session) = drawing_app();
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    execute(
        &mut app,
        &mut session,
        CommandId::RotateEntities,
        CommandPayload::Rotate {
            refs: vec![ref_of(1)],
            base: point(0.0, 0.0),
            angle: std::f64::consts::FRAC_PI_2,
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
            assert!(start.x.abs() < 1e-9 && start.y.abs() < 1e-9, "{start:?}");
            assert!(end.x.abs() < 1e-9 && (end.y - 10.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
    // In place: no entity added or removed.
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before
    );
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
            assert!(
                (start.x).abs() < 1e-9 && (start.y).abs() < 1e-9,
                "{start:?}"
            );
            assert!(
                (end.x - 10.0).abs() < 1e-9 && (end.y).abs() < 1e-9,
                "{end:?}"
            );
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn rotate_with_an_empty_selection_is_refused_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::RotateEntities,
        CommandPayload::Rotate {
            refs: Vec::new(),
            base: point(0.0, 0.0),
            angle: 1.0,
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}

#[test]
fn scale_about_a_base_doubles_distances_and_undoes() {
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::ScaleEntities,
        CommandPayload::Scale {
            refs: vec![ref_of(1)],
            base: point(0.0, 0.0),
            factor: 2.0,
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
            assert!(start.x.abs() < 1e-9, "{start:?}");
            assert!((end.x - 20.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
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
        SemanticGeometry::Line { end, .. } => assert!((end.x - 10.0).abs() < 1e-9),
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn scale_with_a_non_positive_factor_is_refused_without_change() {
    let (mut app, mut session) = drawing_app();
    for factor in [0.0, -2.0, f64::INFINITY, f64::NAN] {
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::ScaleEntities,
            CommandPayload::Scale {
                refs: vec![ref_of(1)],
                base: point(0.0, 0.0),
                factor,
            },
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)), "{factor}");
        assert_eq!(revision(&app), rev);
    }
}

#[test]
fn mirror_across_a_diagonal_axis_keeps_the_original_and_adds_a_copy() {
    let (mut app, mut session) = drawing_app();
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(1)],
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .unwrap();
    // One copy added; the source is untouched.
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 1
    );
    let copy_id = EntityId(outcome.objects[0].0);
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(copy_id)
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert!(start.x.abs() < 1e-9 && start.y.abs() < 1e-9, "{start:?}");
            assert!(end.x.abs() < 1e-9 && (end.y - 10.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { end, .. } => assert!((end.x - 10.0).abs() < 1e-9),
        other => panic!("expected a line, got {other:?}"),
    }
    // One undo removes every mirrored copy and leaves the source.
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
        before
    );
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(copy_id)
        .is_none());
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .is_some());
}

#[test]
fn mirror_refuses_an_arc_or_ellipse_without_change() {
    // Arc target (entity 2 of the shared drawing) is not representable under
    // a reflection and must refuse the whole command.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(2)],
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)));
    assert_eq!(revision(&app), rev);

    // Ellipse target: same explicit refusal.
    let ellipse = ent(
        3,
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(2.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
        0,
    );
    let mut app = line_and_boundary_app(ellipse, None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(3)],
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)));
    assert_eq!(revision(&app), rev);
}

#[test]
fn transform_refs_report_sub_element_and_instance_diagnostics() {
    let (mut app, mut session) = drawing_app();
    let mut sub = ref_of(1);
    sub.sub_element = Some(cad_domain::SubElementId {
        source_key: "edge:0".into(),
        topology_revision: Revision(0),
    });
    let mut inst = ref_of(3);
    inst.instance = cad_domain::InstancePath(vec![EntityId(3)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::RotateEntities,
        CommandPayload::Rotate {
            refs: vec![sub, inst],
            base: point(0.0, 0.0),
            angle: 0.5,
        },
    )
    .unwrap();
    let codes: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|d| d.code.as_str())
        .collect();
    assert!(codes.contains(&"drawing.rotate.sub_element"), "{codes:?}");
    assert!(codes.contains(&"drawing.rotate.instance"), "{codes:?}");
}

#[test]
fn rotate_refuses_a_mixed_selection_without_change() {
    let mut app = line_and_opaque_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .cloned()
        .unwrap();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::RotateEntities,
        CommandPayload::Rotate {
            refs: vec![ref_of(1), ref_of(2)],
            base: point(0.0, 0.0),
            angle: std::f64::consts::FRAC_PI_2,
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)));
    assert_eq!(revision(&app), rev);
    // The supported line is byte-identical: never dropped, never half-applied.
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .cloned()
            .unwrap(),
        before
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn mirror_refuses_a_mixed_selection_without_change() {
    let mut app = line_and_opaque_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .cloned()
        .unwrap();
    let count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(1), ref_of(2)],
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)));
    assert_eq!(revision(&app), rev);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        count,
        "no mirror copy may be added when one geometry is unsupported"
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .cloned()
            .unwrap(),
        before
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn rotate_and_scale_refuse_an_unsupported_kind() {
    let mut app = line_and_opaque_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    for (id, payload) in [
        (
            CommandId::RotateEntities,
            CommandPayload::Rotate {
                refs: vec![ref_of(2)],
                base: point(0.0, 0.0),
                angle: 1.0,
            },
        ),
        (
            CommandId::ScaleEntities,
            CommandPayload::Scale {
                refs: vec![ref_of(2)],
                base: point(0.0, 0.0),
                factor: 2.0,
            },
        ),
    ] {
        let rev = revision(&app);
        let err = execute(&mut app, &mut session, id, payload).unwrap_err();
        assert!(matches!(err, CadError::Unsupported(_)), "{id:?}");
        assert_eq!(revision(&app), rev);
    }
}

#[test]
fn mirror_of_multiple_refs_is_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(1), ref_of(3)],
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 2
    );
    // One undo removes both mirrored copies and restores the count.
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
        before
    );
    for object in &outcome.objects {
        assert!(app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(object.0))
            .is_none());
    }
}

#[test]
fn rotate_about_a_non_origin_base_conjugates() {
    let (mut app, mut session) = drawing_app();
    // 90° about (1,0): (0,0) -> (1,-1); (10,0) -> (1,9).
    execute(
        &mut app,
        &mut session,
        CommandId::RotateEntities,
        CommandPayload::Rotate {
            refs: vec![ref_of(1)],
            base: point(1.0, 0.0),
            angle: std::f64::consts::FRAC_PI_2,
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
            assert!(
                (start.x - 1.0).abs() < 1e-9 && (start.y + 1.0).abs() < 1e-9,
                "{start:?}"
            );
            assert!(
                (end.x - 1.0).abs() < 1e-9 && (end.y - 9.0).abs() < 1e-9,
                "{end:?}"
            );
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn mirror_across_a_non_origin_axis() {
    let (mut app, mut session) = drawing_app();
    // Reflection across the vertical line x = 1: x -> 2 - x, so the copied
    // segment (0,0)-(10,0) becomes (2,0)-(-8,0). The original is retained.
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(1)],
            axis_start: point(1.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .unwrap();
    let copy_id = EntityId(outcome.objects[0].0);
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(copy_id)
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert!(
                (start.x - 2.0).abs() < 1e-9 && start.y.abs() < 1e-9,
                "{start:?}"
            );
            assert!((end.x + 8.0).abs() < 1e-9 && end.y.abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
    // The original is untouched.
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { end, .. } => assert!((end.x - 10.0).abs() < 1e-9),
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn scale_with_an_empty_selection_is_refused_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::ScaleEntities,
        CommandPayload::Scale {
            refs: Vec::new(),
            base: point(0.0, 0.0),
            factor: 2.0,
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}

#[test]
fn mirror_with_an_empty_selection_is_refused_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: Vec::new(),
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}

#[test]
fn mirror_reports_the_uncarried_style_override() {
    let mut app = single_line_app(Some(non_default_attributes()));
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MirrorEntities,
        CommandPayload::Mirror {
            refs: vec![ref_of(1)],
            axis_start: point(0.0, 0.0),
            axis_end: point(1.0, 1.0),
        },
    )
    .unwrap();
    assert_eq!(outcome.diagnostics.len(), 1, "{:?}", outcome.diagnostics);
    assert_eq!(outcome.diagnostics[0].code, "drawing.mirror.style");
    let copy_id = EntityId(outcome.objects[0].0);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(copy_id),
        cad_db::EntityRenderAttributes::default(),
        "the copy inherits the layer style, and that is reported"
    );
}
