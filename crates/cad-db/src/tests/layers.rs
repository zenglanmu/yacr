//! Layer table structural writes: create / rename / delete.
//!
//! Every contract here pins one of the module's rules
//! ([`crate::drawing::layer_edit`]): validate-before-mutate (a refusal leaves the
//! revision, the layer table and the entities byte-identical), a single revision
//! bump per real change, case-insensitive name uniqueness, monotonic layer ids,
//! and the two delete refusals (default layer `0`, an in-use layer).

use super::*;

/// A database with the default layer `0`, a named layer `WALLS` (id 3) and one
/// model-space line on `WALLS`, so rename/delete have a real referencing entity.
fn layer_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(0u128, "0"), (3, "WALLS")] {
        b.insert_layer(Layer {
            id: LayerId(id),
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    let mut entity = raw_entity(
        1,
        SpaceId::Model,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    );
    entity.layer = LayerId(3);
    b.insert_entity(entity).unwrap();
    b.finish().unwrap()
}

/// The whole layer table as comparable rows, for "nothing changed" assertions.
fn table(db: &DrawingDatabase) -> Vec<(u128, String, bool)> {
    db.layers()
        .map(|layer| (layer.id.0, layer.name.clone(), layer.visible))
        .collect()
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

#[test]
fn creating_a_layer_adds_a_visible_row_and_bumps_the_revision_once() {
    let mut db = layer_db();
    let before_revision = db.revision();
    let next_id = db.next_layer_id();

    let (id, changeset) = db
        .create_layer("  DOORS  ", TransactionId(9), "create layer")
        .expect("a fresh legal name is accepted");

    // The stored name is trimmed; the row is visible (the DWG default).
    assert_eq!(db.layer(id).expect("the new row").name, "DOORS");
    assert!(db.layer(id).expect("the new row").visible);
    // Id allocation is the exposed monotonic allocator, never an implicit one.
    assert_eq!(id, next_id);
    assert_eq!(id, LayerId(4), "max(existing) = 3, so the next id is 4");
    assert_eq!(db.next_layer_id(), LayerId(5));

    // Exactly one revision bump, and the change set carries no object change
    // (layer edits are revision-tagged, not entity-addressed).
    assert_eq!(changeset.before, before_revision);
    assert_eq!(changeset.after, Revision(before_revision.0 + 1));
    assert_eq!(changeset.after, db.revision());
    assert!(changeset.changes.is_empty());
    assert_eq!(changeset.transaction, TransactionId(9));
    assert_eq!(changeset.reason, "create layer");
}

#[test]
fn a_create_refuses_a_case_insensitive_duplicate_and_changes_nothing() {
    let mut db = layer_db();
    let before_revision = db.revision();
    let before_table = table(&db);

    let error = db
        .create_layer("walls", TransactionId(1), "duplicate")
        .expect_err("WALLS already exists; names are case-insensitive");
    assert!(
        error.to_string().contains("layer.duplicate_name"),
        "{error}"
    );

    assert_eq!(db.revision(), before_revision);
    assert_eq!(table(&db), before_table);
    assert_eq!(db.next_layer_id(), LayerId(4), "no id was consumed");
}

#[test]
fn a_create_refuses_empty_whitespace_only_and_illegal_names() {
    let mut db = layer_db();
    let before_revision = db.revision();
    let before_table = table(&db);

    for (name, code) in [("", "layer.name_empty"), ("   \t ", "layer.name_empty")] {
        let error = db
            .create_layer(name, TransactionId(1), "empty")
            .expect_err("an empty name is refused");
        assert!(error.to_string().contains(code), "{name:?}: {error}");
    }
    // The documented AutoCAD-illegal set, one representative per character.
    for illegal in [
        '<', '>', '/', '\\', '"', ':', ';', '?', '*', '|', ',', '=', '`',
    ] {
        let name = format!("A{illegal}B");
        let error = db
            .create_layer(&name, TransactionId(1), "illegal")
            .expect_err("an illegal character is refused");
        assert!(
            error.to_string().contains("layer.name_illegal_char"),
            "{name}: {error}"
        );
    }

    assert_eq!(db.revision(), before_revision);
    assert_eq!(table(&db), before_table);
}

// ---------------------------------------------------------------------------
// Rename
// ---------------------------------------------------------------------------

#[test]
fn renaming_a_layer_updates_the_row_and_bumps_the_revision_once() {
    let mut db = layer_db();
    let before_revision = db.revision();

    let changeset = db
        .rename_layer(LayerId(3), "Partitions", TransactionId(2), "rename")
        .expect("a fresh legal name is accepted");

    assert_eq!(db.layer(LayerId(3)).expect("the row").name, "Partitions");
    assert_eq!(changeset.before, before_revision);
    assert_eq!(changeset.after, Revision(before_revision.0 + 1));
    assert_eq!(db.revision(), changeset.after);
    assert!(changeset.changes.is_empty());
    // The entity's own reference is by id, so the rename retargets nothing and
    // leaves no dangling reference.
    assert_eq!(db.entity(EntityId(1)).expect("the line").layer, LayerId(3));
}

#[test]
fn a_rename_refuses_another_layers_name_case_insensitively() {
    let mut db = layer_db();
    let before_revision = db.revision();
    let before_table = table(&db);

    let error = db
        .rename_layer(LayerId(3), "0", TransactionId(2), "rename onto 0")
        .expect_err("layer '0' exists");
    assert!(
        error.to_string().contains("layer.duplicate_name"),
        "{error}"
    );

    let error = db
        .rename_layer(LayerId(3), " 0 ", TransactionId(2), "trimmed duplicate")
        .expect_err("the trimmed name is still a duplicate");
    assert!(
        error.to_string().contains("layer.duplicate_name"),
        "{error}"
    );

    assert_eq!(db.revision(), before_revision);
    assert_eq!(table(&db), before_table);
}

#[test]
fn a_pure_case_rename_is_allowed_but_an_identical_name_is_a_no_op() {
    let mut db = layer_db();
    let before_revision = db.revision();

    // The renamed row keeps its own name: a case change is a real change.
    let changeset = db
        .rename_layer(LayerId(3), "walls", TransactionId(2), "case change")
        .expect("a layer may change its own casing");
    assert_eq!(db.layer(LayerId(3)).expect("the row").name, "walls");
    assert_eq!(changeset.after, Revision(before_revision.0 + 1));

    // The very same name is an authoritative no-op: no bump, no fabricated change.
    let no_op = db
        .rename_layer(LayerId(3), "walls", TransactionId(3), "same name")
        .expect("an identical rename is accepted as a no-op");
    assert_eq!(no_op.before, no_op.after);
    assert!(no_op.changes.is_empty());
    assert_eq!(db.revision(), no_op.after);
}

#[test]
fn renaming_an_unknown_layer_is_refused_without_any_change() {
    let mut db = layer_db();
    let before_revision = db.revision();
    let before_table = table(&db);

    let error = db
        .rename_layer(LayerId(77), "GHOST", TransactionId(2), "unknown")
        .expect_err("id 77 does not exist");
    assert!(error.to_string().contains("layer.unknown_id"), "{error}");

    assert_eq!(db.revision(), before_revision);
    assert_eq!(table(&db), before_table);
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

#[test]
fn deleting_an_unused_layer_removes_the_row_and_bumps_the_revision_once() {
    let mut db = layer_db();
    let (id, _) = db
        .create_layer("DOORS", TransactionId(1), "create")
        .expect("fresh name");
    let before_revision = db.revision();
    let count_before = db.layers().count();

    let changeset = db
        .delete_layer(id, TransactionId(2), "delete unused")
        .expect("nothing references DOORS");

    assert!(db.layer(id).is_none());
    assert_eq!(db.layers().count(), count_before - 1);
    assert_eq!(changeset.before, before_revision);
    assert_eq!(changeset.after, Revision(before_revision.0 + 1));
    assert_eq!(db.revision(), changeset.after);
    assert!(changeset.changes.is_empty());
}

#[test]
fn deleting_a_layer_in_use_is_refused_naming_the_count_and_changes_nothing() {
    let mut db = layer_db();
    let before_revision = db.revision();
    let before_table = table(&db);

    let error = db
        .delete_layer(LayerId(3), TransactionId(2), "delete in use")
        .expect_err("entity 1 is on WALLS");
    let text = error.to_string();
    assert!(text.contains("layer.in_use"), "{text}");
    assert!(text.contains('1'), "the refusal names the count: {text}");
    assert!(text.contains("WALLS"), "{text}");

    assert_eq!(db.revision(), before_revision);
    assert_eq!(table(&db), before_table);
    // No entity was retargeted and the reference is intact.
    assert_eq!(db.entity(EntityId(1)).expect("the line").layer, LayerId(3));
}

#[test]
fn a_block_member_also_makes_a_layer_in_use() {
    // Block members live in the same `entities` table, so a layer used only by a
    // block child must still refuse deletion.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layer(Layer {
        id: LayerId(2),
        name: "BLOCKCHILD".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(5)],
        dynamic_visibility: None,
    })
    .unwrap();
    let mut member = raw_entity(
        5,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    );
    member.layer = LayerId(2);
    b.insert_entity(member).unwrap();
    let mut db = b.finish().unwrap();

    let error = db
        .delete_layer(LayerId(2), TransactionId(1), "delete block layer")
        .expect_err("block member 5 is on BLOCKCHILD");
    assert!(error.to_string().contains("layer.in_use"), "{error}");
}

#[test]
fn the_default_layer_cannot_be_deleted() {
    let mut db = layer_db();
    let before_revision = db.revision();
    let before_table = table(&db);

    let error = db
        .delete_layer(LayerId(0), TransactionId(1), "delete 0")
        .expect_err("layer '0' is the default layer");
    assert!(
        error.to_string().contains("layer.default_undeletable"),
        "{error}"
    );

    assert_eq!(db.revision(), before_revision);
    assert_eq!(table(&db), before_table);
}

#[test]
fn deleting_an_unknown_layer_is_refused_without_any_change() {
    let mut db = layer_db();
    let before_revision = db.revision();
    let before_table = table(&db);

    let error = db
        .delete_layer(LayerId(99), TransactionId(1), "unknown")
        .expect_err("id 99 does not exist");
    assert!(error.to_string().contains("layer.unknown_id"), "{error}");

    assert_eq!(db.revision(), before_revision);
    assert_eq!(table(&db), before_table);
}

#[test]
fn a_deleted_layer_id_is_never_reused() {
    let mut db = layer_db();
    let (first, _) = db
        .create_layer("DOORS", TransactionId(1), "create")
        .expect("fresh name");
    assert_eq!(first, LayerId(4));
    db.delete_layer(first, TransactionId(2), "delete")
        .expect("unused layer");
    assert_eq!(
        db.next_layer_id(),
        LayerId(5),
        "the allocator is not rewound by a delete"
    );

    let (second, _) = db
        .create_layer("WINDOWS", TransactionId(3), "create")
        .expect("fresh name");
    assert_eq!(second, LayerId(5), "a deleted id is never handed out again");
    // Deleting the largest row does not make that id reusable either.
    db.delete_layer(second, TransactionId(4), "delete again")
        .expect("unused layer");
    let (third, _) = db
        .create_layer("ROOF", TransactionId(5), "create")
        .expect("fresh name");
    assert_eq!(third, LayerId(6));
}
