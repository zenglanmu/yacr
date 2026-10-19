//! Committed HATCH fixtures import as first-class hatches (ADR 0018 G.3).
//!
//! These are synthetic/authorized QCAD example drawings; this test files them
//! under the repository's fixture policy and is **not** vendor compatibility
//! evidence.

use std::sync::Arc;

use cad_domain::SemanticGeometry;
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

fn import(path: &str) -> cad_db::DrawingDatabase {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes: Arc<[u8]> = Arc::from(
        std::fs::read(root.join(path))
            .expect("fixture readable")
            .into_boxed_slice(),
    );
    let request = ImportRequest {
        document: cad_domain::DocumentId(1),
        database: cad_domain::DatabaseId(1),
        bytes,
        limits: ImportLimits::default(),
        generation: 1,
    };
    AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("fixture imports")
        .database
}

fn hatch_loop_counts(database: &cad_db::DrawingDatabase) -> Vec<usize> {
    // Block-resident hatches are counted too: the flange's detail views live in
    // a block definition, and the invariant under test is the variant mapping,
    // not the owning space.
    let mut counts: Vec<usize> = database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Hatch(hatch) => Some(hatch.loops().len()),
            // The pre-P6a importer produced a `Compound`; that must not come
            // back.
            SemanticGeometry::Compound(_) => panic!("a hatch imported as a Compound"),
            _ => None,
        })
        .collect();
    counts.sort_unstable();
    counts
}

fn assert_all_hatch_type_keys(database: &cad_db::DrawingDatabase) {
    for entity in database.entities() {
        if matches!(entity.geometry, SemanticGeometry::Hatch(_)) {
            assert_eq!(entity.object.type_key, "AcDbHatch");
        }
    }
}

#[test]
fn flange_hatches_import_as_hatch_with_their_loop_counts() {
    let database = import("fixtures/dxf/qcad-flange/flange.dxf");
    // Five multi-loop SOLID hatches and one user-defined pattern (the `76=0`
    // hatch carrying `78=1`).
    assert_eq!(hatch_loop_counts(&database), vec![1, 5, 6, 7, 7, 8]);
    assert_all_hatch_type_keys(&database);
}

#[test]
fn isometric_grid_hatches_import_as_hatch_with_their_loop_counts() {
    let database = import("fixtures/dxf/qcad-examples/isometric_grid.dxf");
    assert_eq!(hatch_loop_counts(&database), vec![1, 9]);
    assert_all_hatch_type_keys(&database);
}

#[test]
fn the_synthetic_multi_style_fixture_carries_both_non_default_styles() {
    // Every QCAD fixture hatch is style 75=0, so Outer/Ignore are only covered
    // by this synthetic drawing (ADR 0018 G.4/N5).
    let database = import("fixtures/dxf/hatch-styles/multi-style.dxf");
    assert_eq!(hatch_loop_counts(&database), vec![2, 3, 3]);
    assert_all_hatch_type_keys(&database);
    let styles: Vec<cad_domain::HatchStyle> = database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Hatch(hatch) => Some(hatch.style()),
            _ => None,
        })
        .collect();
    assert!(
        styles.contains(&cad_domain::HatchStyle::Outer),
        "{styles:?}"
    );
    assert!(
        styles.contains(&cad_domain::HatchStyle::Ignore),
        "{styles:?}"
    );
}
