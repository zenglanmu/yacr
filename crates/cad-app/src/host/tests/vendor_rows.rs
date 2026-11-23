//! Host-owned vendor semantics through the snapshot seam (ADR-0030 D7).
//!
//! `tables/vendor.json` rows live on the document slot, like the compat/sources
//! report tables: `snapshot_document` copies them into the snapshot and
//! `install_snapshot` copies them back. These contracts cover that seam only;
//! which rows exist (and orphan filtering) is the vendor/plugin boundary's job.

use super::*;
use cad_domain::ymodel::PersistentIdOf;

fn vendor_row(entity: i64) -> serde_json::Value {
    serde_json::json!({
        "entity": entity,
        "handle": "1A2F",
        "kind": "tch",
        "class": "TCH_OPENING",
        "schema": 1,
        "status": "ok",
        "attributes": [{"name": "DN", "type": "f64", "value": 100.0}],
        "params": {"width": 900, "height": 2100, "thickness": 240},
        "refs": [],
        "diagnostics": [],
    })
}

fn blank() -> HostController {
    HostController::with_blank_document([800.0, 600.0]).expect("a blank document opens")
}

#[test]
fn a_document_without_vendor_rows_does_not_write_the_table() {
    let controller = blank();
    let snapshot = controller.snapshot_document().expect("snapshot");
    assert!(
        snapshot.tables.get(cad_db::TableKind::Vendor).is_none(),
        "an empty vector is not an empty table here: the table is the minor-12 tier"
    );
}

#[test]
fn applying_a_plugin_batch_resolves_handles_and_reopens_as_a_yans_pass() {
    let mut source = HostController::with_demo_document([800.0, 600.0]).expect("demo document");
    let row = serde_json::json!({
        "kind": "tch",
        "class": "TCH_OPENING",
        "schema": 1,
        "status": "ok",
        "source": { "handle": "1" },
        "params": { "width": 900 },
        "refs": [{ "role": "wall", "kind": "entity", "handle": "2" }],
        "diagnostics": [],
    });
    assert_eq!(
        source.apply_vendor_semantics(vec![row]).expect("resolve"),
        1
    );
    let stored = source.vendor_rows();
    assert_eq!(
        stored[0]["entity"],
        serde_json::json!(EntityId(1).persistent().raw()),
        "source handle 1 -> entity 1"
    );
    assert_eq!(
        stored[0]["refs"][0]["entity"],
        serde_json::json!(EntityId(2).persistent().raw())
    );
    assert!(
        source.take_vendor_semantics().is_none(),
        "an import's rows were already applied; no YANS pass is owed"
    );

    let snapshot = source.snapshot_document().expect("snapshot");
    let mut reopened = blank();
    reopened
        .install_snapshot(
            &snapshot,
            crate::PersistenceState::untitled(),
            Some("reopen".into()),
        )
        .expect("install");
    let rows = reopened.take_vendor_semantics().expect("one YANS pass");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["entity"],
        serde_json::json!(EntityId(1).persistent().raw())
    );
    assert!(
        reopened.take_vendor_semantics().is_none(),
        "the semantics pass is taken exactly once"
    );
}

#[test]
fn snapshot_install_round_trips_vendor_rows() {
    let mut source = HostController::with_demo_document([800.0, 600.0]).expect("demo document");
    let entity = source
        .drawing()
        .expect("drawing")
        .entities()
        .next()
        .expect("the demo has an entity")
        .id
        .persistent()
        .raw();
    let rows = vec![vendor_row(entity), vendor_row(entity)];
    {
        let document = source
            .application
            .workspace
            .documents
            .get_mut(&source.document_id)
            .expect("the blank document is open");
        document.persistence.set_vendor_rows(rows.clone());
        assert_eq!(document.persistence.vendor_rows(), &rows[..]);
    }

    let snapshot = source.snapshot_document().expect("snapshot");
    assert_eq!(
        snapshot.tables.get(cad_db::TableKind::Vendor),
        Some(&rows[..]),
        "the document slot's rows are the table's rows"
    );

    let mut target = blank();
    target
        .install_snapshot(
            &snapshot,
            crate::PersistenceState::untitled(),
            Some("round-trip".into()),
        )
        .expect("install");
    let persistence = target.persistence().expect("a document is open");
    assert_eq!(
        persistence.vendor_rows(),
        &rows[..],
        "the archive's vendor rows travel back onto the fresh document slot"
    );
    // The rows are not cad-db content: the hydrating database has exactly the
    // source drawing's entities, not one per row.
    assert_eq!(
        target.drawing().expect("drawing").entity_count(),
        source.drawing().expect("source drawing").entity_count(),
        "vendor rows never become entities"
    );
}
