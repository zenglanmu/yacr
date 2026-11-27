//! CPU section-plane clipping for scene batches.
//!
//! A [`ClipPlane`] keeps the half-space `dot(normal, p - origin) <= 0` (the
//! "below" side) and removes geometry above it. The pass is deliberately at the
//! batch boundary so every topology the scene can emit is handled once:
//!
//! * triangle meshes are clipped per triangle and re-triangulated as a fan;
//!   positions, normals, gradient colours and UVs are linearly interpolated;
//! * line lists, mesh-edge streams and arc segments are clipped per segment;
//! * SDF conics are tessellated into lines **only while a section is active**
//!   (the renderer cannot clip an analytic arc), then clipped like any segment;
//! * the cut boundary is appended to a mesh's `edges` stream, so the section
//!   reads as a cut even though the face is not capped.
//!
//! Images, wipeout masks and the cut-face fill are not part of this pass; those
//! are explicit boundaries, not silent approximations.

use cad_domain::Point3;

use super::{RenderBatch, RenderTopology, SceneDelta};

/// A half-space clip plane in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPlane {
    /// A point on the plane.
    pub origin: Point3,
    /// The unit normal; the positive side is removed.
    pub normal: Point3,
}

impl ClipPlane {
    /// Build a plane from an origin and a non-zero normal.
    pub fn new(origin: Point3, normal: Point3) -> Option<Self> {
        let length = (normal.x * normal.x + normal.y * normal.y + normal.z * normal.z).sqrt();
        if !length.is_finite()
            || length < 1.0e-9
            || !origin.x.is_finite()
            || !origin.y.is_finite()
            || !origin.z.is_finite()
        {
            return None;
        }
        Some(ClipPlane {
            origin,
            normal: Point3 {
                x: normal.x / length,
                y: normal.y / length,
                z: normal.z / length,
            },
        })
    }

    /// Signed distance of a world point; positive is removed.
    fn distance(&self, point: [f64; 3]) -> f64 {
        (point[0] - self.origin.x) * self.normal.x
            + (point[1] - self.origin.y) * self.normal.y
            + (point[2] - self.origin.z) * self.normal.z
    }
}

/// Clip every batch in `delta` against `plane`, dropping fully removed batches.
///
/// Images and masks are left untouched (documented boundary).
pub fn clip_delta(delta: &mut SceneDelta, plane: &ClipPlane) {
    delta.added.retain_mut(|batch| {
        clip_batch(batch, plane);
        batch_has_geometry(batch)
    });
}

fn batch_has_geometry(batch: &RenderBatch) -> bool {
    !batch.vertices.is_empty() || !batch.conics.is_empty()
}

fn clip_batch(batch: &mut RenderBatch, plane: &ClipPlane) {
    match batch.topology {
        RenderTopology::Mesh => clip_mesh(batch, plane),
        RenderTopology::Lines | RenderTopology::MeshEdges => {
            let (vertices, colors) =
                clip_line_stream(&batch.vertices, &batch.colors, batch.local_origin, plane);
            batch.vertices = vertices;
            if !batch.colors.is_empty() {
                batch.colors = colors;
            }
        }
        RenderTopology::Conics => clip_conics(batch, plane),
    }
}

#[derive(Clone, Copy)]
struct Vertex {
    position: [f64; 3],
    normal: [f32; 3],
    color: Option<[f32; 3]>,
    uv: Option<[f32; 2]>,
    on_plane: bool,
}

impl Vertex {
    fn local(self, origin: Point3) -> [f32; 3] {
        [
            (self.position[0] - origin.x) as f32,
            (self.position[1] - origin.y) as f32,
            (self.position[2] - origin.z) as f32,
        ]
    }

    fn lerp(a: Vertex, b: Vertex, t: f64) -> Vertex {
        let mix = |x: f64, y: f64| x + (y - x) * t;
        let mix_f32 = |x: [f32; 3], y: [f32; 3]| {
            [
                (f64::from(x[0]) + (f64::from(y[0]) - f64::from(x[0])) * t) as f32,
                (f64::from(x[1]) + (f64::from(y[1]) - f64::from(x[1])) * t) as f32,
                (f64::from(x[2]) + (f64::from(y[2]) - f64::from(x[2])) * t) as f32,
            ]
        };
        Vertex {
            position: [
                mix(a.position[0], b.position[0]),
                mix(a.position[1], b.position[1]),
                mix(a.position[2], b.position[2]),
            ],
            normal: mix_f32(a.normal, b.normal),
            color: match (a.color, b.color) {
                (Some(x), Some(y)) => Some(mix_f32(x, y)),
                _ => None,
            },
            uv: match (a.uv, b.uv) {
                (Some(x), Some(y)) => Some([
                    (f64::from(x[0]) + (f64::from(y[0]) - f64::from(x[0])) * t) as f32,
                    (f64::from(x[1]) + (f64::from(y[1]) - f64::from(x[1])) * t) as f32,
                ]),
                _ => None,
            },
            on_plane: true,
        }
    }
}

fn clip_mesh(batch: &mut RenderBatch, plane: &ClipPlane) {
    let origin = batch.local_origin;
    let vertex = |index: usize| -> Option<Vertex> {
        let position = batch.vertices.get(index)?;
        let normal = batch.normals.get(index).copied().unwrap_or([0.0; 3]);
        Some(Vertex {
            position: [
                f64::from(position[0]) + origin.x,
                f64::from(position[1]) + origin.y,
                f64::from(position[2]) + origin.z,
            ],
            normal,
            color: batch.colors.get(index).copied(),
            uv: batch.uvs.get(index).copied(),
            on_plane: false,
        })
    };
    let triangles: Vec<[usize; 3]> = if batch.indices.is_empty() {
        (0..batch.vertices.len().saturating_sub(2))
            .step_by(3)
            .map(|i| [i, i + 1, i + 2])
            .collect()
    } else {
        batch
            .indices
            .iter()
            .map(|tri| tri.map(|i| i as usize))
            .collect()
    };
    let has_colors = !batch.colors.is_empty();
    let has_uvs = !batch.uvs.is_empty();
    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let mut cut_edges = Vec::new();
    for tri in triangles {
        let Some(mut polygon) = tri
            .iter()
            .map(|index| vertex(*index))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        clip_polygon(&mut polygon, plane);
        if polygon.len() < 3 {
            continue;
        }
        for pair in polygon.windows(2) {
            if pair[0].on_plane && pair[1].on_plane {
                push_edge(&mut cut_edges, pair[0], pair[1], origin);
            }
        }
        if polygon[0].on_plane && polygon[polygon.len() - 1].on_plane {
            push_edge(
                &mut cut_edges,
                polygon[polygon.len() - 1],
                polygon[0],
                origin,
            );
        }
        let base = vertices.len() as u32;
        for point in &polygon {
            vertices.push(point.local(origin));
            normals.push(point.normal);
            if has_colors {
                colors.push(point.color.unwrap_or([1.0; 3]));
            }
            if has_uvs {
                uvs.push(point.uv.unwrap_or([0.0; 2]));
            }
        }
        for fan in 1..polygon.len() - 1 {
            indices.push([base, base + fan as u32, base + fan as u32 + 1]);
        }
    }
    let mut edges = clip_line_positions(&batch.edges, origin, plane);
    edges.extend(cut_edges);
    batch.vertices = vertices;
    batch.normals = normals;
    batch.colors = colors;
    batch.uvs = uvs;
    batch.indices = indices;
    batch.edges = edges;
}

/// Sutherland–Hodgman against one half-space, marking interpolated vertices.
fn clip_polygon(polygon: &mut Vec<Vertex>, plane: &ClipPlane) {
    let mut output: Vec<Vertex> = Vec::with_capacity(polygon.len() + 1);
    for index in 0..polygon.len() {
        let current = polygon[index];
        let previous = polygon[(index + polygon.len() - 1) % polygon.len()];
        let current_distance = plane.distance(current.position);
        let previous_distance = plane.distance(previous.position);
        let current_inside = current_distance <= 0.0;
        let previous_inside = previous_distance <= 0.0;
        if current_inside != previous_inside {
            let denominator = previous_distance - current_distance;
            let t = if denominator.abs() > 1.0e-12 {
                previous_distance / denominator
            } else {
                0.0
            };
            output.push(Vertex::lerp(previous, current, t));
        }
        if current_inside {
            let mut kept = current;
            kept.on_plane = current_distance.abs() <= 1.0e-9;
            output.push(kept);
        }
    }
    *polygon = output;
}

fn clip_line_stream(
    positions: &[[f32; 3]],
    colors: &[[f32; 3]],
    origin: Point3,
    plane: &ClipPlane,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let mut out = Vec::with_capacity(positions.len());
    let mut out_colors = Vec::with_capacity(colors.len());
    for pair_index in 0..positions.len() / 2 {
        let pair = [positions[pair_index * 2], positions[pair_index * 2 + 1]];
        let color_index = pair_index * 2;
        let color = |index: usize| colors.get(index).copied();
        let start = world(pair[0], origin);
        let end = world(pair[1], origin);
        let ds = plane.distance(start);
        let de = plane.distance(end);
        let start_inside = ds <= 0.0;
        let end_inside = de <= 0.0;
        match (start_inside, end_inside) {
            (false, false) => {}
            (true, true) => {
                out.push(pair[0]);
                out.push(pair[1]);
                if !colors.is_empty() {
                    out_colors.push(color(color_index).unwrap_or([1.0; 3]));
                    out_colors.push(color(color_index + 1).unwrap_or([1.0; 3]));
                }
            }
            _ => {
                let denominator = ds - de;
                let t = if denominator.abs() > 1.0e-12 {
                    ds / denominator
                } else {
                    0.0
                };
                let intersection = lerp_position(start, end, t);
                let inside = if start_inside { pair[0] } else { pair[1] };
                let outside_side = if start_inside { pair[1] } else { pair[0] };
                let cut = local(intersection, origin);
                let (a, b) = if start_inside {
                    (inside, cut)
                } else {
                    (cut, inside)
                };
                out.push(a);
                out.push(b);
                if !colors.is_empty() {
                    let (ca, cb) =
                        color_pair(color(color_index), color(color_index + 1), start_inside, t);
                    out_colors.push(ca);
                    out_colors.push(cb);
                }
                let _ = outside_side;
            }
        }
    }
    (out, out_colors)
}

fn color_pair(
    start: Option<[f32; 3]>,
    end: Option<[f32; 3]>,
    start_inside: bool,
    t: f64,
) -> ([f32; 3], [f32; 3]) {
    let start = start.unwrap_or([1.0; 3]);
    let end = end.unwrap_or([1.0; 3]);
    let mix = |a: [f32; 3], b: [f32; 3], t: f64| {
        [
            (f64::from(a[0]) + (f64::from(b[0]) - f64::from(a[0])) * t) as f32,
            (f64::from(a[1]) + (f64::from(b[1]) - f64::from(a[1])) * t) as f32,
            (f64::from(a[2]) + (f64::from(b[2]) - f64::from(a[2])) * t) as f32,
        ]
    };
    let cut = mix(start, end, t);
    if start_inside {
        (start, cut)
    } else {
        (cut, end)
    }
}

fn clip_line_positions(positions: &[[f32; 3]], origin: Point3, plane: &ClipPlane) -> Vec<[f32; 3]> {
    clip_line_stream(positions, &[], origin, plane).0
}

fn clip_conics(batch: &mut RenderBatch, plane: &ClipPlane) {
    let mut positions = Vec::new();
    for conic in &batch.conics {
        let span = if conic.full {
            std::f64::consts::TAU
        } else {
            conic.sweep
        };
        let segments = ((span.abs() / std::f64::consts::TAU) * 64.0)
            .ceil()
            .max(2.0) as usize;
        let mut previous = None;
        for step in 0..=segments {
            let t = conic.start + span * (step as f64 / segments as f64);
            let point = [
                conic.center.x + conic.major.x * t.cos() + conic.minor.x * t.sin(),
                conic.center.y + conic.major.y * t.cos() + conic.minor.y * t.sin(),
                conic.center.z + conic.major.z * t.cos() + conic.minor.z * t.sin(),
            ];
            if let Some(previous) = previous {
                let start = local(previous, batch.local_origin);
                let end = local(point, batch.local_origin);
                positions.push(start);
                positions.push(end);
            }
            previous = Some(point);
        }
    }
    let clipped = clip_line_positions(&positions, batch.local_origin, plane);
    batch.topology = RenderTopology::Lines;
    batch.vertices = clipped;
    batch.normals.clear();
    batch.colors.clear();
    batch.uvs.clear();
    batch.indices.clear();
    batch.edges.clear();
    batch.conics.clear();
}

fn world(point: [f32; 3], origin: Point3) -> [f64; 3] {
    [
        f64::from(point[0]) + origin.x,
        f64::from(point[1]) + origin.y,
        f64::from(point[2]) + origin.z,
    ]
}

fn local(point: [f64; 3], origin: Point3) -> [f32; 3] {
    [
        (point[0] - origin.x) as f32,
        (point[1] - origin.y) as f32,
        (point[2] - origin.z) as f32,
    ]
}

fn lerp_position(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn push_edge(out: &mut Vec<[f32; 3]>, a: Vertex, b: Vertex, origin: Point3) {
    out.push(a.local(origin));
    out.push(b.local(origin));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LinetypePattern;

    fn origin() -> Point3 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }

    fn batch(topology: RenderTopology, vertices: Vec<[f32; 3]>) -> RenderBatch {
        let normals = if topology == RenderTopology::Mesh {
            vec![[0.0, 0.0, 1.0]; vertices.len()]
        } else {
            Vec::new()
        };
        let indices = if topology == RenderTopology::Mesh {
            (0..vertices.len() / 3)
                .map(|i| [(i * 3) as u32, (i * 3 + 1) as u32, (i * 3 + 2) as u32])
                .collect()
        } else {
            Vec::new()
        };
        RenderBatch {
            local_origin: origin(),
            topology,
            vertices,
            normals,
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
            pbr: None,
            indices,
            edges: Vec::new(),
            conics: Vec::new(),
            mirrored: false,
            alpha: 1.0,
            color: [1.0, 1.0, 1.0],
            color_unresolved: false,
            lineweight: 0.0,
            lineweight_unresolved: false,
            linetype: LinetypePattern::continuous(),
            linetype_unresolved: false,
            sources: Vec::new(),
            draw_order: 0,
            text_glyph: false,
        }
    }

    fn z_plane(z: f64) -> ClipPlane {
        ClipPlane::new(
            Point3 { x: 0.0, y: 0.0, z },
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        )
        .expect("unit normal")
    }

    #[test]
    fn a_triangle_crossing_the_plane_is_cut_and_its_boundary_recorded() {
        let mut batch = batch(
            RenderTopology::Mesh,
            vec![[0.0, 0.0, -1.0], [1.0, 0.0, -1.0], [0.0, 1.0, 1.0]],
        );
        clip_batch(&mut batch, &z_plane(0.0));
        assert_eq!(
            batch.indices.len(),
            2,
            "the cut quad fans into two triangles"
        );
        assert_eq!(batch.vertices.len(), 4);
        assert_eq!(batch.edges.len(), 2, "the cut segment is drawn");
        assert!(batch.vertices.iter().all(|point| point[2] <= 1.0e-6));
    }

    #[test]
    fn a_segment_crossing_the_plane_keeps_only_the_below_part() {
        let mut batch = batch(
            RenderTopology::Lines,
            vec![[0.0, 0.0, -2.0], [0.0, 0.0, 2.0]],
        );
        clip_batch(&mut batch, &z_plane(0.0));
        assert_eq!(batch.vertices.len(), 2);
        assert!((batch.vertices[1][2]).abs() < 1.0e-6);
    }

    #[test]
    fn a_fully_removed_batch_is_dropped() {
        let mut delta = SceneDelta {
            stamp: cad_domain::TaskStamp::new(cad_domain::DocumentId(1), 0),
            added: vec![batch(
                RenderTopology::Lines,
                vec![[0.0, 0.0, 1.0], [1.0, 0.0, 2.0]],
            )],
            images: Vec::new(),
            masks: Vec::new(),
        };
        clip_delta(&mut delta, &z_plane(0.0));
        assert!(delta.added.is_empty());
    }

    #[test]
    fn a_conic_is_tessellated_only_when_a_section_is_active() {
        let mut batch = batch(RenderTopology::Conics, Vec::new());
        batch.conics.push(crate::RenderConic {
            center: origin(),
            major: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            minor: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            start: 0.0,
            sweep: std::f64::consts::TAU,
            full: true,
        });
        clip_batch(&mut batch, &z_plane(0.5));
        assert_eq!(batch.topology, RenderTopology::Lines);
        assert!(batch.conics.is_empty());
        assert!(!batch.vertices.is_empty());
        assert!(batch.vertices.iter().all(|point| point[2] <= 1.0e-6));
    }
}
