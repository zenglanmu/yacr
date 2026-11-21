//! The ADR-0043 "save twice" contract: a parameter commit must dirty its own
//! shard, or the second save silently re-emits the old bytes.
//!
//! This is a real save through [`ZipDocumentStore`] with a previous archive, so
//! the raw-copy decision is actually exercised: if
//! `apply_parametric_update` omitted the element shard from its `ChangeScope`,
//! the edited shard would be copied from the first archive and the reopened
//! radius would still be the old value.

use cad_db::{
    ChangeScope, DbEntity, DbObject, DocumentSnapshot, DocumentStore, DrawingDatabase,
    DrawingDatabaseBuilder, Layer, Layout, MemorySink, MemorySource, ParamBindingPatch, ParamPatch,
};
use cad_domain::param::{Expr, ParamBindings, ParamRow, ParamSeq, ParamTable};
use cad_domain::ymodel::ModelKey;
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, LayoutId, Point3, Revision, SemanticGeometry,
    SpaceId, TransactionId, UnitContext,
};
use cad_store_zip::ZipDocumentStore;

fn circle(id: u128, radius: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: cad_domain::ObjectId(id),
            type_key: "AcDbCircle".to_string(),
            revision: Revision(0),
            source_handle: Some(format!("{id:X}")),
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Circle {
            center: Point3::default(),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        },
        draw_order: 0,
    }
}

fn database(radius: f64) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(7));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("insert layer");
    builder
        .insert_layout(Layout {
            id: LayoutId(0),
            name: "Model".to_string(),
            viewports: vec![],
        })
        .expect("insert layout");
    builder
        .insert_entity(circle(1, radius))
        .expect("insert circle");
    builder.finish().expect("build the fixture database")
}

fn snapshot_of(db: &DrawingDatabase, revision: Revision) -> DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(3),
        Vec::new(),
        revision,
    )
    .expect("encode the snapshot")
}

fn write(
    store: &mut ZipDocumentStore,
    snapshot: &DocumentSnapshot,
    dirty: &ChangeScope,
) -> Vec<u8> {
    let mut sink = MemorySink::new();
    store
        .write(snapshot, dirty, &mut sink)
        .expect("write the archive");
    sink.committed_bytes().expect("the save committed").to_vec()
}

fn read(bytes: &[u8]) -> DrawingDatabase {
    let loaded = ZipDocumentStore::new()
        .read(&mut MemorySource::new(bytes.to_vec()))
        .expect("the archive reopens");
    DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate")
}

fn radius(db: &DrawingDatabase) -> f64 {
    match db.entity(EntityId(1)).expect("circle").geometry {
        SemanticGeometry::Circle { radius, .. } => radius,
        ref other => panic!("fixture is not a circle: {other:?}"),
    }
}

/// A parameter create + binding commit survives a save that has a previous
/// archive to raw-copy from, and a second identical save is byte-stable.
#[test]
fn a_parametric_commit_survives_the_second_save() {
    // First archive: the circle at radius 10, no parameters.
    let base = database(10.0);
    let first_snapshot = snapshot_of(&base, Revision(1));
    let first = write(
        &mut ZipDocumentStore::new(),
        &first_snapshot,
        &ChangeScope::default(),
    );

    // The edit: create parameter `r = 20`, bind `Circle.radius` to it. The
    // database's own ChangeScope is what the save will consume.
    let table = ParamTable::from_rows([ParamRow::new(
        ParamSeq(1),
        "r",
        cad_domain::param::ParamKind::Length,
        Expr::num(20.0),
    )
    .expect("row")])
    .expect("table");
    let mut edited = base;
    let outcome = edited
        .apply_parametric_update(
            "bind radius",
            TransactionId(9),
            &[ParamPatch {
                seq: ParamSeq(1),
                before: None,
                after: table.get(ParamSeq(1)).cloned(),
            }],
            &[ParamBindingPatch {
                id: EntityId(1),
                before: None,
                after: Some(
                    ParamBindings::from_entries([(
                        "radius".to_string(),
                        Expr::reference(ParamSeq(1)),
                    )])
                    .expect("bindings"),
                ),
                before_geometry: None,
                after_geometry: None,
            }],
        )
        .expect("apply the parametric update");
    assert_eq!(radius(&edited), 20.0, "the candidate re-derived the radius");
    assert!(
        outcome
            .changes
            .scope
            .touches_shard(ModelKey::ModelSpace, cad_domain::ymodel::Shard(0)),
        "the commit must dirty the element's shard"
    );
    assert!(
        outcome
            .changes
            .scope
            .tables
            .contains(&cad_db::TableKind::Parameters),
        "the commit must dirty the parameter table"
    );

    // Second save: previous archive plus exactly the commit's scope.
    let second_snapshot = snapshot_of(&edited, Revision(2));
    let second = write(
        &mut ZipDocumentStore::new().with_previous(Some(first)),
        &second_snapshot,
        &outcome.changes.scope,
    );
    let reopened = read(&second);
    assert_eq!(
        radius(&reopened),
        20.0,
        "a missed shard would reopen the raw-copied radius 10"
    );
    assert_eq!(
        reopened.parameters().get(ParamSeq(1)).expect("row").name,
        "r"
    );

    // Third save: same snapshot, empty scope. Every entry is raw-copied, so the
    // per-entry content checksums must not move (raw copy preserves compressed
    // bytes; the zip container's own offsets may differ, so the contract is
    // entry-content stability, not whole-file identity).
    let third = write(
        &mut ZipDocumentStore::new().with_previous(Some(second.clone())),
        &second_snapshot,
        &ChangeScope::default(),
    );
    assert_eq!(
        manifest_checksums(&third),
        manifest_checksums(&second),
        "an unchanged parameter snapshot must not change any entry's bytes"
    );
    assert_eq!(radius(&read(&third)), 20.0);
}

/// The element shard is really re-encoded, not copied: its manifest checksum
/// changes while the header/document entry does not (parameter identity is
/// table-local, no header counter).
#[test]
fn the_parametric_commit_rewrites_the_element_shard_not_the_header() {
    let base = database(10.0);
    let first = write(
        &mut ZipDocumentStore::new(),
        &snapshot_of(&base, Revision(1)),
        &ChangeScope::default(),
    );

    let mut edited = base;
    let outcome = edited
        .apply_parametric_update(
            "bind radius",
            TransactionId(9),
            &[ParamPatch {
                seq: ParamSeq(1),
                before: None,
                after: Some(
                    ParamRow::new(
                        ParamSeq(1),
                        "r",
                        cad_domain::param::ParamKind::Length,
                        Expr::num(20.0),
                    )
                    .expect("row"),
                ),
            }],
            &[ParamBindingPatch {
                id: EntityId(1),
                before: None,
                after: Some(
                    ParamBindings::from_entries([(
                        "radius".to_string(),
                        Expr::reference(ParamSeq(1)),
                    )])
                    .expect("bindings"),
                ),
                before_geometry: None,
                after_geometry: None,
            }],
        )
        .expect("apply");
    let second = write(
        &mut ZipDocumentStore::new().with_previous(Some(first.clone())),
        &snapshot_of(&edited, Revision(2)),
        &outcome.changes.scope,
    );

    let first = manifest_checksums(&first);
    let second = manifest_checksums(&second);
    let shard = format!(
        "elements/{}-0.json",
        ModelKey::ModelSpace.persistent().raw()
    );
    assert_ne!(
        first.get(&shard),
        second.get(&shard),
        "the edited shard must be rewritten"
    );
    assert_eq!(
        first.get("document.json"),
        second.get("document.json"),
        "no header counter backs parameters, so document.json must not change"
    );
}

/// `sha256` per entry path from the manifest.
fn manifest_checksums(bytes: &[u8]) -> std::collections::BTreeMap<String, String> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("zip");
    let mut manifest = Vec::new();
    archive
        .by_name("manifest.json")
        .expect("manifest")
        .read_to_end(&mut manifest)
        .expect("read manifest");
    let value: serde_json::Value = serde_json::from_slice(&manifest).expect("json");
    value["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|record| {
            (
                record["path"].as_str().expect("path").to_string(),
                record["sha256"].as_str().expect("sha").to_string(),
            )
        })
        .collect()
}
