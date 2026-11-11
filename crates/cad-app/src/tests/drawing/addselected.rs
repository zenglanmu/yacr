//! `ADDSELECTED` contract tests.
//!
//! The command creates one new entity of the single selected entity's type and
//! general properties (layer, stored `type_key`, colour/linetype/lineweight/
//! transparency) at the typed point, in one transaction / one undo step.
//! Point-anchored `LINE`/`CIRCLE`/`POINT`/`TEXT` are supported; every other kind
//! is the named `addselected.unsupported` refusal.

use super::*;
use crate::command_payload::AddSelectedCommand;

/// Layer `0` + `WALLS` (current), one source row with non-default render
/// attributes, and a viewport.
fn addselected_app(geometry: SemanticGeometry, space: SpaceId) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    if let SpaceId::Paper(layout) = space {
        b.insert_layout(Layout {
            id: layout,
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    }
    let source = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: match geometry {
                SemanticGeometry::Text { .. } => "AcDbMText",
                SemanticGeometry::Circle { .. } => "AcDbCircle",
                SemanticGeometry::Point(_) => "AcDbPoint",
                _ => "AcDbLine",
            }
            .into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(7),
        space,
        geometry,
        draw_order: 0,
    };
    b.insert_entity(source).unwrap();
    b.set_entity_render_attributes(EntityId(1), non_default_attributes())
        .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([41u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    session.active_layer = LayerId(0);
    session.selection = SelectionSet::new();
    session.selection.insert(SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    });
    (app, session)
}

fn drawing(app: &Application) -> &DrawingDatabase {
    app.workspace.documents[&DocumentId(1)].drawing.as_ref()
}

fn add_selected(position: Point3) -> CommandPayload {
    CommandPayload::AddSelected(Box::new(AddSelectedCommand { position }))
}

fn text_geometry(type_key_font: Option<&str>) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "HELLO".into(),
        position: p3(1.0, 2.0, 0.0),
        style: cad_domain::StyleId(0),
        height: 3.0,
        rotation: 0.5,
        font: type_key_font.map(str::to_string),
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width: None,
    }
}

#[test]
fn addselected_copies_a_line_with_its_layer_type_key_and_display_attributes() {
    let (mut app, mut session) = addselected_app(
        SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(10.0, 2.0, 0.0),
        },
        SpaceId::Model,
    );
    let before = revision(&app);
    let source_attributes = drawing(&app).entity_render_attributes(EntityId(1));
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AddSelected),
        add_selected(p3(5.0, 5.0, 0.0)),
    )
    .expect("ADDSELECTED copies the line");
    assert_eq!(outcome.objects.len(), 1);
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.after.0, before.0 + 1, "one revision bump");
    // One transaction: the insert plus the funnel's captured display-state
    // update for the same row (the attribute-preserving path).
    assert_eq!(
        changes
            .changes
            .iter()
            .filter(|change| matches!(change, cad_db::ObjectChange::Insert(_)))
            .count(),
        1
    );

    let entity = drawing(&app)
        .entity(EntityId(outcome.objects[0].0))
        .expect("the new line");
    match &entity.geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, p3(5.0, 5.0, 0.0));
            assert_eq!(*end, p3(15.0, 7.0, 0.0), "the source direction is kept");
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
    assert_eq!(
        entity.layer,
        LayerId(7),
        "the source layer, not the active one"
    );
    assert_eq!(entity.object.type_key, "AcDbLine");
    assert_eq!(
        drawing(&app).entity_render_attributes(entity.id),
        source_attributes,
        "colour/linetype/lineweight/transparency are preserved"
    );
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
}

#[test]
fn addselected_copies_a_circle_point_and_text_geometry() {
    // CIRCLE: same radius/normal, new centre.
    let (mut app, mut session) = addselected_app(
        SemanticGeometry::Circle {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 4.0,
        },
        SpaceId::Model,
    );
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AddSelected),
        add_selected(p3(20.0, 0.0, 0.0)),
    )
    .expect("circle copy");
    match &drawing(&app)
        .entity(EntityId(outcome.objects[0].0))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            assert_eq!(*center, p3(20.0, 0.0, 0.0));
            assert_eq!(*normal, p3(0.0, 0.0, 1.0));
            assert_eq!(*radius, 4.0);
        }
        other => panic!("expected a CIRCLE, got {other:?}"),
    }

    // POINT: the typed position.
    let (mut app, mut session) =
        addselected_app(SemanticGeometry::Point(p3(1.0, 1.0, 0.0)), SpaceId::Model);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AddSelected),
        add_selected(p3(-3.0, 7.0, 0.0)),
    )
    .expect("point copy");
    assert!(matches!(
        drawing(&app)
            .entity(EntityId(outcome.objects[0].0))
            .unwrap()
            .geometry,
        SemanticGeometry::Point(p) if p == p3(-3.0, 7.0, 0.0)
    ));

    // TEXT: everything but the insertion point is preserved, including the
    // stored AcDbMText type key of a multiline source.
    let (mut app, mut session) =
        addselected_app(text_geometry(Some("osifont.ttf")), SpaceId::Model);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AddSelected),
        add_selected(p3(9.0, 9.0, 0.0)),
    )
    .expect("text copy");
    let entity = drawing(&app)
        .entity(EntityId(outcome.objects[0].0))
        .expect("the new text");
    assert_eq!(entity.object.type_key, "AcDbMText");
    match &entity.geometry {
        SemanticGeometry::Text {
            text,
            position,
            height,
            rotation,
            font,
            ..
        } => {
            assert_eq!(text, "HELLO");
            assert_eq!(*position, p3(9.0, 9.0, 0.0));
            assert_eq!(*height, 3.0);
            assert_eq!(*rotation, 0.5);
            assert_eq!(font.as_deref(), Some("osifont.ttf"));
        }
        other => panic!("expected TEXT, got {other:?}"),
    }
}

#[test]
fn addselected_refuses_richer_types_and_bad_input_without_writing() {
    let expect_refusal =
        |app: &mut Application, session: &mut SessionState, payload: CommandPayload, code: &str| {
            let before = revision(app);
            let error = execute(
                app,
                session,
                CommandId::Extended(ExtendedCommandId::AddSelected),
                payload,
            )
            .expect_err("refused");
            assert!(error.to_string().contains(code), "{error}");
            assert_eq!(revision(app), before, "{code} writes nothing");
            assert!(!app.can_undo(&DocumentId(1)), "{code} records no history");
        };

    // ARC needs a sweep and is not point-anchored.
    let (mut app, mut session) = addselected_app(
        SemanticGeometry::Arc {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 2.0,
            start: 0.0,
            sweep: 1.0,
        },
        SpaceId::Model,
    );
    expect_refusal(
        &mut app,
        &mut session,
        add_selected(p3(0.0, 0.0, 0.0)),
        "addselected.unsupported",
    );

    // Non-finite placement.
    let (mut app, mut session) =
        addselected_app(SemanticGeometry::Point(p3(0.0, 0.0, 0.0)), SpaceId::Model);
    expect_refusal(
        &mut app,
        &mut session,
        add_selected(p3(f64::NAN, 0.0, 0.0)),
        "addselected.invalid_point",
    );

    // Empty selection.
    session.selection = SelectionSet::new();
    expect_refusal(
        &mut app,
        &mut session,
        add_selected(p3(1.0, 1.0, 0.0)),
        "addselected.no_selection",
    );

    // Two selected rows.
    session.selection.insert(SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    });
    session.selection.insert(SelectionRef {
        document: DocumentId(1),
        entity: EntityId(2),
        instance: InstancePath::default(),
        sub_element: None,
    });
    expect_refusal(
        &mut app,
        &mut session,
        add_selected(p3(1.0, 1.0, 0.0)),
        "addselected.multiple",
    );

    // A paper-space source is not copied into model space.
    let (mut app, mut session) = addselected_app(
        SemanticGeometry::Point(p3(0.0, 0.0, 0.0)),
        SpaceId::Paper(cad_domain::LayoutId(0)),
    );
    expect_refusal(
        &mut app,
        &mut session,
        add_selected(p3(1.0, 1.0, 0.0)),
        "addselected.not_model_space",
    );

    // A wrong payload is refused before anything else.
    let (mut app, mut session) =
        addselected_app(SemanticGeometry::Point(p3(0.0, 0.0, 0.0)), SpaceId::Model);
    expect_refusal(
        &mut app,
        &mut session,
        CommandPayload::None,
        "addselected.payload",
    );
}
