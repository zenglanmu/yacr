//! Guide overlay batches: camera-scaled dashes and point crosses.
//!
//! Guides live in the transient overlay (not the base scene) so the dash period
//! can follow `OverlayInputs::view_scale`; these tests pin that scaling and the
//! point marker shape without a GPU.

use super::*;
use cad_db::DrawingDatabaseBuilder;
use cad_domain::guide::{GuideLine, GuideTable};
use cad_domain::{DatabaseId, TaskStamp};
use cad_scene::RenderTopology;

use crate::render_scene::overlay::guide_overlay;

fn guide_db(with_point: bool) -> cad_db::DrawingDatabase {
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
            .expect("line guide"),
        )
        .unwrap();
    if with_point {
        table
            .push_point(
                cad_domain::guide::GuidePoint::new(Point3 {
                    x: 2.0,
                    y: 3.0,
                    z: 4.0,
                })
                .expect("point guide"),
            )
            .unwrap();
    }
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder.set_guides(table).unwrap();
    builder.finish().unwrap()
}

fn segment_lengths(batch: &cad_scene::RenderBatch) -> Vec<f32> {
    batch
        .vertices
        .chunks(2)
        .map(|pair| {
            let dx = pair[1][0] - pair[0][0];
            let dy = pair[1][1] - pair[0][1];
            let dz = pair[1][2] - pair[0][2];
            (dx * dx + dy * dy + dz * dz).sqrt()
        })
        .collect()
}

#[test]
fn a_guide_line_dash_follows_the_camera_scale() {
    let db = guide_db(false);
    let near = guide_overlay(&db, Some(0.5), DocumentId(1));
    let line = near
        .batches
        .iter()
        .find(|batch| batch.topology == RenderTopology::Lines)
        .expect("the guide line batch");
    assert!(line.vertices.len() > 2, "the guide is split into dashes");
    assert_eq!(line.linetype.elements.len(), 2);
    for length in segment_lengths(line) {
        assert!(length > 0.0 && length <= 3.0 + 1e-5, "dash {length}");
    }

    // Twice the world-per-pixel scale doubles the dash length (6 px * scale).
    let far = guide_overlay(&db, Some(1.0), DocumentId(1));
    let line = far
        .batches
        .iter()
        .find(|batch| batch.topology == RenderTopology::Lines)
        .expect("the guide line batch");
    let longest = segment_lengths(line).into_iter().fold(0.0, f32::max);
    assert!(longest > 5.9 && longest <= 6.0 + 1e-4, "dash {longest}");
}

#[test]
fn a_guide_point_is_a_screen_scaled_three_axis_cross() {
    let db = guide_db(true);
    let overlay = guide_overlay(&db, Some(0.5), DocumentId(1));
    assert_eq!(overlay.batches.len(), 2, "one line and one point guide");
    let point = overlay
        .batches
        .iter()
        .find(|batch| batch.topology == RenderTopology::Lines && batch.vertices.len() == 6)
        .expect("the point cross");
    assert!(point.linetype.is_continuous(), "the cross stays solid");
    // Arm length is 6 px * 0.5 = 3 world units, so each segment is 6 long.
    for length in segment_lengths(point) {
        assert!((length - 6.0).abs() < 1e-4, "arm {length}");
    }
}

#[test]
fn a_missing_view_scale_falls_back_to_the_model_period() {
    let db = guide_db(false);
    let overlay = guide_overlay(&db, None, DocumentId(1));
    let line = overlay
        .batches
        .iter()
        .find(|batch| batch.topology == RenderTopology::Lines)
        .expect("the guide line batch");
    assert!(line.vertices.len() > 2);
    assert_eq!(line.linetype.elements.len(), 2);
    // The fallback period is at least 5 world units; no zero/NaN dashes.
    for length in segment_lengths(line) {
        assert!(length > 0.0 && length.is_finite(), "dash {length}");
    }
}

#[test]
fn the_base_scene_no_longer_owns_guides() {
    let db = guide_db(false);
    let delta = build_scene(&db, TaskStamp::new(DocumentId(1), 0)).expect("scene");
    assert!(
        delta.added.is_empty(),
        "guides are transient overlay batches, not base geometry"
    );
}
