//! `tables/parameters.json` and the element `params` block (ADR-0043 P0/D11).
//!
//! The store treats parameter rows as opaque `Value`s: `cad-db`'s snapshot codec
//! owns their shape, so these tests assert the bytes survive a round trip and that
//! the per-document minor tier is `8` only when the document actually carries a
//! parameter table or an element `params` block.
//!
//! Kept out of `tests/archive.rs` because that file is at the 1000-line gate.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use cad_db::{
    table_entry, ChangeScope, DocumentSnapshot, DocumentStore, ElementSnapshot, MemorySink,
    MemorySource, ModelKind, ModelRecord, ShardSnapshot, TableKind,
};
use cad_domain::ymodel::{DocumentHeader, IdCounters, ModelKey, PersistentIdOf, Shard};
use cad_domain::{
    BundledKind, DatabaseId, DocumentIdentity, EntityId, Revision, Unit, UnitContext,
};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

use common::with_manifest;

fn header() -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(5),
        units: UnitContext {
            source: Unit::Millimeter,
            display: Unit::Meter,
            display_per_source: Some(0.001),
            decimal_places: 4,
        },
        identity: DocumentIdentity::Temporary(1),
        resource_keys: Vec::new(),
        embed_resources: false,
        variables: serde_json::json!({}),
        next: IdCounters {
            entity: 2,
            block: 1,
            layer: 1,
            layout: 1,
            style: 1,
            dim_style: 1,
            mleader_style: 1,
            linetype: 1,
            scale: 1,
        },
    }
}

fn element(seq: u128) -> Value {
    serde_json::json!({
        "class": "cad:Element",
        "id": EntityId(seq).persistent().raw(),
        "type_key": "AcDbLine",
        "space": {"kind": "model"},
        "layer": 1,
        "source_handle": null,
        "invisible": false,
        "render_attributes": null,
        "geometry": {"class": "cad:Line", "start": [0.0, 0.0, 0.0], "end": [1.0, 0.0, 0.0]},
    })
}

fn snapshot() -> DocumentSnapshot {
    DocumentSnapshot {
        header: header(),
        revision: Revision(2),
        models: vec![ModelRecord {
            key: ModelKey::ModelSpace,
            kind: ModelKind::ModelSpace,
            name: None,
            owner: None,
        }],
        shards: vec![ShardSnapshot {
            model: ModelKey::ModelSpace,
            shard: Shard(0),
            elements: vec![ElementSnapshot {
                id: EntityId(1),
                payload: element(1),
            }],
        }],
        tables: Default::default(),
        textures: Default::default(),
        resources: Default::default(),
        resource_names: Default::default(),
        resource_sources: Default::default(),
    }
}

fn write_bytes(snapshot: &DocumentSnapshot) -> Vec<u8> {
    let mut sink = MemorySink::new();
    ZipDocumentStore::new()
        .write(snapshot, &ChangeScope::default(), &mut sink)
        .expect("write");
    sink.committed_bytes().expect("committed").to_vec()
}

fn read_bytes(bytes: &[u8]) -> cad_db::LoadedDocument {
    let mut source = MemorySource::new(bytes.to_vec());
    ZipDocumentStore::new().read(&mut source).expect("read")
}

/// `manifest.format.minor` of an archive, read as an untrusted tool would.
fn minor_of(bytes: &[u8]) -> u64 {
    let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    let mut file = archive.by_name(MANIFEST_ENTRY).expect("manifest");
    let mut body = Vec::new();
    file.read_to_end(&mut body).expect("manifest body");
    let manifest: Value = serde_json::from_slice(&body).expect("manifest json");
    manifest["format"]["minor"].as_u64().expect("minor")
}

/// Rewrite `manifest.format.minor`, leaving every entry payload untouched.
///
/// The v2 entry payloads stay `Stored` zstd frames: flipping the tier must not
/// also flip the container's codec, or the fixture would be refused for the wrong
/// reason.
fn with_minor(bytes: &[u8], minor: u64) -> Vec<u8> {
    with_manifest(bytes, |manifest| {
        manifest["format"]["minor"] = Value::from(minor);
    })
}

/// One plain expression row and one that references the first by sequence — the
/// two shapes ADR-0043 D4 defines for a parameter table. These are the exact
/// bytes `cad-db`'s codec emits (the store must not care, but a test that only
/// round-trips an invented shape would not prove a real table survives).
fn parameter_rows() -> Vec<Value> {
    vec![
        serde_json::json!({
            "seq": 1,
            "name": "radius",
            "kind": "length",
            "expr": {"num": 12.5},
        }),
        serde_json::json!({
            "seq": 2,
            "name": "diameter",
            "kind": "length",
            "expr": {"op": "mul", "lhs": {"ref": 1}, "rhs": {"num": 2.0}},
        }),
    ]
}

#[test]
fn parameter_rows_round_trip_through_an_archive() {
    let rows = parameter_rows();
    let mut document = snapshot();
    document.tables.set(TableKind::Parameters, rows.clone());

    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 8, "parameter rows earn ADR-0043 minor 8");

    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_params());
    assert_eq!(
        loaded.snapshot.tables.get(TableKind::Parameters),
        Some(&rows[..]),
        "the store carries the rows opaquely, so a decoded Value is the contract"
    );

    // A resave keeps the tier (the table is still there).
    assert_eq!(minor_of(&write_bytes(&loaded.snapshot)), 8);
}

#[test]
fn a_parameter_free_document_stays_minor_four() {
    let bytes = write_bytes(&snapshot());
    assert_eq!(minor_of(&bytes), 4);
    assert!(!read_bytes(&bytes).snapshot.has_params());
}

#[test]
fn a_document_with_only_element_params_stays_minor_eight() {
    // The store never reads the element `params` block: the snapshot codec owns
    // its shape, and `has_params` in `cad-db` decides the tier.
    let params = serde_json::json!({"radius": {"num": 1.0}});
    let mut document = snapshot();
    document.shards[0].elements[0].payload["params"] = params.clone();

    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 8);

    let loaded = read_bytes(&bytes);
    assert!(
        loaded.snapshot.tables.get(TableKind::Parameters).is_none(),
        "no parameter table was given, so none is written"
    );
    assert_eq!(
        loaded.snapshot.shards[0].elements[0].payload["params"], params,
        "the binding block survives the shard round trip"
    );
    assert_eq!(minor_of(&write_bytes(&loaded.snapshot)), 8);
}

#[test]
fn a_bim_document_without_parameters_still_writes_seven() {
    // A BIM document must not be dragged up to 8 by the new tier.
    let mut document = snapshot();
    document.tables.set(
        TableKind::BimDefinitions,
        vec![serde_json::json!({
            "id": "level:ifc:doc-1:L1",
            "kind": "level",
            "name": "Level 1",
            "elevation": 0.0,
            "source": {"connector": "ifc", "document_id": "doc-1", "id": "L1", "native_id": null},
            "evidence": "declared",
        })],
    );

    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 7, "BIM without parameters stays at 7");
    assert!(!read_bytes(&bytes).snapshot.has_params());
}

#[test]
fn a_minor_seven_archive_without_parameters_opens() {
    // Older builds wrote parameter-free documents at 7; this build must still
    // open them (and not re-tier them on save).
    let plain = write_bytes(&snapshot());
    assert_eq!(minor_of(&plain), 4);

    let seven = with_minor(&plain, 7);
    assert_eq!(minor_of(&seven), 7);
    let loaded = read_bytes(&seven);
    assert!(!loaded.snapshot.has_params());
    assert_eq!(
        minor_of(&write_bytes(&loaded.snapshot)),
        4,
        "a resave stamps what the document needs, not what the manifest claimed"
    );
}

#[test]
fn a_parameter_table_entry_is_modelled() {
    // The store's table decoder only requires a JSON array: row shapes are
    // `cad-db`'s contract, not the container's. A shape the codec would reject
    // must still round-trip as an opaque array instead of failing the zip read.
    assert_eq!(table_entry(TableKind::Parameters), "tables/parameters.json");

    let rows = vec![
        serde_json::json!(42),
        serde_json::json!("not a parameter row"),
        serde_json::json!({"seq": "wrong type"}),
    ];
    let mut document = snapshot();
    document.tables.set(TableKind::Parameters, rows.clone());

    let bytes = write_bytes(&document);
    let mut archive = ZipArchive::new(Cursor::new(bytes.clone())).expect("zip");
    assert!(
        archive.by_name(table_entry(TableKind::Parameters)).is_ok(),
        "the parameter table is a live archive entry"
    );
    let mut manifest_file = archive.by_name(MANIFEST_ENTRY).expect("manifest");
    let mut manifest_body = Vec::new();
    manifest_file
        .read_to_end(&mut manifest_body)
        .expect("manifest body");
    let manifest: Value = serde_json::from_slice(&manifest_body).expect("manifest json");
    assert!(
        manifest["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .any(|record| record["path"] == table_entry(TableKind::Parameters)),
        "the entry is listed in the manifest"
    );

    let loaded = read_bytes(&bytes);
    assert_eq!(
        loaded.snapshot.tables.get(TableKind::Parameters),
        Some(&rows[..])
    );
    assert_eq!(minor_of(&bytes), 8);
}

#[test]
fn a_parameter_table_does_not_pull_resources_into_the_document() {
    // Minor 8 is above the resource tier, and the tier order is parameters >
    // BIM > resources > plain: a parameter document that also packs a resource
    // still reports 8, and the payload survives.
    let payload = vec![0x00u8, 0x01, 0x00, 0x00, 0xff];
    let mut hasher = Sha256::new();
    hasher.update(&payload);
    let key: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()[..32]
        .to_string();
    let key = format!("{key}.png");
    let mut document = snapshot();
    document.tables.set(TableKind::Parameters, parameter_rows());
    document.tables.set(
        TableKind::Resources,
        vec![serde_json::json!({
            "key": key,
            "kind": "image",
            "name": "logo.png",
            "source": null,
        })],
    );
    document
        .resources
        .insert((BundledKind::Image, key.clone()), payload.into());
    document
        .resource_names
        .insert((BundledKind::Image, key), "logo.png".to_string());

    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 8);
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_params());
    assert_eq!(loaded.snapshot.resources.len(), 1);
}

/// One P1 configuration row: a parent-less overlay with an override and a
/// suppression. The store is opaque; this is the exact shape `cad-db` writes.
fn configuration_rows() -> Vec<Value> {
    vec![serde_json::json!({
        "name": "thin",
        "parent": null,
        "overrides": {"1": {"num": 4.0}},
        "suppressed": [2],
    })]
}

#[test]
fn a_configuration_table_writes_minor_nine() {
    let mut document = snapshot();
    document
        .tables
        .set(TableKind::Configurations, configuration_rows());
    let bytes = write_bytes(&document);
    assert_eq!(
        minor_of(&bytes),
        9,
        "the P1 configuration table needs the minor-9 tier"
    );
    let loaded = cad_store_zip::ZipDocumentStore::new()
        .read(&mut MemorySource::new(bytes))
        .expect("reopen");
    assert_eq!(
        loaded.snapshot.tables.get(TableKind::Configurations),
        Some(configuration_rows().as_slice())
    );
    assert!(loaded.snapshot.has_configurations());
}

#[test]
fn a_p1_expression_tag_writes_minor_nine() {
    let mut document = snapshot();
    document.tables.set(
        TableKind::Parameters,
        vec![serde_json::json!({
            "seq": 1,
            "name": "inverted",
            "kind": "length",
            "expr": {"neg": {"num": 2.0}},
        })],
    );
    let bytes = write_bytes(&document);
    assert_eq!(
        minor_of(&bytes),
        9,
        "a P1 expression tag needs the minor-9 tier"
    );
    assert!(cad_store_zip::ZipDocumentStore::new()
        .read(&mut MemorySource::new(bytes))
        .is_ok());
}

#[test]
fn a_feature_table_writes_minor_ten() {
    let mut document = snapshot();
    document.tables.set(
        TableKind::Features,
        vec![serde_json::json!({
            "id": {"kind": "entity", "entityId": EntityId(1).persistent().raw()},
            "order": 0,
            "operation": "extrude",
            "inputs": [],
            "parameters": {},
        })],
    );
    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 10, "P2 feature definitions need minor 10");
    assert!(cad_store_zip::ZipDocumentStore::new()
        .read(&mut MemorySource::new(bytes))
        .is_ok());
}

#[test]
fn an_in_context_relation_table_writes_minor_eleven() {
    let mut document = snapshot();
    document.tables.set(
        TableKind::Relations,
        vec![serde_json::json!({
            "source": {"entityId": EntityId(1).persistent().raw()},
            "target": {"entityId": EntityId(2).persistent().raw()},
            "reference": {"kind": "face", "key": "f1"},
            "state": "in_context",
            "label": null,
        })],
    );
    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 11, "P3 in-context edges need minor 11");
    assert!(cad_store_zip::ZipDocumentStore::new()
        .read(&mut MemorySource::new(bytes))
        .is_ok());
}
