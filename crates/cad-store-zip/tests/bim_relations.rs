//! BIM relation rows through a real archive (ADR-0042 D3 tail/D12).
//!
//! `cad-db` has its own codec/shape tests; this file proves the edge rows survive
//! the actual `.ymodel` container and hydrate back into a database. Kept apart
//! from `tests/bim_minor.rs` so neither file approaches the 1000-line gate.

use cad_db::{
    BimKey, BimRelation, BimRelationKind, BimRelations, ChangeScope, DocumentSnapshot,
    DocumentStore, DrawingDatabase, DrawingDatabaseBuilder, MemorySink, MemorySource, TableKind,
};
use cad_domain::{DatabaseId, DocumentIdentity, LayerId, Revision, UnitContext};
use cad_store_zip::ZipDocumentStore;

fn source_key(connector: &str, document: &str, id: &str) -> BimKey {
    BimKey::Source {
        connector: connector.to_string(),
        document_id: document.to_string(),
        id: id.to_string(),
    }
}

fn relations() -> BimRelations {
    let mut relations = BimRelations::new();
    relations
        .insert(BimRelation {
            kind: BimRelationKind::Aggregates,
            parent: source_key("ifc", "doc-1", "Level 0"),
            child: source_key("ifc", "doc-1", "Wall 1"),
            source: Some(source_key("ifc", "doc-1", "Rel 1")),
            label: Some("Level 0 aggregates Wall 1".to_string()),
        })
        .expect("aggregates");
    relations
        .insert(BimRelation {
            kind: BimRelationKind::Group,
            parent: BimKey::Entity(cad_domain::EntityId(3)),
            child: source_key("ifc", "doc-1", "Wall 1"),
            source: None,
            label: None,
        })
        .expect("group");
    relations
}

fn database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(cad_db::Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    builder.set_bim_relations(relations()).expect("relations");
    builder.finish().expect("database")
}

fn snapshot_of(db: &DrawingDatabase) -> DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        Revision(2),
    )
    .expect("snapshot")
}

#[test]
fn relation_rows_round_trip_through_an_archive() {
    let snapshot = snapshot_of(&database());
    let rows = snapshot
        .tables
        .get(TableKind::BimRelations)
        .expect("the writer emitted the relation table");
    assert_eq!(rows.len(), 2);

    let mut sink = MemorySink::new();
    ZipDocumentStore::new()
        .write(&snapshot, &ChangeScope::default(), &mut sink)
        .expect("write");
    let bytes = sink.committed_bytes().expect("committed").to_vec();

    let mut source = MemorySource::new(bytes);
    let loaded = ZipDocumentStore::new().read(&mut source).expect("read");
    assert_eq!(
        loaded.snapshot.tables.get(TableKind::BimRelations),
        Some(rows),
        "the archive carries the rows verbatim"
    );

    let hydrated = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    assert_eq!(hydrated.bim_relations(), &relations());

    // Saving the hydrated document is a fixed point for the edge table.
    let resaved = snapshot_of(&hydrated);
    assert_eq!(
        resaved.tables.get(TableKind::BimRelations),
        Some(rows),
        "a reopen + resave does not reorder or drop edges"
    );
}
