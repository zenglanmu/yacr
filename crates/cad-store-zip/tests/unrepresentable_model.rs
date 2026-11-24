//! A save refuses a model key the format cannot read back.
//!
//! `Paper(LayoutId(0))`/`Block(BlockId(0))` encode model id sequence 0, which is
//! reserved for model space: [`cad_domain::ymodel::ModelKey::from_persistent`]
//! rejects it, and the reader would classify the resulting `elements/` entry as
//! an unmodelled blob. The writer therefore refuses the snapshot with
//! `ymodel.unrepresentable_model` instead of producing an archive nothing can
//! open (defense in depth — the allocators never issue 0).

use cad_db::{
    ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, ModelKind, ModelRecord,
    ShardSnapshot,
};
use cad_domain::ymodel::{markers, DocumentHeader, IdCounters, ModelKey, Shard};
use cad_domain::{BlockId, DatabaseId, DocumentIdentity, LayoutId, Revision, UnitContext};
use cad_store_zip::ZipDocumentStore;

fn header() -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(1),
        units: UnitContext::drawing_units(),
        identity: DocumentIdentity::Temporary(1),
        resource_keys: Vec::new(),
        embed_resources: false,
        application_domain: None,
        variables: serde_json::json!({}),
        next: IdCounters::default(),
    }
}

fn snapshot(models: Vec<ModelRecord>, shards: Vec<ShardSnapshot>) -> DocumentSnapshot {
    DocumentSnapshot {
        header: header(),
        revision: Revision(1),
        models,
        shards,
        tables: Default::default(),
        textures: Default::default(),
        resources: Default::default(),
        resource_names: Default::default(),
        resource_sources: Default::default(),
    }
}

fn block_model(id: u128) -> ModelRecord {
    ModelRecord {
        key: ModelKey::Block(BlockId(id)),
        kind: ModelKind::Block,
        name: Some(format!("B{id}")),
        owner: None,
    }
}

fn refused(snapshot: &DocumentSnapshot) -> (cad_db::StoreError, MemorySink) {
    let mut sink = MemorySink::new();
    let error = ZipDocumentStore::new()
        .write(snapshot, &ChangeScope::default(), &mut sink)
        .expect_err("a reserved model sequence must be refused");
    (error, sink)
}

#[test]
fn a_reserved_shard_model_sequence_is_refused_on_write() {
    for model in [ModelKey::Block(BlockId(0)), ModelKey::Paper(LayoutId(0))] {
        let snapshot = snapshot(
            Vec::new(),
            vec![ShardSnapshot {
                model,
                shard: Shard(0),
                elements: Vec::new(),
            }],
        );
        let (error, sink) = refused(&snapshot);
        assert_eq!(
            error.marker,
            markers::UNREPRESENTABLE_MODEL,
            "{model:?}: {error}"
        );
        // The detail names the entry path and the offending key.
        assert!(error.detail.contains("elements/"), "{error}");
        assert!(error.detail.contains(&format!("{model:?}")), "{error}");
        assert!(
            sink.committed_bytes().is_none(),
            "a refused save must not publish a partial archive"
        );
    }
}

#[test]
fn a_reserved_model_record_is_refused_even_without_a_shard() {
    // An empty block has no element shard, but its model row alone would make
    // `tables/models.json` undecodable.
    let key = ModelKey::Block(BlockId(0));
    let snapshot = snapshot(vec![block_model(0)], Vec::new());
    let (error, sink) = refused(&snapshot);
    assert_eq!(error.marker, markers::UNREPRESENTABLE_MODEL, "{error}");
    assert!(error.detail.contains("tables/models.json"), "{error}");
    assert!(error.detail.contains(&format!("{key:?}")), "{error}");
    assert!(sink.committed_bytes().is_none());
}

#[test]
fn a_representable_block_model_still_writes_and_reads_back() {
    let key = ModelKey::Block(BlockId(1));
    let snapshot = snapshot(
        vec![block_model(1)],
        vec![ShardSnapshot {
            model: key,
            shard: Shard(0),
            elements: Vec::new(),
        }],
    );
    let mut sink = MemorySink::new();
    ZipDocumentStore::new()
        .write(&snapshot, &ChangeScope::default(), &mut sink)
        .expect("block 1 is representable");
    let bytes = sink.into_committed_bytes().expect("committed");
    let mut source = MemorySource::new(bytes);
    let loaded = ZipDocumentStore::new()
        .read(&mut source)
        .expect("the archive reads back");
    assert_eq!(loaded.snapshot.models, snapshot.models);
    assert_eq!(loaded.snapshot.shards[0].model, key);
}
