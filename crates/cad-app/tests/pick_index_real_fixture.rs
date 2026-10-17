//! Real-fixture pick timing harness (ADR-0012 §10 S4 evidence).
//!
//! Not part of the default gates (`#[ignore]`): it needs the committed
//! `fixtures/complex-test.dwg` (or `YACR_TEST_DWG`) and runs a grid of picks,
//! which is slow on the pre-index path.
//!
//! It drives the **production** entry point `cad_app::picking::pick_at_screen`
//! over a pixel grid, timing each call. The first call builds the persisted
//! index; the rest reuse it (hover and click share this path). Run it with
//! `YACR_PICK_EXHAUSTIVE=1` to force the pre-index exhaustive scan — the same
//! harness then measures the baseline, so before/after are apples-to-apples.
//!
//! ```text
//! YACR_PROFILE_PICK=1 cargo test -p cad-app --test pick_index_real_fixture \
//!     --locked -- --ignored --nocapture
//! ```
//!
//! Honest scope: a local/synthetic timing harness on one committed fixture, not
//! a performance conclusion and not a visual/pixel acceptance.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use cad_app::camera::{Camera, Projection};
use cad_app::picking::pick_at_screen;
use cad_domain::{DatabaseId, DocumentId, Point3, TolerancePolicy};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn fixture() -> Option<PathBuf> {
    let path = std::env::var_os("YACR_TEST_DWG")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("fixtures").join("complex-test.dwg"));
    path.is_file().then_some(path)
}

fn grid() -> (usize, usize) {
    let raw = std::env::var("YACR_PICK_HARNESS_GRID").unwrap_or_default();
    let mut parts = raw.split('x');
    let cols = parts.next().and_then(|v| v.parse().ok()).unwrap_or(6);
    let rows = parts.next().and_then(|v| v.parse().ok()).unwrap_or(4);
    (cols.max(1), rows.max(1))
}

#[test]
#[ignore = "needs the real fixture; run with --ignored"]
fn real_fixture_pick_timings() {
    let Some(path) = fixture() else {
        eprintln!("SKIP real_fixture_pick_timings: no fixture (set YACR_TEST_DWG)");
        return;
    };
    let bytes: Arc<[u8]> = std::fs::read(&path).unwrap().into();
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes,
        limits: ImportLimits::default(),
        generation: 0,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("import");
    let db = imported.database;
    let document = DocumentId(1);
    let entity_count = db.model_space().len();
    eprintln!(
        "YACR_PICK_HARNESS fixture={} entities={entity_count} exhaustive={}",
        path.display(),
        std::env::var_os("YACR_PICK_EXHAUSTIVE").is_some()
    );

    let (min, max) = db.bounds().expect("drawing bounds");
    let center = Point3 {
        x: 0.5 * (min.x + max.x),
        y: 0.5 * (min.y + max.y),
        z: 0.0,
    };
    let size = [926.0, 266.0];
    let ex = (max.x - min.x).max(1e-6);
    let ey = (max.y - min.y).max(1e-6);
    let scale = (ex / size[0]).max(ey / size[1]) * 1.05;
    let camera = Camera {
        eye: Point3 {
            x: center.x,
            y: center.y,
            z: center.z + 1000.0,
        },
        target: center,
        up: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        projection: Projection::Orthographic { scale },
    };

    let (cols, rows) = grid();
    let mut cold_ms = None;
    let mut warm_ms: Vec<f64> = Vec::new();
    let mut hits = 0usize;
    for row in 0..rows {
        for col in 0..cols {
            let pixel = [
                (col as f64 + 0.5) / cols as f64 * size[0],
                (row as f64 + 0.5) / rows as f64 * size[1],
            ];
            let started = Instant::now();
            let report = pick_at_screen(
                &db,
                document,
                &camera,
                pixel,
                size,
                &TolerancePolicy::default(),
                cad_app::BackFacePolicy::Cull,
            )
            .expect("pick");
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            if report.hit.is_some() {
                hits += 1;
            }
            if cold_ms.is_none() {
                cold_ms = Some(elapsed);
            } else {
                warm_ms.push(elapsed);
            }
        }
    }
    warm_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = warm_ms.get(warm_ms.len() / 2).copied().unwrap_or(f64::NAN);
    let max = warm_ms.last().copied().unwrap_or(f64::NAN);
    let cold = cold_ms.unwrap_or(f64::NAN);
    eprintln!(
        "YACR_PICK_HARNESS picks={} hits={hits} cold_ms={cold:.3} warm_median_ms={median:.3} warm_max_ms={max:.3}",
        cols * rows,
    );
}
