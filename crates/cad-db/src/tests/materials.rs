//! Document materials (`tables/materials.json`, minor 19): the table write path
//! and its snapshot round trip.
//!
//! The contract: one paint is one revision with only `TableKind::Materials`
//! dirty; an identical paint is refused as a no-op; a deleted entity loses its
//! assignment; the snapshot encodes the table only when the document holds a
//! material; and a malformed table (duplicate name, dangling assignment) never
//! loads.

use cad_domain::material::{builtin_material, markers, Material, MaterialDocument, MaterialSeq};
use cad_domain::ymodel::id::PersistentIdOf;
use cad_domain::{
    DatabaseId, DocumentIdentity, Point3, Revision, SemanticGeometry, SpaceId, TransactionId,
    UnitContext,
};
use serde_json::Value;

use super::*;

fn empty_db() -> DrawingDatabase {
    DrawingDatabaseBuilder::new(DatabaseId(1))
        .finish()
        .expect("an empty database")
}

fn painted_db() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for id in [7_u128, 8] {
        let entity = raw_entity(
            id,
            SpaceId::Model,
            SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
        );
        builder.insert_entity(entity).unwrap();
    }
    let mut materials = MaterialDocument::new();
    let red = materials.ensure("Red", [178, 34, 34], 0.0).unwrap();
    materials.assign(EntityId(7), red).unwrap();
    builder.set_materials(materials).unwrap();
    builder.finish().expect("the database is valid")
}

fn snapshot(db: &DrawingDatabase) -> DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        db.revision(),
    )
    .unwrap()
}

#[test]
fn a_paint_is_one_revision_and_one_materials_dirty_mark() {
    let mut db = painted_db();
    let before = db.revision();
    let mut next = db.materials().clone();
    let blue = next.ensure("Blue", [41, 128, 185], 0.0).unwrap();
    next.assign(EntityId(8), blue).unwrap();
    let changes = db
        .apply_material_document(next.clone(), TransactionId(1), "PAINT Blue")
        .expect("apply");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, db.revision());
    assert!(changes.changes.is_empty(), "no geometry row moved");
    assert!(changes.scope.tables.contains(&TableKind::Materials));
    assert!(!changes.scope.header);
    assert_eq!(
        db.materials()
            .assignment(EntityId(8))
            .map(|material| material.name.as_str()),
        Some("Blue")
    );
    assert_eq!(db.materials(), &next);
}

#[test]
fn an_identical_paint_is_refused_as_a_no_op() {
    let mut db = painted_db();
    let revision = db.revision();
    let error = db
        .apply_material_document(db.materials().clone(), TransactionId(2), "PAINT Red")
        .expect_err("nothing changes");
    assert!(
        format!("{error:?}").contains(markers::MATERIAL_NO_CHANGE),
        "{error:?}"
    );
    assert_eq!(db.revision(), revision);
}

#[test]
fn deleting_a_painted_entity_drops_its_assignment() {
    let mut db = painted_db();
    assert!(db.materials().assignment(EntityId(7)).is_some());
    let revision = db.revision();
    db.apply_drawing_changes("ERASE", TransactionId(3), vec![(EntityId(7), None)])
        .expect("delete");
    assert_eq!(db.revision(), Revision(revision.0 + 1));
    assert!(
        db.materials().assignment(EntityId(7)).is_none(),
        "the assignment is pruned with the entity"
    );
    // The material row itself stays: the table is a palette, not a per-entity
    // side effect.
    assert!(db.materials().resolve("Red").is_some());
}

#[test]
fn the_snapshot_writes_the_table_only_when_it_holds_a_material_and_reads_it_back() {
    let db = painted_db();
    let snapshot = snapshot(&db);
    assert!(snapshot.has_materials());
    let rows = snapshot
        .tables
        .get(TableKind::Materials)
        .expect("table entry");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["seq"], Value::from(1));
    assert_eq!(rows[0]["name"], Value::from("Red"));
    assert_eq!(
        rows[0]["entities"],
        Value::Array(vec![Value::from(EntityId(7).persistent().raw())])
    );

    let loaded = DrawingDatabase::from_snapshot(&snapshot).unwrap();
    assert_eq!(loaded.materials(), db.materials());
}

#[test]
fn a_document_without_the_table_loads_with_no_materials() {
    let db = empty_db();
    let snapshot = snapshot(&db);
    assert!(!snapshot.has_materials());
    assert!(snapshot.tables.get(TableKind::Materials).is_none());
    let loaded = DrawingDatabase::from_snapshot(&snapshot).unwrap();
    assert!(loaded.materials().is_empty());
}

#[test]
fn a_duplicate_name_refuses_the_load() {
    let db = painted_db();
    let mut snapshot = snapshot(&db);
    let mut first = snapshot.tables.get(TableKind::Materials).unwrap()[0].clone();
    let mut second = first.clone();
    first["seq"] = Value::from(1);
    first["name"] = Value::from("Red");
    second["seq"] = Value::from(2);
    second["name"] = Value::from("red");
    second["entities"] = Value::Array(Vec::new());
    snapshot
        .tables
        .set(TableKind::Materials, vec![first, second]);
    let error = DrawingDatabase::from_snapshot(&snapshot).unwrap_err();
    assert!(
        format!("{error:?}").contains("duplicate material name"),
        "{error:?}"
    );
}

#[test]
fn an_assignment_to_a_missing_entity_refuses_the_load() {
    let db = painted_db();
    let mut snapshot = snapshot(&db);
    let mut row = snapshot.tables.get(TableKind::Materials).unwrap()[0].clone();
    row["entities"] = Value::Array(vec![Value::from(EntityId(999).persistent().raw())]);
    snapshot.tables.set(TableKind::Materials, vec![row]);
    let error = DrawingDatabase::from_snapshot(&snapshot).unwrap_err();
    assert!(
        format!("{error:?}").contains("is painted but does not exist"),
        "{error:?}"
    );
}

#[test]
fn an_opacity_outside_the_range_refuses_the_load() {
    let db = painted_db();
    let mut snapshot = snapshot(&db);
    let mut row = snapshot.tables.get(TableKind::Materials).unwrap()[0].clone();
    row["transparency"] = Value::from(1.5);
    snapshot.tables.set(TableKind::Materials, vec![row]);
    let error = DrawingDatabase::from_snapshot(&snapshot).unwrap_err();
    assert!(
        format!("{error:?}").contains(markers::MATERIAL_TRANSPARENCY_INVALID),
        "{error:?}"
    );
}

#[test]
fn the_builtin_palette_is_flat_and_resolves_case_insensitively() {
    let (name, color, transparency) = builtin_material("glass").expect("Glass is built in");
    assert_eq!(name, "Glass");
    assert!(transparency > 0.0 && transparency < 1.0);
    assert_eq!(color.len(), 3);
    assert!(builtin_material("not-a-material").is_none());
}

#[test]
fn ensure_does_not_silently_edit_an_existing_material() {
    let mut document = MaterialDocument::new();
    let seq = document.ensure("Red", [178, 34, 34], 0.0).unwrap();
    let again = document.ensure("red", [0, 0, 0], 0.9).unwrap();
    assert_eq!(seq, again);
    let row: &Material = document.get(seq).unwrap();
    assert_eq!(row.color, [178, 34, 34]);
    assert_eq!(row.transparency, 0.0);
    assert_eq!(document.materials().len(), 1);
    assert_eq!(
        document.ensure("Blue", [0, 0, 255], 0.0).unwrap(),
        MaterialSeq(2)
    );
}

#[test]
fn a_face_paint_is_encoded_as_faces_and_reads_back() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(raw_entity(
            7,
            SpaceId::Model,
            SemanticGeometry::Mesh(cad_domain::Mesh {
                vertices: vec![
                    Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 0.0,
                        y: 1.0,
                        z: 0.0,
                    },
                ],
                triangles: vec![[0, 1, 2]],
                normals: Vec::new(),
                face_sources: vec![Some(cad_domain::SubElementId {
                    source_key: "skp:face:0".into(),
                    topology_revision: Revision(0),
                })],
                colors: Vec::new(),
                uvs: Vec::new(),
                texture: None,
                pbr: None,
            }),
        ))
        .unwrap();
    let mut materials = MaterialDocument::new();
    let red = materials.ensure("Red", [178, 34, 34], 0.0).unwrap();
    materials
        .assign_face(EntityId(7), "skp:face:0", red)
        .unwrap();
    builder.set_materials(materials).unwrap();
    let db = builder.finish().unwrap();

    let snapshot = snapshot(&db);
    assert!(snapshot.has_face_materials());
    let rows = snapshot
        .tables
        .get(TableKind::Materials)
        .expect("the material table");
    assert_eq!(rows[0]["faces"][0]["face"], Value::from("skp:face:0"));
    assert_eq!(rows[0]["entities"], Value::from(Vec::<Value>::new()));

    let loaded = DrawingDatabase::from_snapshot(&snapshot).unwrap();
    let row = loaded
        .materials()
        .face_assignment(EntityId(7), "skp:face:0")
        .expect("the face paint survives");
    assert_eq!(row.name, "Red");
    assert!(loaded
        .materials()
        .face_assignment(EntityId(7), "skp:face:1")
        .is_none());
}

#[test]
fn erasing_an_entity_drops_its_face_paints_and_a_stale_face_never_validates_a_missing_material() {
    let mut document = MaterialDocument::new();
    let red = document.ensure("Red", [178, 34, 34], 0.0).unwrap();
    document.assign(EntityId(7), red).unwrap();
    document
        .assign_face(EntityId(7), "skp:face:0", red)
        .unwrap();
    document
        .assign_face(EntityId(8), "skp:face:2", red)
        .unwrap();
    document.clear(EntityId(7));
    assert!(document.assignment(EntityId(7)).is_none());
    assert!(document
        .face_assignment(EntityId(7), "skp:face:0")
        .is_none());
    assert!(document
        .face_assignment(EntityId(8), "skp:face:2")
        .is_some());
    document
        .assign_face(EntityId(8), "skp:face:3", MaterialSeq(99))
        .expect_err("an absent material is refused");
}
