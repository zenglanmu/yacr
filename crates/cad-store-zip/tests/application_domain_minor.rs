//! The `application_domain` tier (ADR-0053 D2): a document that names a domain is
//! written at minor 13; a document that names none keeps the older bytes and its
//! lower minor.

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{
    ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, ModelKind, ModelRecord,
};
use cad_domain::ymodel::{DocumentHeader, IdCounters, ModelKey};
use cad_domain::{ApplicationDomain, DatabaseId, DocumentIdentity, Revision, Unit, UnitContext};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

fn header() -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(9),
        units: UnitContext {
            source: Unit::Millimeter,
            display: Unit::Millimeter,
            display_per_source: Some(1.0),
            decimal_places: 2,
        },
        identity: DocumentIdentity::Temporary(2),
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

fn snapshot() -> DocumentSnapshot {
    DocumentSnapshot {
        header: header(),
        revision: Revision(1),
        models: vec![ModelRecord {
            key: ModelKey::ModelSpace,
            kind: ModelKind::ModelSpace,
            name: None,
            owner: None,
        }],
        shards: Vec::new(),
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

fn minor_of(bytes: &[u8]) -> u64 {
    let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    let mut file = archive.by_name(MANIFEST_ENTRY).expect("manifest");
    let mut body = Vec::new();
    file.read_to_end(&mut body).expect("manifest body");
    let manifest: Value = serde_json::from_slice(&body).expect("manifest json");
    manifest["format"]["minor"].as_u64().expect("minor")
}

#[test]
fn a_named_domain_is_written_at_minor_thirteen_and_reads_back() {
    let mut document = snapshot();
    document.header.application_domain = Some(ApplicationDomain::Bim);
    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 13, "the domain scalar is the newest tier");
    let loaded = read_bytes(&bytes);
    assert_eq!(
        loaded.snapshot.header.application_domain,
        Some(ApplicationDomain::Bim),
        "the token round-trips verbatim"
    );
}

#[test]
fn an_unnamed_document_keeps_its_lower_minor() {
    let bytes = write_bytes(&snapshot());
    assert_eq!(minor_of(&bytes), 4);
    assert_eq!(read_bytes(&bytes).snapshot.header.application_domain, None);
}

#[test]
fn the_domain_tier_wins_over_a_lower_feature_tier() {
    let mut document = snapshot();
    document.header.application_domain = Some(ApplicationDomain::Mechanical);
    // A resources-only document would otherwise be minor 5; the domain scalar is
    // newer, so the archive must be 13.
    document.header.embed_resources = true;
    let bytes = write_bytes(&document);
    assert_eq!(minor_of(&bytes), 13);
}
