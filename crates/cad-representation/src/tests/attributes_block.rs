//! Block-attribute rendering contracts (ADR-0055 M2).
//!
//! An `INSERT` draws one text fragment per attribute value in definition order,
//! under the reference transform; an `ATTDEF` member is a definition and is not
//! drawn a second time. `ATTDISP` selects which values draw, and a `constant`
//! definition always draws.

use super::*;

fn text(x: f64, y: f64) -> AttributeText {
    AttributeText {
        position: p(x, y),
        height: 1.0,
        ..AttributeText::default()
    }
}

fn flags(invisible: bool, constant: bool) -> AttributeFlags {
    AttributeFlags {
        invisible,
        constant,
        ..AttributeFlags::default()
    }
}

fn definition(tag: &str, order: u32, definitions: AttributeFlags) -> SemanticGeometry {
    SemanticGeometry::AttributeDefinition(Box::new(AttributeDefinition {
        tag: tag.into(),
        prompt: format!("{tag}?"),
        default_value: format!("default-{tag}"),
        flags: definitions,
        order,
        text: text(0.0, 0.0),
        source_handle: None,
    }))
}

fn value(tag: &str, text_value: &str, definitions: AttributeFlags, y: f64) -> AttributeValue {
    AttributeValue {
        tag: tag.into(),
        value: text_value.into(),
        flags: definitions,
        text: text(0.5, y),
        layer: LayerId(0),
        appearance: AttributeAppearance::default(),
        source_handle: None,
    }
}

/// Block 7 owns three `ATTDEF` members; model space holds one reference at
/// `(10, 20)` carrying the three values in reverse order.
fn attribute_database() -> DrawingDatabase {
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(7),
        entities: vec![EntityId(71), EntityId(72), EntityId(73)],
        dynamic_visibility: None,
    })
    .unwrap();
    // Definition order 0, 1, 2.
    for (id, tag, order, definitions) in [
        (71u128, "VISIBLE", 0u32, flags(false, false)),
        (72, "HIDDEN", 1, flags(true, false)),
        (73, "CONST", 2, flags(true, true)),
    ] {
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbAttributeDefinition".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space: SpaceId::Block(BlockId(7)),
            geometry: definition(tag, order, definitions),
            draw_order: id as i64,
        })
        .unwrap();
    }
    let mut insert = insert_entity(1, SpaceId::Model, 7, 10.0);
    insert.geometry = SemanticGeometry::Insert {
        block: BlockId(7),
        transform: Transform3::translation(p(10.0, 20.0)),
        // Deliberately reverse of definition order: expansion must sort.
        attributes: vec![
            value("CONST", "const-value", flags(true, true), 3.0),
            value("HIDDEN", "hidden-value", flags(true, false), 2.0),
            value("VISIBLE", "visible-value", flags(false, false), 1.0),
        ],
    };
    b.insert_entity(insert).unwrap();
    b.finish().unwrap()
}

fn expanded_texts(mode: AttributeDisplay) -> Vec<(String, Point3)> {
    let db = attribute_database();
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(
            &db,
            db.entity(EntityId(1)).unwrap(),
            &context().with_attdisp(mode),
        )
        .unwrap();
    rep.fragments
        .into_iter()
        .filter_map(|fragment| match fragment.primitive {
            DisplayPrimitive::Text { text, origin, .. } => Some((text, origin)),
            _ => None,
        })
        .collect()
}

#[test]
fn attdisp_normal_draws_visible_and_constant_values_only() {
    let texts = expanded_texts(AttributeDisplay::Normal);
    let values: Vec<&str> = texts.iter().map(|(text, _)| text.as_str()).collect();
    assert_eq!(values, vec!["visible-value", "const-value"]);
    // The model positions are the block-local value positions under the INSERT
    // transform; the definition at (0,0) is not drawn.
    assert_eq!(texts[0].1, p(10.5, 21.0));
    assert_eq!(texts[1].1, p(10.5, 23.0));
}

#[test]
fn attdisp_on_draws_every_value_in_definition_order() {
    let texts = expanded_texts(AttributeDisplay::On);
    let values: Vec<&str> = texts.iter().map(|(text, _)| text.as_str()).collect();
    assert_eq!(
        values,
        vec!["visible-value", "hidden-value", "const-value"],
        "values are emitted in ATTDEF order, not ATTRIB order"
    );
}

#[test]
fn attdisp_off_hides_every_non_constant_value_but_keeps_constants() {
    let texts = expanded_texts(AttributeDisplay::Off);
    let values: Vec<&str> = texts.iter().map(|(text, _)| text.as_str()).collect();
    assert_eq!(
        values,
        vec!["const-value"],
        "a constant definition draws even when ATTDISP is Off"
    );
}

#[test]
fn attdef_members_are_not_drawn_as_an_extra_fragment_inside_a_reference() {
    let db = attribute_database();
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(
            &db,
            db.entity(EntityId(1)).unwrap(),
            &context().with_attdisp(AttributeDisplay::On),
        )
        .unwrap();
    // Exactly the three values; no default text box for the definitions.
    assert_eq!(rep.fragments.len(), 3);
    assert!(
        rep.diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "representation.empty"),
        "a definition member is skipped, not reported as an empty drawable: {:?}",
        rep.diagnostics
    );
}
