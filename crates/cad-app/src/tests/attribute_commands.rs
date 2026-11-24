//! Block-attribute command contracts (ADR-0055 M3).
//!
//! Every command here is exercised through the ordinary [`Application::execute`]
//! funnel. The tests pin the three promises the command layer makes:
//!
//! * one successful write = one transaction / one revision / one undo step;
//! * a refusal (duplicate tag, unknown tag, constant value, locked position, bad
//!   selection) writes nothing and records no history;
//! * a definition change that must reach references writes **every** referencing
//!   `INSERT` in the same transaction (`ATTSYNC`, `ATTREDEF`, `BATTORDER`).
//!
//! `BURST` converts values (constant values included) to `TEXT`; `ATTDISP` is a
//! display setting and is covered in [`crate::tests::extended_attrs5`].

use super::*;
use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Style};
use cad_domain::{
    AttributeDefinition, AttributeFlags, AttributeText, InstancePath, SelectionRef, StyleId,
    TextAlignH, TextAlignV, TextLineSpacing,
};

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

fn revision_of(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn drawing(app: &Application) -> &DrawingDatabase {
    app.workspace.documents[&DocumentId(1)].drawing.as_ref()
}

fn attribute_text(x: f64, y: f64, height: f64) -> AttributeText {
    AttributeText {
        position: point(x, y),
        height,
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

fn definition_geometry(
    tag: &str,
    default_value: &str,
    flags: AttributeFlags,
    order: u32,
) -> SemanticGeometry {
    SemanticGeometry::AttributeDefinition(Box::new(AttributeDefinition {
        tag: tag.into(),
        prompt: format!("{tag}?"),
        default_value: default_value.into(),
        flags,
        order,
        text: attribute_text(1.0, 2.0, 2.5),
        source_handle: None,
    }))
}

fn attribute_value(tag: &str, value: &str, flags: AttributeFlags) -> AttributeValue {
    AttributeValue {
        tag: tag.into(),
        value: value.into(),
        flags,
        text: attribute_text(1.0, 2.0, 2.5),
        layer: LayerId(0),
        appearance: Default::default(),
        source_handle: None,
    }
}

fn entity(id: u128, space: SpaceId, layer: LayerId, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: match &geometry {
                SemanticGeometry::AttributeDefinition(_) => "AcDbAttributeDefinition",
                SemanticGeometry::Insert { .. } => "AcDbBlockReference",
                _ => "AcDbLine",
            }
            .into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space,
        geometry,
        draw_order: 0,
    }
}

fn insert_geometry(block: BlockId, values: Vec<AttributeValue>) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block,
        transform: Transform3::identity(),
        attributes: values,
    }
}

/// A drawing with one block `TITLE`, the given definitions, and one INSERT per
/// entry in `inserts` (the attribute value list of each reference).
fn application_with_attributes(
    definitions: &[(&str, &str, AttributeFlags, u32)],
    inserts: &[Vec<AttributeValue>],
) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let block = BlockId(1);
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    let mut members = Vec::new();
    let mut next = 10u128;
    let line = entity(
        next,
        SpaceId::Block(block),
        LayerId(0),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    );
    builder.insert_entity(line).unwrap();
    members.push(EntityId(next));
    next += 1;
    for (tag, default_value, flags, order) in definitions {
        let member = entity(
            next,
            SpaceId::Block(block),
            LayerId(0),
            definition_geometry(tag, default_value, *flags, *order),
        );
        builder.insert_entity(member).unwrap();
        members.push(EntityId(next));
        next += 1;
    }
    builder
        .insert_block(BlockDefinition {
            id: block,
            entities: members,
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(block, "TITLE", point(0.0, 0.0))
        .unwrap();
    for values in inserts {
        let insert = entity(
            next,
            SpaceId::Model,
            LayerId(0),
            insert_geometry(block, values.clone()),
        );
        builder.insert_entity(insert).unwrap();
        next += 1;
    }
    let drawing = builder.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn definition_payload(tag: &str, default_value: &str) -> AttributeDefinitionCommand {
    AttributeDefinitionCommand {
        block: "TITLE".into(),
        tag: tag.into(),
        prompt: format!("{tag}?"),
        default_value: default_value.into(),
        flags: AttributeFlags::default(),
        order: None,
        position: point(3.0, 4.0),
        height: 2.5,
        rotation: 0.0,
        width_factor: 1.0,
        wrap_width: None,
        oblique: 0.0,
    }
}

fn edit_payload(tag: &str, value: &str, scope: AttributeEditScope) -> AttributeEditCommand {
    AttributeEditCommand {
        block: "TITLE".into(),
        assignments: vec![AttributeAssignment {
            tag: tag.into(),
            value: value.into(),
        }],
        scope,
        position: None,
        height: None,
        rotation: None,
    }
}

fn tags_and_values(drawing: &DrawingDatabase, id: EntityId) -> Vec<(String, String)> {
    let SemanticGeometry::Insert { attributes, .. } = &drawing.entity(id).unwrap().geometry else {
        panic!("not an insert");
    };
    attributes
        .iter()
        .map(|value| (value.tag.clone(), value.value.clone()))
        .collect()
}

#[test]
fn attdef_adds_one_member_in_one_transaction_and_carries_the_next_insert() {
    let (mut app, mut session) = application_with_attributes(&[], &[Vec::new()]);
    let before = revision_of(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AttDef),
        CommandPayload::Attribute(AttributeCommand::Definition(definition_payload(
            "ROOM", "A-101",
        ))),
    )
    .expect("ATTDEF writes");
    assert_eq!(revision_of(&app), Revision(before.0 + 1), "one revision");
    assert!(outcome.changes.is_some());
    assert!(
        outcome
            .changes
            .as_ref()
            .unwrap()
            .scope
            .tables
            .contains(&cad_db::TableKind::Blocks),
        "a definition edit dirties the block table (ADR-0055 D3)"
    );
    assert!(app.can_undo(&DocumentId(1)), "one undo step");
    let block = drawing(&app).block(BlockId(1)).unwrap();
    assert_eq!(block.entities.len(), 2, "the definition becomes a member");
    let definitions = drawing(&app)
        .block_entities(BlockId(1))
        .into_iter()
        .filter(|member| matches!(member.geometry, SemanticGeometry::AttributeDefinition(_)))
        .count();
    assert_eq!(definitions, 1);
    // The already-open reference is not rewritten by ATTDEF; that is ATTSYNC's
    // job, and the next insertion receives the default value.
    let reference = drawing(&app).model_space()[0].id;
    assert!(tags_and_values(drawing(&app), reference).is_empty());
    let defaults = drawing(&app).default_attribute_values(BlockId(1));
    assert_eq!(defaults[0].tag, "ROOM");
    assert_eq!(defaults[0].value, "A-101");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert_eq!(drawing(&app).block(BlockId(1)).unwrap().entities.len(), 1);
    assert!(drawing(&app)
        .block_entities(BlockId(1))
        .iter()
        .all(|member| { !matches!(member.geometry, SemanticGeometry::AttributeDefinition(_)) }));
}

#[test]
fn attdef_duplicate_tag_is_refused_without_a_write() {
    let (mut app, mut session) =
        application_with_attributes(&[("ROOM", "A", AttributeFlags::default(), 0)], &[]);
    let before = revision_of(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AttDef),
        CommandPayload::Attribute(AttributeCommand::Definition(definition_payload(
            "room", "B",
        ))),
    )
    .expect_err("duplicate tag");
    assert!(
        error.to_string().contains("attribute.duplicate_tag"),
        "{error}"
    );
    assert_eq!(revision_of(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn attedit_selection_edits_only_the_selected_reference() {
    let values = attribute_value("ROOM", "A", AttributeFlags::default());
    let (mut app, mut session) = application_with_attributes(
        &[("ROOM", "A", AttributeFlags::default(), 0)],
        &[vec![values.clone()], vec![values]],
    );
    let (first, second) = {
        let model = drawing(&app).model_space();
        (model[0].id, model[1].id)
    };
    session.selection = SelectionSet::from_refs([ref_of(first.0)]);
    let before = revision_of(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AttEdit),
        CommandPayload::Attribute(AttributeCommand::Edit(edit_payload(
            "ROOM",
            "B",
            AttributeEditScope::Selection,
        ))),
    )
    .expect("ATTEDIT writes");
    assert_eq!(revision_of(&app), Revision(before.0 + 1));
    assert_eq!(
        tags_and_values(drawing(&app), first),
        vec![("ROOM".into(), "B".into())]
    );
    assert_eq!(
        tags_and_values(drawing(&app), second),
        vec![("ROOM".into(), "A".into())],
        "the unselected reference is untouched"
    );
}

#[test]
fn gatte_edits_every_reference_and_keeps_one_undo_step() {
    let values = attribute_value("ROOM", "A", AttributeFlags::default());
    let (mut app, mut session) = application_with_attributes(
        &[("ROOM", "A", AttributeFlags::default(), 0)],
        &[vec![values.clone()], vec![values]],
    );
    let before = revision_of(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Gatte),
        CommandPayload::Attribute(AttributeCommand::Edit(edit_payload(
            "ROOM",
            "B",
            AttributeEditScope::AllInstances,
        ))),
    )
    .expect("GATTE writes");
    assert_eq!(revision_of(&app), Revision(before.0 + 1));
    assert!(
        !outcome
            .changes
            .as_ref()
            .unwrap()
            .scope
            .tables
            .contains(&cad_db::TableKind::Blocks),
        "a value edit is entity-scoped, not a definition edit"
    );
    for insert in drawing(&app).model_space() {
        assert_eq!(
            tags_and_values(drawing(&app), insert.id),
            vec![("ROOM".into(), "B".into())]
        );
    }
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    for insert in drawing(&app).model_space() {
        assert_eq!(
            tags_and_values(drawing(&app), insert.id),
            vec![("ROOM".into(), "A".into())]
        );
    }
}

#[test]
fn attedit_refuses_unknown_constant_and_locked_position_without_writing() {
    let constant = AttributeFlags {
        constant: true,
        ..AttributeFlags::default()
    };
    let locked = AttributeFlags {
        locked_position: true,
        ..AttributeFlags::default()
    };
    let (mut app, mut session) = application_with_attributes(
        &[
            ("ROOM", "A", AttributeFlags::default(), 0),
            ("SERIAL", "S", constant, 1),
            ("LOCKED", "L", locked, 2),
        ],
        &[vec![
            attribute_value("ROOM", "A", AttributeFlags::default()),
            attribute_value("SERIAL", "S", constant),
            attribute_value("LOCKED", "L", locked),
        ]],
    );
    let insert = drawing(&app).model_space()[0].id;
    session.selection = SelectionSet::from_refs([ref_of(insert.0)]);
    let before = revision_of(&app);

    let unknown = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AttEdit),
        CommandPayload::Attribute(AttributeCommand::Edit(edit_payload(
            "NOPE",
            "X",
            AttributeEditScope::Selection,
        ))),
    )
    .expect_err("unknown tag");
    assert!(
        unknown.to_string().contains("attribute.unknown_tag"),
        "{unknown}"
    );

    let constant_error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AttEdit),
        CommandPayload::Attribute(AttributeCommand::Edit(edit_payload(
            "SERIAL",
            "X",
            AttributeEditScope::Selection,
        ))),
    )
    .expect_err("constant");
    assert!(
        constant_error.to_string().contains("attribute.constant"),
        "{constant_error}"
    );

    let mut placement = edit_payload("LOCKED", "X", AttributeEditScope::Selection);
    placement.position = Some(point(9.0, 9.0));
    let locked_error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::EAttEdit),
        CommandPayload::Attribute(AttributeCommand::Edit(placement)),
    )
    .expect_err("locked position");
    assert!(
        locked_error
            .to_string()
            .contains("attribute.locked_position"),
        "{locked_error}"
    );
    assert_eq!(revision_of(&app), before, "every refusal writes nothing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn attsync_materializes_missing_values_on_every_reference() {
    let (mut app, mut session) = application_with_attributes(
        &[("ROOM", "A", AttributeFlags::default(), 0)],
        &[Vec::new(), Vec::new()],
    );
    let before = revision_of(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AttSync),
        CommandPayload::Attribute(AttributeCommand::Sync(AttributeSyncCommand {
            block: "TITLE".into(),
        })),
    )
    .expect("ATTSYNC writes");
    assert_eq!(revision_of(&app), Revision(before.0 + 1));
    for insert in drawing(&app).model_space() {
        assert_eq!(
            tags_and_values(drawing(&app), insert.id),
            vec![("ROOM".into(), "A".into())]
        );
    }
}

#[test]
fn attredef_propagates_the_new_definition_to_every_reference() {
    let (mut app, mut session) = application_with_attributes(
        &[("ROOM", "A", AttributeFlags::default(), 0)],
        &[vec![attribute_value(
            "ROOM",
            "KEEP",
            AttributeFlags::default(),
        )]],
    );
    let before = revision_of(&app);
    let mut payload = definition_payload("FLOOR", "1");
    payload.order = Some(1);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AttRedef),
        CommandPayload::Attribute(AttributeCommand::Definition(payload)),
    )
    .expect("ATTREDEF writes");
    assert_eq!(revision_of(&app), Revision(before.0 + 1));
    assert!(outcome.changes.is_some());
    let insert = drawing(&app).model_space()[0].id;
    assert_eq!(
        tags_and_values(drawing(&app), insert),
        vec![("ROOM".into(), "KEEP".into()), ("FLOOR".into(), "1".into())],
        "matching values survive and the new tag is materialized"
    );
}

#[test]
fn battorder_rewrites_every_definition_and_reference_order() {
    let (mut app, mut session) = application_with_attributes(
        &[
            ("A", "1", AttributeFlags::default(), 0),
            ("B", "2", AttributeFlags::default(), 1),
        ],
        &[vec![
            attribute_value("A", "1", AttributeFlags::default()),
            attribute_value("B", "2", AttributeFlags::default()),
        ]],
    );
    let before = revision_of(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::BattOrder),
        CommandPayload::Attribute(AttributeCommand::Order(AttributeOrderCommand {
            block: "TITLE".into(),
            tags: vec!["B".into(), "A".into()],
        })),
    )
    .expect("BATTORDER writes");
    assert_eq!(revision_of(&app), Revision(before.0 + 1));
    assert!(
        outcome
            .changes
            .as_ref()
            .unwrap()
            .scope
            .tables
            .contains(&cad_db::TableKind::Blocks),
        "reordering definitions dirties the block table (ADR-0055 D3)"
    );
    let insert = drawing(&app).model_space()[0].id;
    assert_eq!(
        tags_and_values(drawing(&app), insert),
        vec![("B".into(), "2".into()), ("A".into(), "1".into())]
    );
    let mut order: Vec<(u32, String)> = drawing(&app)
        .block_entities(BlockId(1))
        .into_iter()
        .filter_map(|member| match &member.geometry {
            SemanticGeometry::AttributeDefinition(definition) => {
                Some((definition.order, definition.tag.clone()))
            }
            _ => None,
        })
        .collect();
    order.sort();
    assert_eq!(order, vec![(0, "B".into()), (1, "A".into())]);
}

#[test]
fn battorder_refuses_an_incomplete_order_without_writing() {
    let (mut app, mut session) =
        application_with_attributes(&[("A", "1", AttributeFlags::default(), 0)], &[Vec::new()]);
    let before = revision_of(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::BattOrder),
        CommandPayload::Attribute(AttributeCommand::Order(AttributeOrderCommand {
            block: "TITLE".into(),
            tags: vec!["A".into(), "B".into()],
        })),
    )
    .expect_err("order mismatch");
    assert!(
        error.to_string().contains("attribute.order_mismatch"),
        "{error}"
    );
    assert_eq!(revision_of(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn burst_converts_values_including_constants_to_text_and_deletes_the_source() {
    let constant = AttributeFlags {
        constant: true,
        ..AttributeFlags::default()
    };
    let (mut app, mut session) = application_with_attributes(
        &[
            ("ROOM", "A", AttributeFlags::default(), 0),
            ("SERIAL", "42", constant, 1),
        ],
        &[vec![
            attribute_value("ROOM", "A-101", AttributeFlags::default()),
            attribute_value("SERIAL", "42", constant),
        ]],
    );
    let source = drawing(&app).model_space()[0].id;
    session.selection = SelectionSet::from_refs([ref_of(source.0)]);
    let before = revision_of(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Burst),
        CommandPayload::None,
    )
    .expect("BURST writes");
    assert_eq!(revision_of(&app), Revision(before.0 + 1));
    assert!(drawing(&app).entity(source).is_none(), "source deleted");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "burst.exploded"));
    let texts: Vec<String> = drawing(&app)
        .model_space()
        .into_iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(texts.contains(&"A-101".to_string()), "{texts:?}");
    assert!(
        texts.contains(&"42".to_string()),
        "constant value converted: {texts:?}"
    );
}

#[test]
fn attdef_bare_is_the_read_only_definition_report() {
    let (mut app, mut session) =
        application_with_attributes(&[("ROOM", "A", AttributeFlags::default(), 0)], &[]);
    let before = revision_of(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::AttDef),
        CommandPayload::None,
    )
    .expect("the report is admitted");
    assert!(outcome.changes.is_none());
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "attdef.entry"
                && diagnostic.message.contains("ROOM"))
    );
    assert_eq!(revision_of(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}
