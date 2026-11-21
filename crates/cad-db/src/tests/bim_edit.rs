//! The controlled BIM write path (`DrawingDatabase::apply_bim_updates`).
//!
//! What must hold for a BIM property edit to be honest:
//!
//! 1. it is **replace-only** — it overwrites an existing `bim` block and never
//!    attaches one to a non-BIM entity (that would fabricate provenance);
//! 2. it is all-or-nothing — a bad id or an over-budget value leaves the
//!    database, the side table and the revision untouched;
//! 3. the published scope names **exactly** the addressed elements' shards.
//!    `copyable_shard` in `cad-store-zip` copies any shard it was not told about,
//!    so a missing `element()` would save the old `bim` bytes on the second save
//!    while reporting success;
//! 4. it touches nothing else: the definition/relation tables and the untouched
//!    fields of the element stay byte-for-byte equal, so `has_bim()` cannot
//!    flip (minor version never changes).

use std::collections::BTreeSet;

use cad_domain::ymodel::{shard_of, ModelKey};

use super::*;

// ---------------------------------------------------------------- fixtures ---

fn source(id: &str) -> BimSource {
    BimSource {
        connector: "ifc".into(),
        document_id: Some("doc-1".into()),
        id: Some(id.into()),
        native_id: Some("22".into()),
    }
}

/// One element with two psets, so "only the addressed value changed" is a real
/// assertion (one untouched pset, one untouched property inside the edited set).
fn element_for(source_id: &str) -> BimElement {
    BimElement {
        category: Some("IfcWall".into()),
        name: Some("Basic Wall:Generic 200mm".into()),
        source: source(source_id),
        level: Some("level:ifc:doc-1:L1".into()),
        element_type: None,
        material: None,
        psets: vec![
            BimPropertySet {
                name: "Pset_WallCommon".into(),
                properties: vec![
                    BimProperty {
                        name: "IsExternal".into(),
                        value: BimValue::Bool(true),
                    },
                    BimProperty {
                        name: "Reference".into(),
                        value: BimValue::Real {
                            value: 0.2,
                            ifc_type: Some("IfcPositiveLengthMeasure".into()),
                            unit: Some("m".into()),
                        },
                    },
                ],
            },
            BimPropertySet {
                name: "Pset_Other".into(),
                properties: vec![BimProperty {
                    name: "Note".into(),
                    value: BimValue::Text("untouched".into()),
                }],
            },
        ],
        evidence: BimEvidence::Declared,
    }
}

fn definitions() -> BimDefinitions {
    let mut definitions = BimDefinitions::new();
    definitions
        .insert(BimDefinition::Level {
            id: "level:ifc:doc-1:L1".into(),
            name: "Level 1".into(),
            elevation: Some(0.0),
            source: source("level-1"),
            evidence: BimEvidence::Declared,
        })
        .expect("level");
    definitions
}

/// Layer `0`, two model-space lines (only id 1 carries BIM), one definition row
/// and one relation edge.
fn db_with_bim() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .expect("layer");
    builder
        .insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .expect("entity 1");
    builder
        .insert_entity(line_entity(2, point(0.0, 0.0), point(0.0, 1.0)))
        .expect("entity 2");
    builder
        .set_bim_definitions(definitions())
        .expect("definitions");
    builder
        .set_entity_bim(EntityId(1), element_for("wall-1"))
        .expect("bim");
    let mut db = builder.finish().expect("database");
    db.bim_relations
        .insert(BimRelation {
            kind: BimRelationKind::Aggregates,
            parent: BimKey::Entity(EntityId(1)),
            child: BimKey::Source {
                connector: "ifc".into(),
                document_id: "doc-1".into(),
                id: "part-1".into(),
            },
            source: None,
            label: Some("whole-part".into()),
        })
        .expect("relation");
    db
}

fn edited_element() -> BimElement {
    let mut element = element_for("wall-1");
    element.name = Some("Renamed wall".into());
    element.psets[0].properties[0].value = BimValue::Bool(false);
    element
}

// ------------------------------------------------------------------ tests ----

#[test]
fn apply_bim_updates_bumps_revision_and_dirties_the_element_shard() {
    let mut db = db_with_bim();
    let before = db.revision();
    let patch = edited_element();

    let changes = db
        .apply_bim_updates(
            "edit bim",
            TransactionId(7),
            &[(EntityId(1), patch.clone())],
        )
        .expect("apply");

    assert_eq!(changes.database, DatabaseId(1));
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, Revision(before.0 + 1));
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(changes.reason, "edit bim");
    assert_eq!(changes.transaction, TransactionId(7));

    // One metadata update per addressed id, in the caller's order.
    assert_eq!(
        changes.changes,
        vec![ObjectChange::Update(ObjectId(1), ChangeMask::METADATA)]
    );

    // The `bim` block lives in the element row, so exactly that shard is dirty.
    assert_eq!(
        changes.scope.shards,
        BTreeSet::from([(ModelKey::ModelSpace, shard_of(1))])
    );
    // A replace never moves the row and touches no table or header field.
    assert!(changes.scope.prev_shards.is_empty());
    assert!(changes.scope.tables.is_empty());
    assert!(!changes.scope.header);

    assert_eq!(db.bim_element(EntityId(1)), Some(&patch));
}

#[test]
fn apply_bim_updates_is_all_or_nothing() {
    let mut db = db_with_bim();
    let before = db.revision();
    let original = db.bim_element(EntityId(1)).cloned().expect("bim");
    // Entity 2 exists but has no BIM data: the second update is refused, so the
    // first must not have been applied either.
    let updates = [
        (EntityId(1), edited_element()),
        (EntityId(2), element_for("wall-2")),
    ];
    let error = db
        .apply_bim_updates("edit bim", TransactionId(8), &updates)
        .expect_err("refused");
    assert!(
        format!("{error:?}").contains("property.unsupported_key"),
        "{error:?}"
    );

    assert_eq!(db.revision(), before, "no revision bump on failure");
    let after = db.bim_element(EntityId(1)).expect("bim");
    assert_eq!(after.category, original.category);
    assert_eq!(after.name, original.name);
    assert_eq!(after.source, original.source);
    assert_eq!(after.level, original.level);
    assert_eq!(after.element_type, original.element_type);
    assert_eq!(after.material, original.material);
    assert_eq!(after.psets, original.psets);
    assert_eq!(after.evidence, original.evidence);
}

#[test]
fn apply_bim_updates_refuses_a_non_bim_entity() {
    let mut db = db_with_bim();
    let before = db.revision();
    let before_len = db.bim_elements().len();

    let error = db
        .apply_bim_updates(
            "edit bim",
            TransactionId(9),
            &[(EntityId(2), element_for("wall-2"))],
        )
        .expect_err("refused");

    // Entity 2 exists (so this is not `target_missing`) but owns no `bim` block;
    // attaching one here would fabricate BIM identity.
    assert!(
        format!("{error:?}").contains("property.unsupported_key"),
        "{error:?}"
    );
    assert_eq!(db.revision(), before);
    assert_eq!(db.bim_elements().len(), before_len);
    assert!(db.bim_element(EntityId(2)).is_none());
}

#[test]
fn apply_bim_updates_refuses_a_missing_entity() {
    let mut db = db_with_bim();
    let before = db.revision();

    let error = db
        .apply_bim_updates(
            "edit bim",
            TransactionId(10),
            &[(EntityId(99), element_for("wall-99"))],
        )
        .expect_err("refused");
    assert!(
        format!("{error:?}").contains("property.target_missing"),
        "{error:?}"
    );
    assert_eq!(db.revision(), before);
    assert_eq!(db.bim_elements().len(), 1);
}

#[test]
fn apply_bim_updates_refuses_a_budget_breach() {
    let mut db = db_with_bim();
    let before = db.revision();
    let original = db.bim_element(EntityId(1)).cloned().expect("bim");

    let mut too_long = edited_element();
    too_long.psets[1].properties.push(BimProperty {
        name: "Description".into(),
        // One byte over the 4096-byte per-string budget: refused, never
        // truncated.
        value: BimValue::Text("x".repeat(crate::BIM_MAX_STRING_BYTES + 1)),
    });
    let error = db
        .apply_bim_updates("edit bim", TransactionId(11), &[(EntityId(1), too_long)])
        .expect_err("refused");
    assert!(
        format!("{error:?}").contains("property.bim_too_long"),
        "{error:?}"
    );
    assert_eq!(db.revision(), before);
    assert_eq!(db.bim_element(EntityId(1)), Some(&original));
}

#[test]
fn apply_bim_updates_refuses_a_duplicate_id() {
    let mut db = db_with_bim();
    let before = db.revision();

    let updates = [
        (EntityId(1), edited_element()),
        (EntityId(1), element_for("wall-1")),
    ];
    let error = db
        .apply_bim_updates("edit bim", TransactionId(12), &updates)
        .expect_err("refused");
    assert!(matches!(error, CadError::Invariant(_)), "{error:?}");
    assert_eq!(db.revision(), before);
}

#[test]
fn apply_bim_updates_with_an_empty_batch_is_a_no_op() {
    let mut db = db_with_bim();
    let before = db.revision();
    let revision_before = db.revision();

    let changes = db
        .apply_bim_updates("edit bim", TransactionId(13), &[])
        .expect("no-op");

    assert_eq!(db.revision(), revision_before);
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, before);
    assert!(changes.changes.is_empty());
    assert!(changes.scope.is_empty());
}

#[test]
fn apply_bim_updates_only_touches_the_addressed_value() {
    let mut db = db_with_bim();
    let original = db.bim_element(EntityId(1)).cloned().expect("bim");
    let definitions_before = db.bim_definitions().clone();
    let relations_before = db.bim_relations().clone();
    let snapshot_before = snapshot_of(&db);
    assert!(snapshot_before.has_bim());

    let before = db.revision();
    let mut edited = original.clone();
    edited.psets[0].properties[0].value = BimValue::Bool(false);
    let changes = db
        .apply_bim_updates("edit bim", TransactionId(14), &[(EntityId(1), edited)])
        .expect("apply");
    assert_eq!(changes.before, before);

    let after = db.bim_element(EntityId(1)).expect("bim");
    // The addressed value changed...
    assert_eq!(after.psets[0].properties[0].value, BimValue::Bool(false));
    // ...and nothing else in the element did.
    assert_eq!(after.source, original.source);
    assert_eq!(after.evidence, original.evidence);
    assert_eq!(after.category, original.category);
    assert_eq!(after.name, original.name);
    assert_eq!(after.level, original.level);
    assert_eq!(after.psets[0].name, original.psets[0].name);
    assert_eq!(
        after.psets[0].properties[1],
        original.psets[0].properties[1]
    );
    assert_eq!(after.psets[1], original.psets[1]);
    // The definition/relation tables are untouched by a per-element write.
    assert_eq!(db.bim_definitions(), &definitions_before);
    assert_eq!(db.bim_relations(), &relations_before);

    // Replace-only: the block still exists, so the document is still BIM and
    // `has_bim()` (the minor-version switch) cannot flip.
    let snapshot_after = snapshot_of(&db);
    assert!(snapshot_after.has_bim());
}

fn snapshot_of(db: &DrawingDatabase) -> crate::DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        db.revision(),
    )
    .expect("snapshot")
}
