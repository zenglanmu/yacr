//! Named-view export: `tables/views.json` rows → DXF `VIEW` table → re-import.
//!
//! The test proves our own writer/reader agreement (internal consistency). The
//! per-row approximation (world-space height vs. per-viewport scale) is reported
//! as `export.view_camera_approximated`, so a silent loss is impossible.

use super::*;
use cad_domain::view::{NamedView, NamedViewTable, ViewCamera, ViewProjection};
use cad_domain::{SpaceId, WorkPlane};

fn view(name: &str) -> NamedView {
    let camera = ViewCamera {
        eye: p(0.0, 0.0, 10.0),
        target: p(1.0, 2.0, 0.0),
        up: p(0.0, 1.0, 0.0),
        projection: ViewProjection::Orthographic { scale: 2.0 },
    };
    NamedView {
        name: name.into(),
        camera,
        is_3d: false,
        saved_2d_camera: camera,
        work_plane: WorkPlane {
            origin: p(0.0, 0.0, 0.0),
            u: p(1.0, 0.0, 0.0),
            v: p(0.0, 1.0, 0.0),
        },
        space: SpaceId::Model,
    }
}

fn db_with_views() -> DrawingDatabase {
    let mut table = NamedViewTable::new();
    table.save(view("Plan A")).unwrap();
    table.save(view("Plan B")).unwrap();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.set_named_views(table).unwrap();
    builder.finish().unwrap()
}

#[test]
fn named_views_write_a_dxf_view_table_and_reimport_with_the_reported_approximation() {
    let db = db_with_views();
    let request = ExportRequest {
        database: &db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
        acis_version: AcisVersion::V7_0,
        dxf_target: DxfTarget::R2018,
    };
    let (bytes, report) = export(&request).expect("export");

    // The parser really reads a VIEW table back (not just our in-memory rows).
    let doc = DxfReader::from_reader(Cursor::new(bytes.clone()))
        .expect("DXF reader")
        .read()
        .expect("our DXF parses");
    assert_eq!(doc.views.len(), 2, "both named views are in the DXF table");
    assert!(report
        .notes
        .iter()
        .any(|note| note.code == "export.view_camera_approximated"));

    let imported = import_bytes(&bytes);
    let table = imported.named_views();
    assert_eq!(table.len(), 2);
    let plan = table.resolve("plan a").expect("case-insensitive import");
    assert_eq!(plan.name, "Plan A");
    assert_eq!(plan.space, SpaceId::Model);
    assert_eq!(plan.camera.target, p(1.0, 2.0, 0.0));
}

#[test]
fn a_degenerate_view_is_reported_as_dropped_not_silently_skipped() {
    let mut bad = view("Bad");
    bad.camera.eye = bad.camera.target;
    let mut table = NamedViewTable::new();
    table.save(bad).unwrap();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.set_named_views(table).unwrap();
    let db = builder.finish().unwrap();

    let request = ExportRequest {
        database: &db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
        acis_version: AcisVersion::V7_0,
        dxf_target: DxfTarget::R2018,
    };
    let (_bytes, report) = export(&request).expect("export");
    assert!(report
        .entries
        .iter()
        .any(|entry| entry.code == "export.view_degenerate_camera"));
    assert_eq!(report.counts.dropped, 1);
}
