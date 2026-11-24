//! `XPLODE` property/attribute-control contracts.
//!
//! Split out of `extended_edit4` so both files stay inside the 1000-line code
//! gate. Each test pins the actual database effect: every child receives the
//! typed properties in **one** transaction / one undo step, the block-definition
//! `ATTDEF` entities are emitted when values are dropped, and every refusal
//! writes nothing.

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor, EntityLineType,
    EntityLineWeight, EntityRenderAttributes, EntityTransparency, Layer, LinetypePattern, Style,
};
use cad_domain::{
    AttributeDefinition, AttributeFlags, AttributeText, AttributeValue, StyleId, TextAlignH,
    TextAlignV, TextLineSpacing,
};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(start: Point3, end: Point3) -> SemanticGeometry {
    SemanticGeometry::Line { start, end }
}

fn insert(block: u128, transform: Transform3, attributes: Vec<AttributeValue>) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: BlockId(block),
        transform,
        attributes,
    }
}

fn attribute_text() -> AttributeText {
    AttributeText {
        position: p3(1.0, 2.0, 0.0),
        height: 2.5,
        rotation: 0.0,
        style: StyleId(0),
        width_factor: 1.0,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        line_spacing: TextLineSpacing::default(),
        font: None,
        oblique: 0.0,
    }
}

fn definition(tag: &str, default_value: &str) -> SemanticGeometry {
    SemanticGeometry::AttributeDefinition(Box::new(AttributeDefinition {
        tag: tag.into(),
        prompt: format!("{tag}?"),
        default_value: default_value.into(),
        flags: AttributeFlags::default(),
        order: 0,
        text: attribute_text(),
        source_handle: None,
    }))
}

fn value(tag: &str, value: &str) -> AttributeValue {
    AttributeValue {
        tag: tag.into(),
        value: value.into(),
        flags: AttributeFlags::default(),
        text: attribute_text(),
        layer: LayerId(0),
        appearance: Default::default(),
        source_handle: None,
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

fn block(id: u128, entities: Vec<u128>) -> BlockDefinition {
    BlockDefinition {
        id: BlockId(id),
        entities: entities.into_iter().map(EntityId).collect(),
        dynamic_visibility: None,
    }
}

fn build(
    entities: Vec<DbEntity>,
    blocks: Vec<BlockDefinition>,
    display: Vec<(u128, EntityRenderAttributes)>,
) -> (Application, SessionState) {
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
    b.insert_style(Style {
        id: StyleId(0),
        name: "Standard".into(),
        resource_keys: Vec::new(),
    })
    .unwrap();
    for block in blocks {
        b.insert_block(block).unwrap();
    }
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    for (id, attributes) in display {
        b.set_entity_render_attributes(EntityId(id), attributes)
            .unwrap();
    }
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
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

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn xplode(
    app: &mut Application,
    session: &mut SessionState,
    settings: Option<XPlodeSettings>,
) -> CadResult<CommandOutcome> {
    let payload = match settings {
        Some(settings) => {
            CommandPayload::SelectionTool(Box::new(SelectionToolPayload::XPlode(settings)))
        }
        None => CommandPayload::None,
    };
    app.execute(
        session,
        command(CommandId::Extended(ExtendedCommandId::XPlode), payload),
    )
}

fn property(key: &str, value: &str) -> EntityPropertyWrite {
    EntityPropertyWrite {
        key: key.into(),
        value: value.into(),
    }
}

/// A block with two LINE members on layer `0`.
fn two_line_block() -> (Application, SessionState) {
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
                101,
                line(p3(0.0, 1.0, 0.0), p3(1.0, 1.0, 0.0)),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbLine",
                1,
            ),
            part(
                1,
                insert(1, Transform3::identity(), Vec::new()),
                LayerId(3),
                SpaceId::Model,
                "AcDbBlockReference",
                2,
            ),
        ],
        vec![block(1, vec![100, 101])],
        Vec::new(),
    )
}

#[test]
fn xplode_applies_layer_and_color_to_every_child_in_one_undo() {
    let (mut app, mut session) = two_line_block();
    let before = revision(&app);
    select(&mut session, &[1]);
    let outcome = xplode(
        &mut app,
        &mut session,
        Some(XPlodeSettings {
            inherit_from_parent: false,
            properties: vec![property("layer", "WALLS"), property("color", "aci:1")],
        }),
    )
    .expect("XPLODE with a typed property list is admitted");

    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");
    assert!(drawing_of(&app).entity(EntityId(1)).is_none());
    assert_eq!(outcome.objects.len(), 2);
    let children = drawing_of(&app).model_space();
    assert_eq!(children.len(), 2, "both block members were produced");
    for child in children {
        assert_eq!(
            child.layer,
            LayerId(3),
            "the whole selection's layer was set"
        );
        assert_eq!(
            drawing_of(&app).entity_render_attributes(child.id).color,
            EntityColor::Explicit([255, 0, 0]),
            "aci:1 is the canonical red"
        );
    }
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "xplode.properties"));
}

#[test]
fn xplode_inherit_resolves_layer_zero_and_byblock_against_the_reference() {
    let member = EntityRenderAttributes {
        color: EntityColor::ByBlock,
        lineweight: EntityLineWeight::ByBlock,
        linetype: EntityLineType::ByBlock,
        transparency: EntityTransparency::ByBlock,
        ..Default::default()
    };
    let parent = EntityRenderAttributes {
        color: EntityColor::Explicit([10, 20, 30]),
        lineweight: EntityLineWeight::Explicit(0.5),
        linetype: EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern {
                elements: vec![2.0, -1.0],
                cycle: 3.0,
            },
            scale: 2.0,
        },
        linetype_scale: 2.0,
        transparency: EntityTransparency::Explicit(0.75),
        ..Default::default()
    };
    let (mut app, mut session) = build(
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
                insert(1, Transform3::identity(), Vec::new()),
                LayerId(3),
                SpaceId::Model,
                "AcDbBlockReference",
                1,
            ),
        ],
        vec![block(1, vec![100])],
        vec![(100, member), (1, parent)],
    );
    select(&mut session, &[1]);
    let outcome = xplode(
        &mut app,
        &mut session,
        Some(XPlodeSettings {
            inherit_from_parent: true,
            properties: Vec::new(),
        }),
    )
    .expect("XPLODE INHERIT is admitted");

    assert_eq!(undo_depth(&app), 1);
    let child = drawing_of(&app).model_space()[0];
    assert_eq!(
        child.layer,
        LayerId(3),
        "layer-0 member uses the reference layer"
    );
    let attributes = drawing_of(&app).entity_render_attributes(child.id);
    assert_eq!(attributes.color, EntityColor::Explicit([10, 20, 30]));
    assert_eq!(attributes.lineweight, EntityLineWeight::Explicit(0.5));
    assert_eq!(
        attributes.linetype,
        EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern {
                elements: vec![2.0, -1.0],
                cycle: 3.0,
            },
            scale: 2.0,
        }
    );
    assert_eq!(attributes.linetype_scale, 2.0);
    assert_eq!(attributes.transparency, EntityTransparency::Explicit(0.75));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "xplode.inherit"));
}

#[test]
fn xplode_drops_values_and_emits_the_block_definitions() {
    let (mut app, mut session) = build(
        vec![
            part(
                100,
                definition("TAG1", "DEFAULT"),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbAttributeDefinition",
                0,
            ),
            part(
                101,
                line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbLine",
                1,
            ),
            part(
                1,
                insert(1, Transform3::identity(), vec![value("TAG1", "VALUE")]),
                LayerId(0),
                SpaceId::Model,
                "AcDbBlockReference",
                2,
            ),
        ],
        vec![block(1, vec![100, 101])],
        Vec::new(),
    );
    select(&mut session, &[1]);
    let outcome = xplode(
        &mut app,
        &mut session,
        Some(XPlodeSettings {
            inherit_from_parent: false,
            properties: vec![property("layer", "WALLS"), property("color", "aci:1")],
        }),
    )
    .expect("XPLODE with controls is admitted");

    assert_eq!(undo_depth(&app), 1);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "xplode.attributes_reverted"));
    let entities = drawing_of(&app).model_space();
    assert_eq!(entities.len(), 2, "the LINE and the ATTDEF were produced");
    let definition_entity = entities
        .iter()
        .find(|entity| matches!(&entity.geometry, SemanticGeometry::AttributeDefinition(_)))
        .expect("the block definition's ATTDEF is emitted");
    let SemanticGeometry::AttributeDefinition(definition) = &definition_entity.geometry else {
        unreachable!("the find above selected an ATTDEF");
    };
    assert_eq!(definition.default_value, "DEFAULT");
    assert_eq!(
        definition_entity.layer,
        LayerId(3),
        "the property controls reach the emitted ATTDEF too"
    );
    assert_eq!(
        drawing_of(&app)
            .entity_render_attributes(definition_entity.id)
            .color,
        EntityColor::Explicit([255, 0, 0])
    );
    assert!(
        !entities.iter().any(|entity| matches!(
            &entity.geometry,
            SemanticGeometry::Text { text, .. } if text == "VALUE"
        )),
        "XPLODE drops the reference's value; BURST is the converting command"
    );
}

#[test]
fn xplode_refuses_unsupported_or_conflicting_controls_without_writing() {
    let (mut app, mut session) = two_line_block();
    let before = revision(&app);
    select(&mut session, &[1]);
    let error = xplode(
        &mut app,
        &mut session,
        Some(XPlodeSettings {
            inherit_from_parent: false,
            properties: vec![property("transparency", "50")],
        }),
    )
    .expect_err("XPLODE has no transparency option");
    assert!(
        error.to_string().contains("xplode.unsupported_property"),
        "{error}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert!(drawing_of(&app).entity(EntityId(1)).is_some());

    let error = xplode(
        &mut app,
        &mut session,
        Some(XPlodeSettings {
            inherit_from_parent: true,
            properties: vec![property("layer", "WALLS")],
        }),
    )
    .expect_err("the two options are mutually exclusive");
    assert!(
        error.to_string().contains("xplode.conflicting_options"),
        "{error}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    let error = xplode(
        &mut app,
        &mut session,
        Some(XPlodeSettings {
            inherit_from_parent: false,
            properties: vec![property("color", "not-a-color")],
        }),
    )
    .expect_err("the colour grammar refuses a bad value");
    assert!(
        error.to_string().contains("xplode.invalid_value"),
        "{error}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn xplode_refuses_a_non_insert_selection_without_writing() {
    let (mut app, mut session) = build(
        vec![part(
            1,
            line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
            LayerId(0),
            SpaceId::Model,
            "AcDbLine",
            0,
        )],
        Vec::new(),
        Vec::new(),
    );
    let before = revision(&app);
    select(&mut session, &[1]);
    let error =
        xplode(&mut app, &mut session, None).expect_err("XPLODE's domain is block references");
    assert!(error.to_string().contains("xplode.unsupported"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

/// One block reference whose placement and layer visibility the test controls.
fn positioned_block_app(
    transform: Transform3,
    insert_layer: LayerId,
    layer_visible: bool,
) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layer(Layer {
        id: LayerId(3),
        name: "WALLS".into(),
        visible: layer_visible,
    })
    .unwrap();
    b.insert_block(block(1, vec![100])).unwrap();
    b.insert_entity(part(
        100,
        line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
        LayerId(0),
        SpaceId::Block(BlockId(1)),
        "AcDbLine",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, transform, Vec::new()),
        insert_layer,
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
}

#[test]
fn xplode_refuses_non_uniform_and_frozen_layer_sources_without_writing() {
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = 2.0;
    matrix[1][1] = 3.0;
    let (mut app, mut session) = positioned_block_app(Transform3 { matrix }, LayerId(0), true);
    let before = revision(&app);
    select(&mut session, &[1]);
    let error =
        xplode(&mut app, &mut session, None).expect_err("XPLODE refuses unequal X/Y scales");
    assert!(error.to_string().contains("xplode.non_uniform"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    let (mut app, mut session) = positioned_block_app(Transform3::identity(), LayerId(3), false);
    let before = revision(&app);
    select(&mut session, &[1]);
    let error =
        xplode(&mut app, &mut session, None).expect_err("XPLODE refuses a frozen-layer object");
    assert!(error.to_string().contains("xplode.frozen_layer"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}
