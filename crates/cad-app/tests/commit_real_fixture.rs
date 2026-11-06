//! Opt-in CPU reproduction of a one-line edit on the complex fixture.
//! No GUI/GPU or pixel claim; timings are diagnostic, not pass thresholds.
#![cfg(not(target_arch = "wasm32"))]

use std::{path::PathBuf, sync::Arc, time::Instant};

use cad_app::render_scene::build_scene;
use cad_app::render_scene::{CadSceneController, OverlayInputs};
use cad_db::{DbEntity, DbObject};
use cad_domain::*;
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

#[test]
#[ignore = "real fixture CPU timing; run explicitly with --ignored --nocapture"]
fn one_line_commit_on_complex_fixture() {
    let path = std::env::var_os("YACR_TEST_DWG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/complex-test.dwg")
        });
    let started = Instant::now();
    let imported = AcadrustImporter::new()
        .import(
            &ImportRequest {
                document: DocumentId(1),
                database: DatabaseId(1),
                bytes: std::fs::read(&path).expect("fixture required").into(),
                limits: ImportLimits::default(),
                generation: 0,
                xref: None,
            },
            &|| false,
        )
        .expect("import");
    eprintln!(
        "commit-fixture import_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let original = Arc::new(imported.database);
    let started = Instant::now();
    let before = build_scene(&original, TaskStamp::new(DocumentId(1), 0)).expect("base scene");
    eprintln!(
        "commit-fixture base_ms={:.3} batches={}",
        started.elapsed().as_secs_f64() * 1000.0,
        before.added.len()
    );
    let started = Instant::now();
    let mut candidate = original.as_ref().clone();
    eprintln!(
        "commit-fixture clone_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let source = candidate.model_space()[0];
    let layer = source.layer;
    let order = candidate
        .entities()
        .map(|e| e.draw_order)
        .max()
        .unwrap()
        .saturating_add(1);
    let id = candidate.allocate_entity_id();
    let revision = candidate.revision();
    let started = Instant::now();
    let mut tx = candidate
        .begin_drawing_transaction("fixture one-line commit", TransactionId(1))
        .unwrap();
    tx.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(id.0),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer,
        space: SpaceId::Model,
        draw_order: order,
        geometry: SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
        },
    })
    .unwrap();
    let changes = tx.commit().expect("commit");
    eprintln!(
        "commit-fixture transaction_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(changes.changes.len(), 1);
    assert_eq!(original.revision(), revision);
    assert!(original.entity(id).is_none());
    assert!(candidate.entity(id).is_some());
    let started = Instant::now();
    let after = build_scene(&candidate, TaskStamp::new(DocumentId(1), 1)).expect("edited scene");
    eprintln!(
        "commit-fixture rebuild_ms={:.3} batches={}",
        started.elapsed().as_secs_f64() * 1000.0,
        after.added.len()
    );
    assert_eq!(after.added.len(), before.added.len() + 1);
}

#[test]
#[ignore = "real fixture CPU append equivalence; run explicitly with --ignored --nocapture"]
fn incremental_line_commit_matches_full_fixture() {
    let path = std::env::var_os("YACR_TEST_DWG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/complex-test.dwg")
        });
    let imported = AcadrustImporter::new()
        .import(
            &ImportRequest {
                document: DocumentId(1),
                database: DatabaseId(1),
                bytes: std::fs::read(path).expect("fixture required").into(),
                limits: ImportLimits::default(),
                generation: 0,
                xref: None,
            },
            &|| false,
        )
        .unwrap();
    let before = Arc::new(imported.database);
    let mut host = cad_app::host::HostController::with_demo_document([800.0, 600.0]).unwrap();
    host.session.active_layer =
        cad_app::layers::visible_model_entities(&before, &cad_app::layers::LayerOverrideSet::new())
            [0]
        .layer;
    host.application
        .workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = before.clone();
    let mut controller = CadSceneController::default();
    let layers = cad_app::layers::LayerOverrideSet::new();
    let overlays = OverlayInputs::default();
    let started = Instant::now();
    controller
        .prepare_shared(
            Some(before.clone()),
            DocumentId(1),
            None,
            &layers,
            cad_representation::SpaceSelection::Model,
        )
        .unwrap();
    eprintln!(
        "append-fixture full_prepare_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let predecessor = controller.ready.as_ref().unwrap().base.clone();
    let started = Instant::now();
    let (trace, receipt) = host
        .execute_traced_with_edit(cad_app::Command {
            schema_version: 1,
            id: cad_app::CommandId::CreateLine,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload: cad_app::CommandPayload::Points(vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 1.0,
                    z: 0.0,
                },
            ]),
        })
        .unwrap();
    trace.result.as_ref().unwrap();
    eprintln!(
        "append-fixture command_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let after = host.drawing().unwrap();
    let receipt = receipt.expect("successful LINE receipt");
    let started = Instant::now();
    controller
        .prepare_shared_with_overlays_and_images_and_edit(
            Some(after.clone()),
            DocumentId(1),
            None,
            &layers,
            cad_representation::SpaceSelection::Model,
            &overlays,
            None,
            Some(&receipt),
        )
        .unwrap();
    let ready = controller.ready.as_ref().unwrap();
    let append = ready.base_append.as_ref().expect("fixture uses append");
    assert!(Arc::ptr_eq(&append.predecessor, &predecessor));
    assert_eq!(ready.base.added.len() - append.keep_batches, 1);
    eprintln!(
        "append-fixture incremental_prepare_ms={:.3} kept={} staged={}",
        started.elapsed().as_secs_f64() * 1000.0,
        append.keep_batches,
        ready.base.added.len() - append.keep_batches
    );
    let started = Instant::now();
    let full = build_scene(&after, ready.base.stamp.clone()).unwrap();
    eprintln!(
        "append-fixture full_reference_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(ready.base.added, full.added);
    assert_eq!(ready.base.images, full.images);
    assert_eq!(ready.base.masks, full.masks);
}
