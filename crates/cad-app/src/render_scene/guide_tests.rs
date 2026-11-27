//! Guide batches: a persisted guide becomes one solid line segment in the
//! scene, without entering the entity index or picking.

use super::*;
use cad_db::DrawingDatabaseBuilder;
use cad_domain::guide::{GuideLine, GuideTable};
use cad_domain::{DatabaseId, TaskStamp};
use cad_scene::RenderTopology;

#[test]
fn a_persisted_guide_is_a_scene_line_batch() {
    let mut table = GuideTable::new();
    table
        .push_line(
            GuideLine::new(
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .expect("guide"),
        )
        .unwrap();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder.set_guides(table).unwrap();
    let db = builder.finish().unwrap();
    let delta = build_scene(&db, TaskStamp::new(DocumentId(1), 0)).expect("scene");
    let batch = delta
        .added
        .iter()
        .find(|batch| {
            batch.topology == RenderTopology::Lines
                && batch.vertices.len() > 2
                && !batch.linetype.is_continuous()
        })
        .expect("the dashed guide line batch");
    assert!((batch.color[0] - 0.20).abs() < 1e-6);
    assert!((batch.color[1] - 0.45).abs() < 1e-6);
    assert!((batch.color[2] - 0.85).abs() < 1e-6);
    // The line is bounded and split into dash/gap runs; no single full-length
    // segment is emitted and every run is short.
    assert!(batch.vertices.len() >= 4, "{:?}", batch.vertices.len());
    for pair in batch.vertices.chunks(2) {
        let dx = pair[1][0] - pair[0][0];
        let dy = pair[1][1] - pair[0][1];
        let dz = pair[1][2] - pair[0][2];
        let length = (dx * dx + dy * dy + dz * dz).sqrt();
        assert!(length > 0.0 && length <= 10.0, "dash length {length}");
    }
    assert_eq!(batch.linetype.elements.len(), 2);
}

#[test]
fn a_guide_point_is_a_three_axis_cross() {
    let mut table = GuideTable::new();
    table
        .push_point(
            cad_domain::guide::GuidePoint::new(Point3 {
                x: 2.0,
                y: 3.0,
                z: 4.0,
            })
            .expect("point"),
        )
        .unwrap();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder.set_guides(table).unwrap();
    let db = builder.finish().unwrap();
    let delta = build_scene(&db, TaskStamp::new(DocumentId(1), 0)).expect("scene");
    let batch = delta
        .added
        .iter()
        .find(|batch| batch.topology == RenderTopology::Lines && batch.vertices.len() == 6)
        .expect("three crossing segments");
    assert!((batch.color[0] - 0.20).abs() < 1e-6);
}
