//! The block-attribute tier (ADR-0055): a document whose block carries an
//! `ATTDEF` or whose `INSERT` carries values is written at minor 14; an ordinary
//! document keeps its lower minor and a minor-13 archive still opens.
//!
//! Kept in its own file because `tests/archive.rs` is at the 1000-line gate.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{
    ChangeScope, DocumentSnapshot, DocumentStore, ElementSnapshot, MemorySink, MemorySource,
    ModelKind, ModelRecord, ShardSnapshot,
};
use cad_domain::ymodel::{DocumentHeader, IdCounters, ModelKey, PersistentIdOf, Shard};
use cad_domain::{
    BlockId, DatabaseId, DocumentIdentity, EntityId, LayerId, Revision, StyleId, Unit, UnitContext,
};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

use common::{explode, with_manifest};

fn header() -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(6),
        units: UnitContext {
            source: Unit::Millimeter,
            display: Unit::Millimeter,
            display_per_source: Some(1.0),
            decimal_places: 2,
        },
        identity: DocumentIdentity::Temporary(3),
        resource_keys: Vec::new(),
        embed_resources: false,
        application_domain: None,
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

/// One element row whose geometry is `geometry`.
fn element(seq: u128, geometry: Value) -> ElementSnapshot {
    ElementSnapshot {
        id: EntityId(seq),
        payload: serde_json::json!({
            "class": "cad:Element",
            "id": EntityId(seq).persistent().raw(),
            "type_key": match geometry["class"].as_str() {
                Some("cad:AttributeDefinition") => "AcDbAttributeDefinition",
                _ => "AcDbBlockReference",
            },
            "space": {"kind": "model"},
            "layer": 1,
            "source_handle": null,
            "invisible": false,
            "render_attributes": null,
            "geometry": geometry,
        }),
    }
}

fn snapshot(geometry: Option<Value>) -> DocumentSnapshot {
    DocumentSnapshot {
        header: header(),
        revision: Revision(1),
        models: vec![ModelRecord {
            key: ModelKey::ModelSpace,
            kind: ModelKind::ModelSpace,
            name: None,
            owner: None,
        }],
        shards: geometry
            .map(|geometry| {
                vec![ShardSnapshot {
                    model: ModelKey::ModelSpace,
                    shard: Shard(0),
                    elements: vec![element(1, geometry)],
                }]
            })
            .unwrap_or_default(),
        tables: Default::default(),
        textures: Default::default(),
        resources: Default::default(),
        resource_names: Default::default(),
        resource_sources: Default::default(),
    }
}

fn definition_geometry() -> Value {
    serde_json::json!({
        "class": "cad:AttributeDefinition",
        "tag": "PART",
        "prompt": "Part?",
        "default_value": "N/A",
        "flags": {
            "invisible": false,
            "constant": false,
            "verify": false,
            "preset": false,
            "locked_position": false,
            "annotative": false
        },
        "order": 0,
        "text": {
            "position": {"x": 1.0, "y": 2.0, "z": 0.0},
            "height": 2.5,
            "rotation": 0.0,
            "style": StyleId(1).persistent().raw(),
            "width_factor": 1.0,
            "h_align": "left",
            "v_align": "baseline",
            "wrap_width": null,
            "line_spacing": {"factor": 1.0, "style": "at_least"},
            "font": null,
            "oblique": 0.0
        },
        "source_handle": null,
    })
}

fn insert_geometry(with_values: bool) -> Value {
    let mut geometry = serde_json::json!({
        "class": "cad:Insert",
        "block": BlockId(1).persistent().raw(),
        "transform": {"matrix": [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0]
        ]},
    });
    if with_values {
        geometry["attributes"] = serde_json::json!([{
            "tag": "PART",
            "value": "A-100",
            "flags": {
                "invisible": false,
                "constant": false,
                "verify": false,
                "preset": false,
                "locked_position": false,
                "annotative": false
            },
            "text": {
                "position": {"x": 1.0, "y": 2.0, "z": 0.0},
                "height": 2.5,
                "rotation": 0.0,
                "style": StyleId(1).persistent().raw(),
                "width_factor": 1.0,
                "h_align": "left",
                "v_align": "baseline",
                "wrap_width": null,
                "line_spacing": {"factor": 1.0, "style": "at_least"},
                "font": null,
                "oblique": 0.0
            },
            "layer": LayerId(1).persistent().raw(),
            "appearance": {
                "transparency": {"kind": "bylayer"},
                "color": {"kind": "bylayer"},
                "lineweight": {"kind": "bylayer"},
                "linetype": {"kind": "bylayer"},
                "linetype_scale": 1.0
            },
            "source_handle": null
        }]);
    }
    geometry
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

fn minor_of(bytes: &[u8]) -> u64 {
    let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    let mut file = archive.by_name(MANIFEST_ENTRY).expect("manifest");
    let mut body = Vec::new();
    file.read_to_end(&mut body).expect("manifest body");
    let manifest: Value = serde_json::from_slice(&body).expect("manifest json");
    manifest["format"]["minor"].as_u64().expect("minor")
}

fn with_minor(bytes: &[u8], minor: u64) -> Vec<u8> {
    with_manifest(bytes, |manifest| {
        manifest["format"]["read"] = Value::from(2);
        manifest["format"]["minor"] = Value::from(minor);
    })
}

#[test]
fn an_attribute_definition_is_written_at_minor_fourteen_and_reads_back() {
    let document = snapshot(Some(definition_geometry()));
    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 14, "a definition is the minor-14 tier");
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_block_attributes());
    assert_eq!(
        loaded.snapshot.shards[0].elements[0].payload["geometry"],
        definition_geometry(),
        "the definition payload round-trips verbatim"
    );
}

#[test]
fn an_insert_with_values_is_written_at_minor_fourteen() {
    let document = snapshot(Some(insert_geometry(true)));
    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 14);
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_block_attributes());
    assert_eq!(
        loaded.snapshot.shards[0].elements[0].payload["geometry"]["attributes"][0]["tag"],
        Value::String("PART".to_string())
    );
}

#[test]
fn an_insert_without_values_keeps_the_plain_minor() {
    let document = snapshot(Some(insert_geometry(false)));
    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 4, "an empty value list is not a tier");
    assert!(!read_bytes(&bytes).snapshot.has_block_attributes());
}

#[test]
fn a_minor_thirteen_archive_still_opens() {
    // A document written before attributes existed: the insert has no
    // `attributes` key at all, and the archive is stamped 13.
    let document = snapshot(Some(insert_geometry(false)));
    let bytes = with_minor(&write_bytes(&document), 13);
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_block_attributes());
    assert_eq!(minor_of(&bytes), 13);
}

#[test]
fn a_minor_above_the_ceiling_is_refused() {
    let document = snapshot(Some(definition_geometry()));
    let bytes = with_minor(
        &write_bytes(&document),
        u64::from(cad_domain::ymodel::FORMAT_MINOR) + 1,
    );
    let mut source = MemorySource::new(bytes);
    let error = ZipDocumentStore::new()
        .read(&mut source)
        .expect_err("a newer minor is refused");
    assert_eq!(
        error.marker,
        cad_domain::ymodel::markers::UNSUPPORTED_MINOR,
        "{error}"
    );
}

#[test]
fn the_class_registry_spells_the_attribute_classes_and_kinds() {
    let bytes = write_bytes(&snapshot(Some(definition_geometry())));
    let (entries, _) = explode(&bytes);
    let classes_entry = entries
        .into_iter()
        .find(|(name, _)| name == "schema/classes.json")
        .expect("class registry entry");
    let value: Value = serde_json::from_slice(&classes_entry.1).expect("class registry json");
    let classes = value["classes"].as_array().expect("classes array");

    let definition = classes
        .iter()
        .find(|class| class["type_key"] == "cad:AttributeDefinition")
        .expect("cad:AttributeDefinition is registered");
    assert_eq!(
        definition["id"]["ordinal"].as_u64(),
        Some(21),
        "the definition is the 21st geometry class"
    );
    let field = |name: &str| {
        definition["fields"]
            .as_array()
            .expect("fields")
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    assert_eq!(field("flags")["kind"], "attribute_flags");
    assert_eq!(field("text")["kind"], "attribute_text");

    let insert = classes
        .iter()
        .find(|class| class["type_key"] == "cad:Insert")
        .expect("cad:Insert is registered");
    let attributes = insert["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .find(|field| field["name"] == "attributes")
        .expect("cad:Insert declares attributes");
    assert_eq!(attributes["kind"], "attribute_values");
}
