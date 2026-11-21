//! `manifest.format.minor` per document (ADR-0040 D12, ADR-0042 D6).
//!
//! The version gate runs before anything is decoded, so a document stamped with
//! a higher minor than it needs is refused by older builds for no reason. These
//! tests pin the three tiers — `4` plain, `5` resources only, `6` BIM — through
//! real archives, and prove a minor-5 archive without BIM still opens.
//!
//! Kept in its own file because `tests/archive.rs` is at the 1000-line gate; it
//! needs only the public store/db API plus `zip` to read the manifest.

use std::io::{Cursor, Read};
use std::sync::Arc;

use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use cad_db::{
    ChangeScope, DocumentSnapshot, DocumentStore, ElementSnapshot, MemorySink, MemorySource,
    ModelKind, ModelRecord, ShardSnapshot, TableKind,
};
use cad_domain::ymodel::{DocumentHeader, IdCounters, ModelKey, PersistentIdOf, Shard};
use cad_domain::{
    BundledKind, DatabaseId, DocumentIdentity, EntityId, Revision, Unit, UnitContext,
};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

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

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bim_block() -> Value {
    serde_json::json!({
        "category": "IfcWall",
        "name": "Basic Wall",
        "source": {"connector": "ifc", "document_id": "doc-1", "id": "G1", "native_id": null},
        "level": null,
        "type": null,
        "material": null,
        "psets": [],
        "evidence": "declared",
    })
}

fn bim_definition() -> Value {
    serde_json::json!({
        "id": "level:ifc:doc-1:L1",
        "kind": "level",
        "name": "Level 1",
        "elevation": 0.0,
        "source": {"connector": "ifc", "document_id": "doc-1", "id": "L1", "native_id": null},
        "evidence": "declared",
    })
}

#[test]
fn a_bim_free_document_stays_minor_four() {
    let bytes = write_bytes(&snapshot());
    assert_eq!(minor_of(&bytes), 4);
    assert!(!read_bytes(&bytes).snapshot.has_bim());
}

#[test]
fn a_resources_only_document_stays_minor_five() {
    let payload = vec![0x00u8, 0x01, 0x00, 0x00, 0xff];
    let key = format!("{}.ttf", &sha256_hex(&payload)[..32]);
    let mut document = snapshot();
    document
        .resources
        .insert((BundledKind::Font, key.clone()), Arc::from(payload));
    document
        .resource_names
        .insert((BundledKind::Font, key.clone()), "arial.ttf".to_string());
    document.tables.set(
        TableKind::Resources,
        vec![serde_json::json!({"key": key, "kind": "font", "name": "arial.ttf", "source": null})],
    );

    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 5, "resources alone are minor 5");
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_bim());
    assert_eq!(loaded.snapshot.resources.len(), 1);
}

#[test]
fn a_bim_document_writes_minor_six_and_reopens() {
    let mut document = snapshot();
    document
        .tables
        .set(TableKind::BimDefinitions, vec![bim_definition()]);
    document.shards[0].elements[0].payload["bim"] = bim_block();

    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 6);

    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_bim());
    assert_eq!(
        loaded.snapshot.shards[0].elements[0].payload["bim"],
        bim_block()
    );
    assert_eq!(
        loaded.snapshot.tables.get(TableKind::BimDefinitions),
        Some(&[bim_definition()][..])
    );
    // A second save keeps the tier (the table and the block are still there).
    assert_eq!(minor_of(&write_bytes(&loaded.snapshot)), 6);
}

/// Rewrite `manifest.format.minor`, leaving every other entry untouched.
fn with_minor(bytes: &[u8], minor: u64) -> Vec<u8> {
    use std::io::Write;

    use zip::write::SimpleFileOptions;

    let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("entry");
        let name = file.name().expect("name").to_string();
        let mut body = Vec::new();
        file.read_to_end(&mut body).expect("body");
        entries.push((name, body));
    }
    let mut patched = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut patched));
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        for (name, body) in &entries {
            writer.start_file(name.as_str(), options).expect("start");
            if name == MANIFEST_ENTRY {
                let mut manifest: Value = serde_json::from_slice(body).expect("json");
                manifest["format"]["minor"] = Value::from(minor);
                serde_json::to_writer(&mut writer, &manifest).expect("manifest");
            } else {
                writer.write_all(body).expect("body");
            }
        }
        writer.finish().expect("finish");
    }
    patched
}

#[test]
fn a_minor_five_archive_without_bim_opens() {
    let mut document = snapshot();
    document.header.embed_resources = true;
    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 5, "the flag alone earns minor 5");
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_bim());

    // The ceiling still applies: one above this build's minor is refused.
    let mut source = MemorySource::new(with_minor(&bytes, 7));
    let error = ZipDocumentStore::new()
        .read(&mut source)
        .expect_err("minor 7 is newer than this build");
    assert_eq!(
        error.marker,
        cad_domain::ymodel::markers::UNSUPPORTED_MINOR,
        "{error}"
    );
}

#[test]
fn a_minor_five_archive_that_holds_bim_is_accepted_and_resaved_at_six() {
    // The ADR-0042 compatibility matrix: a non-conforming minor-5 archive with a
    // `bim` block is accepted and preserved, and the next save stamps minor 6
    // instead of dropping the data.
    let mut document = snapshot();
    document.shards[0].elements[0].payload["bim"] = bim_block();
    let six = write_bytes(&document);
    assert_eq!(minor_of(&six), 6);

    let relabelled = with_minor(&six, 5);
    let loaded = read_bytes(&relabelled);
    assert!(loaded.snapshot.has_bim(), "the block is preserved");
    assert_eq!(minor_of(&write_bytes(&loaded.snapshot)), 6);
}
