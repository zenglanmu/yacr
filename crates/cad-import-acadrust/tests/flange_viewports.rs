//! Committed QCAD `flange` fixture: paper-viewport anchoring contract.
//!
//! The sample stores the real 2D view centre in DXF group 12/22 and leaves the
//! 3D `view_target` (group 17/27/37) at the origin. Anchoring on `view_target`
//! collapses every viewport to "model origin at the paper centre", which is the
//! regression this test pins. The fixture is an openly distributed QCAD example
//! (provenance in `fixtures/dxf/qcad-flange/SOURCE.md`), not vendor evidence.

use std::sync::Arc;

use cad_domain::{DatabaseId, DocumentId, Point3};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

fn import() -> cad_db::DrawingDatabase {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes: Arc<[u8]> = Arc::from(
        std::fs::read(root.join("fixtures/dxf/qcad-flange/flange.dxf"))
            .expect("fixture readable")
            .into_boxed_slice(),
    );
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes,
        limits: ImportLimits::default(),
        generation: 1,
        xref: None,
    };
    AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("fixture imports")
        .database
}

fn centre_of(clip: &[Point3]) -> (f64, f64) {
    let n = clip.len() as f64;
    (
        clip.iter().map(|p| p.x).sum::<f64>() / n,
        clip.iter().map(|p| p.y).sum::<f64>() / n,
    )
}

#[test]
fn qcad_flange_viewports_anchor_on_the_saved_2d_view_centre() {
    let database = import();
    // (paper viewport centre, model point that must sit at that centre, handle).
    let expected = [
        (
            (115.0, 140.0),
            (30.068_943_298_968_92, -8.340_850_515_463_671),
            "1E80",
        ),
        (
            (67.5, 57.5),
            (81.626_977_037_036_97, -73.232_060_185_185_2),
            "21AF",
        ),
        ((240.0, 155.0), (82.400_703_435_596_45, 17.6875), "21B0"),
        (
            (181.891_944_843_628_8, 258.174_423_392_866_4),
            (-2.551_198_959_217_217, -10.354_830_091_883_23),
            "38C9",
        ),
    ];
    let mut matched = [false; 4];

    for layout in database.layouts() {
        for viewport in &layout.viewports {
            let (centre_x, centre_y) = centre_of(&viewport.clip);
            let Some(index) = expected.iter().position(|entry| {
                (entry.0 .0 - centre_x).abs() < 1e-6 && (entry.0 .1 - centre_y).abs() < 1e-6
            }) else {
                continue;
            };
            // The stored transform is paper → model: the paper rectangle centre
            // must map back to the file's saved view centre.
            let model = viewport.model_to_paper.apply_point(Point3 {
                x: centre_x,
                y: centre_y,
                z: 0.0,
            });
            let (want_x, want_y) = expected[index].1;
            assert!(
                (model.x - want_x).abs() < 1e-6,
                "viewport {} anchors x at {} instead of {want_x}",
                expected[index].2,
                model.x
            );
            assert!(
                (model.y - want_y).abs() < 1e-6,
                "viewport {} anchors y at {} instead of {want_y}",
                expected[index].2,
                model.y
            );
            matched[index] = true;
        }
    }

    assert!(
        matched.iter().all(|seen| *seen),
        "not every content viewport was imported with its saved centre: {matched:?}"
    );
}
