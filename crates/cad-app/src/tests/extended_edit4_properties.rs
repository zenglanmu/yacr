//! Typed-property and FLATTEN contracts of the Wave 4 edit lane.
//!
//! Split out of `extended_edit4.rs` (the 1000-line code gate) when `CHPROP` /
//! `CHANGE` gained their typed property list and `FLATTEN` gained the upright
//! INSERT and point-placed 2D cases. Every test asserts the stored state, exactly
//! one transaction / one undo step for a write, and that a refusal writes
//! nothing (no revision bump, no history entry).

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor,
    EntityRenderAttributes, Layer, Layout,
};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(start: Point3, end: Point3) -> SemanticGeometry {
    SemanticGeometry::Line { start, end }
}

fn insert(block: u128, transform: Transform3) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: BlockId(block),
        transform,
        attributes: Vec::new(),
    }
}

fn part(
    id: u128,
    geometry: SemanticGeometry,
    layer: LayerId,
    space: SpaceId,
    type_key: &str,
    order: i64,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space,
        geometry,
        draw_order: order,
    }
}

fn drawing_of(app: &Application) -> &DrawingDatabase {
    &app.workspace.documents[&DocumentId(1)].drawing
}

fn revision(app: &Application) -> Revision {
    drawing_of(app).revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn model_count(app: &Application) -> usize {
    drawing_of(app).model_space().len()
}

fn geometry_of(app: &Application, id: u128) -> SemanticGeometry {
    drawing_of(app)
        .entity(EntityId(id))
        .expect("entity must exist")
        .geometry
        .clone()
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Extended(ext), CommandPayload::None),
    )
}

fn run_payload(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::Extended(ext), payload))
}

fn build(entities: Vec<DbEntity>, blocks: Vec<BlockDefinition>) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for block in blocks {
        b.insert_block(block).unwrap();
    }
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
}

/// Block 1 has one LINE member (0,0)-(1,0) and entity 1 is an INSERT of it.
fn block_app(transform: Transform3) -> (Application, SessionState) {
    build(
        vec![
            part(
                100,
                line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbLine",
                0,
            ),
            part(
                1,
                insert(1, transform),
                LayerId(0),
                SpaceId::Model,
                "AcDbBlockReference",
                1,
            ),
        ],
        vec![BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        }],
    )
}

#[test]
fn flatten_moves_an_upright_insert_insertion_z_to_zero() {
    let (mut app, mut session) = block_app(Transform3::translation(p3(5.0, 5.0, 3.0)));
    select(&mut session, &[1]);
    run(&mut app, &mut session, ExtendedCommandId::Flatten).unwrap();
    assert_eq!(undo_depth(&app), 1);
    let entity = drawing_of(&app).entity(EntityId(1)).unwrap().clone();
    let SemanticGeometry::Insert { transform, .. } = entity.geometry else {
        panic!("the reference stays an INSERT");
    };
    assert_eq!(transform.matrix[2][3], 0.0, "the insertion z is zeroed");
    assert_eq!(entity.object.type_key, "AcDbBlockReference");
    assert_eq!(
        model_count(&app),
        1,
        "the block member stays in the definition"
    );

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    let SemanticGeometry::Insert { transform, .. } = geometry_of(&app, 1) else {
        panic!("undo restores the reference");
    };
    assert_eq!(transform.matrix[2][3], 3.0);
}

#[test]
fn flatten_moves_a_text_position_z_to_zero() {
    let text = SemanticGeometry::Text {
        text: "HELLO".into(),
        position: p3(1.0, 2.0, 5.0),
        style: cad_domain::StyleId(0),
        height: 3.0,
        rotation: 0.0,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
        mirrored: false,
    };
    let (mut app, mut session) = build(
        vec![part(1, text, LayerId(0), SpaceId::Model, "AcDbText", 0)],
        Vec::new(),
    );
    select(&mut session, &[1]);
    run(&mut app, &mut session, ExtendedCommandId::Flatten).unwrap();
    assert_eq!(undo_depth(&app), 1);
    let SemanticGeometry::Text { position, .. } = geometry_of(&app, 1) else {
        panic!("the TEXT stays a TEXT");
    };
    assert_eq!(position, p3(1.0, 2.0, 0.0));
}

#[test]
fn flatten_refuses_non_flat_block_members_and_tilted_references_without_writing() {
    // The member LINE itself is away from z = 0: moving only the insertion z
    // would leave a non-flat reference, and the block definition is never edited.
    let (mut app, mut session) = build(
        vec![
            part(
                100,
                line(p3(0.0, 0.0, 4.0), p3(1.0, 0.0, 4.0)),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbLine",
                0,
            ),
            part(
                1,
                insert(1, Transform3::translation(p3(5.0, 5.0, 3.0))),
                LayerId(0),
                SpaceId::Model,
                "AcDbBlockReference",
                1,
            ),
        ],
        vec![BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        }],
    );
    let before = revision(&app);
    select(&mut session, &[1]);
    let error = run(&mut app, &mut session, ExtendedCommandId::Flatten).unwrap_err();
    assert!(error.to_string().contains("flatten.unsupported"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    let SemanticGeometry::Insert { transform, .. } = geometry_of(&app, 1) else {
        panic!("the reference is untouched");
    };
    assert_eq!(transform.matrix[2][3], 3.0);

    // A reference rotated out of the world XY plane has no z-only flat form.
    let (mut app, mut session) = block_app(Transform3::from_basis(
        p3(1.0, 0.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(5.0, 5.0, 3.0),
    ));
    let before = revision(&app);
    select(&mut session, &[1]);
    let error = run(&mut app, &mut session, ExtendedCommandId::Flatten).unwrap_err();
    assert!(error.to_string().contains("flatten.unsupported"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

fn change_properties(properties: &[(&str, &str)]) -> CommandPayload {
    CommandPayload::SelectionTool(Box::new(SelectionToolPayload::ChangeProperties(
        crate::ChangePropertiesRequest {
            properties: properties
                .iter()
                .map(|(key, value)| crate::EntityPropertyWrite {
                    key: (*key).to_string(),
                    value: (*value).to_string(),
                })
                .collect(),
        },
    )))
}

/// One LINE on `WALLS` (id 1) with an explicit red colour and 0.5 mm lineweight
/// in the requested space.
fn property_app(space: SpaceId) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    // A paper-space row must reference a real layout record.
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_entity(part(
        1,
        line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
        LayerId(3),
        space,
        "AcDbLine",
        0,
    ))
    .unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        EntityRenderAttributes {
            clip: None,
            color: EntityColor::Explicit([255, 0, 0]),
            lineweight: EntityLineWeight::Explicit(0.5),
            ..EntityRenderAttributes::default()
        },
    )
    .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
}

#[test]
fn chprop_writes_every_typed_property_in_one_transaction_and_undoes_once() {
    let (mut app, mut session) = property_app(SpaceId::Model);
    let before = revision(&app);
    select(&mut session, &[1]);

    let outcome = run_payload(
        &mut app,
        &mut session,
        ExtendedCommandId::ChProp,
        change_properties(&[
            ("layer", "0"),
            ("color", "aci:3"),
            ("lineweight", "0.35"),
            ("linetype", "ByBlock"),
            ("transparency", "40"),
        ]),
    )
    .unwrap();
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert_eq!(outcome.diagnostics[0].code, "chprop.applied");
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision bump");
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");

    let entity = drawing_of(&app).entity(EntityId(1)).unwrap();
    assert_eq!(entity.layer, LayerId(0));
    let attributes = drawing_of(&app).entity_render_attributes(EntityId(1));
    assert_eq!(
        attributes.color,
        EntityColor::Explicit(
            crate::property_value::rgb_for_aci(3).expect("ACI 3 is a palette entry")
        )
    );
    assert_eq!(attributes.lineweight, EntityLineWeight::Explicit(0.35));
    assert_eq!(
        attributes.linetype,
        cad_db::EntityLineType::ByBlock,
        "linetype follows the same grammar as SETENTITYPROPERTY"
    );
    assert_eq!(
        crate::property_value::transparency_value(attributes.transparency),
        "40"
    );

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    let restored = drawing_of(&app).entity(EntityId(1)).unwrap();
    assert_eq!(restored.layer, LayerId(3));
    let attributes = drawing_of(&app).entity_render_attributes(EntityId(1));
    assert_eq!(attributes.color, EntityColor::Explicit([255, 0, 0]));
    assert_eq!(attributes.lineweight, EntityLineWeight::Explicit(0.5));
    assert_eq!(attributes.linetype, cad_db::EntityLineType::ByLayer);
    assert_eq!(model_count(&app), 1);
}

#[test]
fn chprop_and_change_accept_paper_space_rows_and_the_last_value_of_a_key_wins() {
    for ext in [ExtendedCommandId::ChProp, ExtendedCommandId::Change] {
        let (mut app, mut session) = property_app(SpaceId::Paper(LayoutId(1)));
        select(&mut session, &[1]);
        run_payload(
            &mut app,
            &mut session,
            ext,
            change_properties(&[("color", "aci:5"), ("layer", "0"), ("color", "aci:4")]),
        )
        .unwrap();
        assert_eq!(undo_depth(&app), 1, "{ext:?} is one undo step");
        let entity = drawing_of(&app).entity(EntityId(1)).unwrap();
        assert_eq!(entity.layer, LayerId(0));
        assert_eq!(
            drawing_of(&app).entity_render_attributes(EntityId(1)).color,
            EntityColor::Explicit(
                crate::property_value::rgb_for_aci(4).expect("ACI 4 is a palette entry")
            ),
            "the last write of a repeated key wins"
        );
    }
}

#[test]
fn chprop_refuses_unsupported_property_names_and_values_without_writing() {
    let cases: Vec<(CommandPayload, &str)> = vec![
        (
            change_properties(&[("thickness", "2")]),
            "chprop.unsupported_property",
        ),
        (
            change_properties(&[("text", "hello")]),
            "chprop.unsupported_property",
        ),
        (
            change_properties(&[("ltscale", "2")]),
            "chprop.unsupported_property",
        ),
        (
            change_properties(&[("layer", "0"), ("color", "not-a-color")]),
            "chprop.invalid_value",
        ),
        (
            change_properties(&[("layer", "NO_SUCH_LAYER")]),
            "chprop.unknown_layer",
        ),
        (change_properties(&[]), "chprop.no_properties"),
        (CommandPayload::None, "chprop.no_properties"),
        (
            CommandPayload::Points(vec![p3(0.0, 0.0, 0.0)]),
            "chprop.payload",
        ),
    ];
    for (payload, expected) in cases {
        let (mut app, mut session) = property_app(SpaceId::Model);
        let before = revision(&app);
        select(&mut session, &[1]);
        let error = run_payload(&mut app, &mut session, ExtendedCommandId::ChProp, payload)
            .expect_err("the refusal must not write");
        assert!(error.to_string().contains(expected), "{expected}: {error}");
        assert_eq!(revision(&app), before, "{expected} bumps no revision");
        assert_eq!(undo_depth(&app), 0, "{expected} records no history");
        let entity = drawing_of(&app).entity(EntityId(1)).unwrap();
        assert_eq!(entity.layer, LayerId(3), "{expected} leaves the layer");
        let attributes = drawing_of(&app).entity_render_attributes(EntityId(1));
        assert_eq!(attributes.color, EntityColor::Explicit([255, 0, 0]));
        assert_eq!(attributes.lineweight, EntityLineWeight::Explicit(0.5));
    }

    // CHANGE shares the same grammar and reports under its own prefix.
    let (mut app, mut session) = property_app(SpaceId::Model);
    let before = revision(&app);
    select(&mut session, &[1]);
    let error = run_payload(
        &mut app,
        &mut session,
        ExtendedCommandId::Change,
        change_properties(&[("color", "not-a-color")]),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("change.invalid_value"),
        "{error}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn chprop_reports_unchanged_without_a_transaction_when_every_value_already_holds() {
    let (mut app, mut session) = property_app(SpaceId::Model);
    let before = revision(&app);
    select(&mut session, &[1]);
    let outcome = run_payload(
        &mut app,
        &mut session,
        ExtendedCommandId::ChProp,
        change_properties(&[("layer", "WALLS"), ("color", "aci:1")]),
    )
    .unwrap();
    assert_eq!(outcome.diagnostics[0].code, "chprop.unchanged");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}
