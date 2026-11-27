//! Construction-guide overlay batches.
//!
//! Guides are document content but not entities: they are bounded line segments
//! (the render batch has no infinite-line primitive), split into dash/gap runs
//! here. Unlike the base scene, this overlay is rebuilt on camera/zoom changes,
//! so the dash period can be **screen-space consistent**: `view_scale` is world
//! units per logical pixel, and the dash/gap are 6/4 logical pixels times it.
//! A missing/degenerate scale falls back to the model-relative period used
//! before (extent / 100, at least 5 world units).

use cad_db::LinetypePattern;
use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef};
use cad_representation::subdivide_dashes;
use cad_scene::{RenderBatch, RenderTopology};

use super::*;

/// Guides draw above the axes/grid and below POINT markers and all transient
/// chrome. They are model reference geometry, not a gesture.
pub const GUIDE_DRAW_ORDER: i64 = 840_000;

/// The guide line/point colour.
const GUIDE_COLOR: [f32; 3] = [0.20, 0.45, 0.85];

pub(crate) fn guide_overlay(
    database: &DrawingDatabase,
    view_scale: Option<f64>,
    document: DocumentId,
) -> VisualOverlay {
    let guides = database.guides();
    if guides.is_empty() {
        return VisualOverlay::default();
    }
    let extent = database
        .bounds()
        .map(|(min, max)| {
            let dx = max.x - min.x;
            let dy = max.y - min.y;
            let dz = max.z - min.z;
            (dx.hypot(dy).hypot(dz) * 10.0).max(1000.0)
        })
        .unwrap_or(1000.0);
    let screen = view_scale.filter(|scale| scale.is_finite() && *scale > 0.0);
    let period = match screen {
        Some(scale) => (6.0 + 4.0) * scale,
        None => (extent / 100.0).max(5.0),
    };
    let dash = period * 0.6;
    let gap = period * 0.4;
    let pattern = LinetypePattern::from_elements([dash, -gap]);
    let mut overlay = VisualOverlay::default();
    for (index, guide) in guides.guides().iter().enumerate() {
        let origin = guide.point();
        let mut vertices: Vec<[f32; 3]> = Vec::new();
        match guide {
            cad_domain::guide::Guide::Line(line) => {
                let start = Point3 {
                    x: line.point.x - line.direction.x * extent,
                    y: line.point.y - line.direction.y * extent,
                    z: line.point.z - line.direction.z * extent,
                };
                let end = Point3 {
                    x: line.point.x + line.direction.x * extent,
                    y: line.point.y + line.direction.y * extent,
                    z: line.point.z + line.direction.z * extent,
                };
                let (runs, _fallback) = subdivide_dashes(&[start, end], &pattern, 1.0, 1.0);
                for run in runs {
                    for pair in run.windows(2) {
                        push_relative(&mut vertices, origin, pair[0]);
                        push_relative(&mut vertices, origin, pair[1]);
                    }
                }
            }
            cad_domain::guide::Guide::Point(mark) => {
                let cross = match screen {
                    Some(scale) => 6.0 * scale,
                    None => (extent * 0.005).clamp(1.0, 25.0),
                };
                for axis in 0..3 {
                    let mut a = mark.point;
                    let mut b = mark.point;
                    match axis {
                        0 => {
                            a.x -= cross;
                            b.x += cross;
                        }
                        1 => {
                            a.y -= cross;
                            b.y += cross;
                        }
                        _ => {
                            a.z -= cross;
                            b.z += cross;
                        }
                    }
                    push_relative(&mut vertices, origin, a);
                    push_relative(&mut vertices, origin, b);
                }
            }
        }
        if vertices.is_empty() {
            continue;
        }
        let batch_pattern = match guide {
            cad_domain::guide::Guide::Line(_) => pattern.clone(),
            cad_domain::guide::Guide::Point(_) => LinetypePattern::continuous(),
        };
        overlay.batches.push(RenderBatch {
            local_origin: origin,
            topology: RenderTopology::Lines,
            vertices,
            normals: Vec::new(),
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
            pbr: None,
            indices: Vec::new(),
            edges: Vec::new(),
            conics: Vec::new(),
            mirrored: false,
            alpha: 1.0,
            color: cad_scene::sanitize_color(GUIDE_COLOR),
            color_unresolved: false,
            lineweight: 0.0,
            lineweight_unresolved: false,
            linetype: batch_pattern,
            linetype_unresolved: false,
            sources: vec![SelectionRef {
                document,
                // Guides are not pick targets; 0 is never an allocated entity id.
                entity: EntityId(0),
                instance: InstancePath::default(),
                sub_element: None,
            }],
            draw_order: GUIDE_DRAW_ORDER + index as i64,
            text_glyph: false,
        });
    }
    overlay
}

fn push_relative(vertices: &mut Vec<[f32; 3]>, origin: Point3, point: Point3) {
    vertices.push([
        (point.x - origin.x) as f32,
        (point.y - origin.y) as f32,
        (point.z - origin.z) as f32,
    ]);
}
