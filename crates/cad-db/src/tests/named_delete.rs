//! The named-object **delete** APIs: block, text style and linetype.
//!
//! Every contract pins one of `drawing::named_delete`'s rules: validate before
//! mutate (a refusal leaves the table and the revision byte-identical), one
//! revision bump per delete, the named in-use refusal (a block with an INSERT, a
//! style used by an entity or a dimension style, a linetype referenced by a layer
//! row or an entity name), and the two default rows (`Standard`, `Continuous`).
//! A block delete removes its member entities in the same revision.

use super::*;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn line() -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p(0.0, 0.0),
        end: p(1.0, 0.0),
    }
}

fn text(style: StyleId) -> SemanticGeometry {
    SemanticGeometry::Text {
        position: p(0.0, 0.0),
        height: 1.0,
        text: "hi".into(),
        style,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        line_spacing: TextLineSpacing::default(),
        width_factor: 1.0,
    }
}

fn base_builder() -> DrawingDatabaseBuilder {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b
}

// ---------------------------------------------------------------------------
// Block
// ---------------------------------------------------------------------------

#[test]
fn deleting_an_unused_block_removes_the_row_and_its_members_in_one_revision() {
    let mut b = base_builder();
    b.insert_block(BlockDefinition {
        id: BlockId(7),
        entities: vec![EntityId(5)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(raw_entity(5, SpaceId::Block(BlockId(7)), line()))
        .unwrap();
    let mut db = b.finish().unwrap();
    let before = db.revision();
    db.ensure_block_deletable(BlockId(7))
        .expect("nothing references block 7");

    let changes = db
        .delete_block(BlockId(7), TransactionId(1), "purge block")
        .expect("the delete is legal");

    assert!(db.block(BlockId(7)).is_none());
    assert!(db.entity(EntityId(5)).is_none(), "the member went with it");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, Revision(before.0 + 1));
    assert_eq!(db.revision(), changes.after);
    assert!(
        changes
            .changes
            .iter()
            .any(|change| matches!(change, crate::ObjectChange::Delete(object) if object.0 == 5)),
        "the member delete is announced: {:?}",
        changes.changes
    );
}

#[test]
fn a_block_with_an_insert_is_refused_by_name_and_changes_nothing() {
    let mut b = base_builder();
    b.insert_block(BlockDefinition {
        id: BlockId(7),
        entities: vec![EntityId(5)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(raw_entity(5, SpaceId::Block(BlockId(7)), line()))
        .unwrap();
    b.insert_entity(raw_entity(
        6,
        SpaceId::Model,
        SemanticGeometry::Insert {
            block: BlockId(7),
            transform: Transform3::translation(p(0.0, 0.0)),
        },
    ))
    .unwrap();
    let mut db = b.finish().unwrap();
    let before = db.revision();

    let error = db
        .delete_block(BlockId(7), TransactionId(1), "purge block")
        .expect_err("INSERT 6 references block 7");
    let text = error.to_string();
    assert!(text.contains("block.in_use"), "{text}");
    assert!(text.contains('7'), "the refusal names the block id: {text}");

    assert_eq!(db.revision(), before);
    assert!(db.block(BlockId(7)).is_some());
    assert!(db.entity(EntityId(5)).is_some());
    assert!(db.entity(EntityId(6)).is_some());
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

#[test]
fn deleting_an_unused_style_removes_the_row_in_one_revision() {
    let mut b = base_builder();
    b.insert_style(Style {
        id: StyleId(1),
        name: "Standard".into(),
        resource_keys: Vec::new(),
    })
    .unwrap();
    b.insert_style(Style {
        id: StyleId(2),
        name: "NOTES".into(),
        resource_keys: Vec::new(),
    })
    .unwrap();
    let mut db = b.finish().unwrap();
    let before = db.revision();

    let changes = db
        .delete_style(StyleId(2), TransactionId(1), "purge style")
        .expect("nothing uses NOTES");

    assert!(db.style(StyleId(2)).is_none());
    assert!(db.style(StyleId(1)).is_some(), "Standard stays");
    assert_eq!(changes.after, Revision(before.0 + 1));
}

#[test]
fn a_style_used_by_text_is_refused_by_name_and_changes_nothing() {
    let mut b = base_builder();
    b.insert_style(Style {
        id: StyleId(1),
        name: "Standard".into(),
        resource_keys: Vec::new(),
    })
    .unwrap();
    b.insert_style(Style {
        id: StyleId(2),
        name: "NOTES".into(),
        resource_keys: Vec::new(),
    })
    .unwrap();
    b.insert_entity(raw_entity(5, SpaceId::Model, text(StyleId(2))))
        .unwrap();
    let mut db = b.finish().unwrap();
    let before = db.revision();

    let error = db
        .delete_style(StyleId(2), TransactionId(1), "purge style")
        .expect_err("entity 5 names NOTES");
    let text_error = error.to_string();
    assert!(text_error.contains("style.in_use"), "{text_error}");
    assert!(text_error.contains("NOTES"), "{text_error}");
    assert_eq!(db.revision(), before);
    assert!(db.style(StyleId(2)).is_some());

    // The drawing default cannot be purged even when nothing uses it.
    let error = db
        .delete_style(StyleId(1), TransactionId(2), "purge Standard")
        .expect_err("Standard is the drawing default");
    assert!(
        error.to_string().contains("style.standard_undeletable"),
        "{error}"
    );
    assert_eq!(db.revision(), before);
}

// ---------------------------------------------------------------------------
// Linetype
// ---------------------------------------------------------------------------

#[test]
fn deleting_a_linetype_with_no_name_reference_removes_the_row() {
    let mut b = base_builder();
    b.insert_linetype(LineType {
        id: LinetypeId(1),
        name: "Continuous".into(),
        pattern: LinetypePattern::default(),
        complex: false,
    })
    .unwrap();
    b.insert_linetype(LineType {
        id: LinetypeId(2),
        name: "DASHED".into(),
        pattern: LinetypePattern::default(),
        complex: false,
    })
    .unwrap();
    let mut db = b.finish().unwrap();
    let before = db.revision();

    let changes = db
        .delete_linetype(LinetypeId(2), TransactionId(1), "purge linetype")
        .expect("no layer or entity names DASHED");

    assert!(db.linetype(LinetypeId(2)).is_none());
    assert!(db.linetype(LinetypeId(1)).is_some());
    assert_eq!(changes.after, Revision(before.0 + 1));
}

#[test]
fn a_linetype_referenced_by_a_layer_or_an_entity_is_refused_by_name() {
    let mut b = base_builder();
    b.insert_linetype(LineType {
        id: LinetypeId(1),
        name: "Continuous".into(),
        pattern: LinetypePattern::default(),
        complex: false,
    })
    .unwrap();
    b.insert_linetype(LineType {
        id: LinetypeId(2),
        name: "DASHED".into(),
        pattern: LinetypePattern::default(),
        complex: false,
    })
    .unwrap();
    b.insert_layer(Layer {
        id: LayerId(3),
        name: "WALLS".into(),
        visible: true,
    })
    .unwrap();
    let mut db = b.finish().unwrap();
    db.set_layer_attributes(
        LayerId(3),
        LayerAttributes {
            linetype: "DASHED".into(),
            ..LayerAttributes::default()
        },
        TransactionId(1),
        "layer attributes",
    )
    .unwrap();
    let before = db.revision();

    let error = db
        .delete_linetype(LinetypeId(2), TransactionId(2), "purge linetype")
        .expect_err("layer WALLS names DASHED");
    let text = error.to_string();
    assert!(text.contains("linetype.in_use"), "{text}");
    assert!(text.contains("DASHED"), "{text}");
    assert_eq!(db.revision(), before);
    assert!(db.linetype(LinetypeId(2)).is_some());

    let error = db
        .delete_linetype(LinetypeId(1), TransactionId(3), "purge Continuous")
        .expect_err("Continuous is the drawing default");
    assert!(
        error
            .to_string()
            .contains("linetype.continuous_undeletable"),
        "{error}"
    );
    assert_eq!(db.revision(), before);
}
