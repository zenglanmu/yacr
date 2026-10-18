use super::*;

#[test]
fn erase_then_undo_restores_geometry_and_display_state() {
    let attributes = non_default_attributes();
    let mut app = single_line_app(Some(attributes.clone()));
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_none());
    // The delete pruned the side structures, so a naive undo would lose them.
    assert_eq!(
        drawing.entity_render_attributes(EntityId(1)),
        cad_db::EntityRenderAttributes::default()
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let restored = drawing.entity(EntityId(1)).expect("restored entity");
    match &restored.geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(0.0, 0.0));
            assert_eq!(*end, point(10.0, 0.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
    assert_eq!(restored.layer, LayerId(0));
    assert_eq!(
        drawing.entity_render_attributes(EntityId(1)),
        attributes,
        "undo must restore the render attributes"
    );
}

#[test]
fn copy_reports_the_uncarried_style_override() {
    let mut app = single_line_app(Some(non_default_attributes()));
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CopyEntities,
        CommandPayload::Copy {
            refs: vec![ref_of(1)],
            delta: p3(5.0, 0.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(outcome.diagnostics.len(), 1, "{:?}", outcome.diagnostics);
    assert_eq!(outcome.diagnostics[0].code, "drawing.copy.style");
    let copy_id = EntityId(outcome.objects[0].0);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing.entity_render_attributes(copy_id),
        cad_db::EntityRenderAttributes::default(),
        "the copy inherits the layer style, and that is reported"
    );
}

#[test]
fn erase_reports_sub_element_and_instance_refs() {
    let (mut app, mut session) = drawing_app();
    let mut sub = ref_of(1);
    sub.sub_element = Some(cad_domain::SubElementId {
        source_key: "edge:0".into(),
        topology_revision: Revision(0),
    });
    let mut inst = ref_of(2);
    inst.instance = cad_domain::InstancePath(vec![EntityId(3)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![sub, inst]),
    )
    .unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .unwrap();
    let codes: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|d| d.code.as_str())
        .collect();
    assert!(codes.contains(&"drawing.erase.sub_element"), "{codes:?}");
    assert!(codes.contains(&"drawing.erase.instance"), "{codes:?}");
}

#[test]
fn copy_reports_sub_element_and_instance_refs() {
    let (mut app, mut session) = drawing_app();
    let mut sub = ref_of(1);
    sub.sub_element = Some(cad_domain::SubElementId {
        source_key: "edge:0".into(),
        topology_revision: Revision(0),
    });
    let mut inst = ref_of(2);
    inst.instance = cad_domain::InstancePath(vec![EntityId(3)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CopyEntities,
        CommandPayload::Copy {
            refs: vec![sub, inst],
            delta: p3(1.0, 1.0, 0.0),
        },
    )
    .unwrap();
    let codes: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|d| d.code.as_str())
        .collect();
    assert!(codes.contains(&"drawing.copy.sub_element"), "{codes:?}");
    assert!(codes.contains(&"drawing.copy.instance"), "{codes:?}");
}

#[test]
fn erase_with_a_payload_is_refused_without_change() {
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .unwrap();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .is_some());
}

#[test]
fn image_and_mask_class_names_match_the_importer_keys() {
    let image = SemanticGeometry::Image {
        origin: point(0.0, 0.0),
        u: p3(1.0, 0.0, 0.0),
        v: p3(0.0, 1.0, 0.0),
        pixels: [1.0, 1.0],
        file: None,
        clip: None,
        visible: true,
    };
    let mask = SemanticGeometry::Mask {
        boundary: vec![point(0.0, 0.0), point(1.0, 0.0), point(1.0, 1.0)],
        inverted: false,
    };
    assert_eq!(crate::app_drawing::class_name(&image), "AcDbRasterImage");
    assert_eq!(crate::app_drawing::class_name(&mask), "AcDbWipeout");
}
