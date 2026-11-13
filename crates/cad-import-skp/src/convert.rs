//! `skpwright::Model` -> world-space neutral meshes.
//!
//! `skpwright` hands back one materialized [`skpwright::Mesh`] per geometry run
//! (vertices already in **metres**, faces as ordered vertex rings with holes)
//! plus a composed scene of [`skpwright::Node`]s carrying row-major 4x4 world
//! transforms. This module turns that into the adapter's unit of insertion: one
//! [`PlacedMesh`] per (placement, effective material) group, with flat
//! per-face normals and every face triangulated in its own plane.
//!
//! Two decisions are deliberate:
//!
//! * **Faces are triangulated per face, with their own vertices.** SketchUp
//!   faces are flat and independently painted, so sharing vertices between
//!   faces would force one normal and one colour onto different planes. The
//!   extra vertices are the price of flat shading and exact per-material
//!   grouping.
//! * **Holes use the workspace's own even-odd filler**
//!   ([`cad_domain::hatch::fill_rings`]). It validates its output area against
//!   the ring nesting, so a face whose rings are degenerate is **skipped and
//!   reported**, never filled with an invented approximation.

use std::collections::BTreeMap;

use cad_domain::hatch::{fill_rings, triangulate};
use cad_domain::{CadError, Mesh, Point3};
use cad_import_core::ImportLimits;

/// Maximum instance nesting depth walked. A corrupt or hostile file must not
/// turn the scene walk into unbounded recursion.
pub const MAX_DEPTH: usize = 64;

/// Why one face produced no triangles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceSkip {
    /// Fewer than three ring vertices, a zero-length face normal, or an index
    /// that does not name a vertex of the run.
    Malformed,
    /// The rings are degenerate, self-intersecting, or over the filler's point
    /// budget. The face is reported, not approximated.
    Untriangulable,
}

/// One insertable entity: a material group of one placed run, in world metres.
#[derive(Debug, Clone)]
pub struct PlacedMesh {
    pub mesh: Mesh,
    /// `None` = no material anywhere on this group (`ByLayer`).
    pub color: Option<[u8; 3]>,
    /// `Some(opacity)` only when the material is genuinely translucent.
    pub opacity: Option<f32>,
    /// Effective material slot this group was built from (`0` = none).
    pub material: u16,
    /// Triangles contributed (equal to `mesh.triangles.len()`).
    pub triangles: usize,
}

/// What the scene walk saw. Every field is either reported as a diagnostic or
/// used to derive one; nothing here is a silent drop.
#[derive(Debug, Default, Clone)]
pub struct SkpStats {
    /// Geometry runs in the model (definitions + the root run).
    pub runs: usize,
    /// Runs actually placed in the visible scene.
    pub placed_runs: usize,
    /// Scene nodes visited.
    pub nodes: usize,
    /// Instances pruned because the instance itself is hidden.
    pub hidden_instances_pruned: usize,
    /// Instances pruned because their tag (layer) is invisible.
    pub layer_instances_pruned: usize,
    /// Faces skipped because their own tag (layer) is invisible.
    pub invisible_layer_faces_skipped: usize,
    /// Faces skipped because the face itself is hidden.
    pub hidden_faces_skipped: usize,
    /// Faces that produced triangles.
    pub faces_emitted: usize,
    /// Faces dropped because their rings could not be triangulated.
    pub faces_untriangulable: usize,
    /// Faces dropped because their ring indices did not resolve.
    pub faces_malformed: usize,
    /// Placement nests deeper than [`MAX_DEPTH`].
    pub depth_exceeded: usize,
    /// Edges in the placed runs (SketchUp line/curve geometry). Recorded, not
    /// imported: this adapter publishes triangles only.
    pub edges_not_imported: usize,
    /// Materials in the model.
    pub materials: usize,
    /// Tags (layers) in the model.
    pub layers: usize,
    /// Materials that carry an embedded texture, whose image is not sampled.
    pub textured_materials: usize,
}

/// The plottable result of a scene walk.
#[derive(Debug, Default)]
pub struct SkpScene {
    pub placed: Vec<PlacedMesh>,
    pub stats: SkpStats,
}

impl SkpScene {
    /// Entities this scene will insert.
    pub fn entity_count(&self) -> usize {
        self.placed.len()
    }
}

/// Walk `model`'s scene and materialize every visible placement.
///
/// Visibility is WYSIWYG, not archival: hidden instances, hidden faces and
/// invisible tags are pruned (and counted). Layer *names* are flattened onto
/// layer `0`, which the caller reports.
pub fn read_scene(
    model: &skpwright::Model,
    limits: &ImportLimits,
    cancelled: &dyn Fn() -> bool,
) -> cad_domain::CadResult<SkpScene> {
    let mut stats = SkpStats {
        runs: model.geometry.len(),
        materials: model.materials.len(),
        layers: model.layers.len(),
        textured_materials: model
            .materials
            .iter()
            .filter(|m| matches!(m, skpwright::Material::Textured { .. }))
            .count(),
        ..SkpStats::default()
    };
    let mut placed = Vec::new();
    let layer_visible = |slot: u16| model.layer_of(slot).is_none_or(|layer| layer.visible);

    // The model-level run (loose geometry no instance places) sits at identity.
    for index in 0..model.geometry.len() {
        if model.geometry[index].def_index.is_some() {
            continue;
        }
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        place_run(
            model,
            index,
            &IDENTITY,
            0,
            &layer_visible,
            limits,
            cancelled,
            &mut placed,
            &mut stats,
        )?;
    }

    for node in model.scene() {
        walk(
            model,
            &node,
            0,
            &layer_visible,
            limits,
            cancelled,
            &mut placed,
            &mut stats,
        )?;
    }

    Ok(SkpScene { placed, stats })
}

const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

#[allow(clippy::too_many_arguments)]
fn walk(
    model: &skpwright::Model,
    node: &skpwright::Node,
    depth: usize,
    layer_visible: &dyn Fn(u16) -> bool,
    limits: &ImportLimits,
    cancelled: &dyn Fn() -> bool,
    placed: &mut Vec<PlacedMesh>,
    stats: &mut SkpStats,
) -> cad_domain::CadResult<()> {
    if depth > MAX_DEPTH {
        stats.depth_exceeded += 1;
        return Ok(());
    }
    stats.nodes += 1;
    if node.hidden {
        stats.hidden_instances_pruned += 1;
        return Ok(());
    }
    if !layer_visible(node.layer) {
        stats.layer_instances_pruned += 1;
        return Ok(());
    }
    if let Some(index) = node.run {
        place_run(
            model,
            index,
            &node.world,
            node.material,
            layer_visible,
            limits,
            cancelled,
            placed,
            stats,
        )?;
    }
    for child in &node.children {
        walk(
            model,
            child,
            depth + 1,
            layer_visible,
            limits,
            cancelled,
            placed,
            stats,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn place_run(
    model: &skpwright::Model,
    run_index: usize,
    world: &[f64; 16],
    inherited_material: u16,
    layer_visible: &dyn Fn(u16) -> bool,
    limits: &ImportLimits,
    cancelled: &dyn Fn() -> bool,
    placed: &mut Vec<PlacedMesh>,
    stats: &mut SkpStats,
) -> cad_domain::CadResult<()> {
    let Some(run) = model.geometry.get(run_index) else {
        return Ok(());
    };
    stats.placed_runs += 1;
    let groups = group_faces(&run.mesh, inherited_material, layer_visible, stats);

    for (material, faces) in groups {
        let (color, opacity) = material_style(model, material);
        let mut out = GroupBuffers::default();
        for face in faces {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            match emit_face(&run.mesh, face, world, &mut out) {
                Ok(()) => stats.faces_emitted += 1,
                Err(FaceSkip::Malformed) => stats.faces_malformed += 1,
                Err(FaceSkip::Untriangulable) => stats.faces_untriangulable += 1,
            }
            check_face_budget(stats, limits)?;
        }
        if out.triangles.is_empty() {
            continue;
        }
        placed.push(PlacedMesh {
            triangles: out.triangles.len(),
            mesh: Mesh {
                vertices: out.vertices,
                triangles: out.triangles,
                normals: out.normals,
                face_sources: Vec::new(),
                colors: Vec::new(),
            },
            color,
            opacity,
            material,
        });
    }

    stats.edges_not_imported += run.mesh.edges.len();
    Ok(())
}

fn check_face_budget(stats: &SkpStats, limits: &ImportLimits) -> cad_domain::CadResult<()> {
    if let Some(max) = limits.max_mesh_faces {
        if stats.faces_emitted > max {
            return Err(CadError::InvalidInput(format!(
                "import.skp.limits_exceeded: more than {max} faces"
            )));
        }
    }
    if let Some(max) = limits.max_mesh_face_indices {
        if stats.faces_emitted.saturating_mul(3) > max {
            return Err(CadError::InvalidInput(format!(
                "import.skp.limits_exceeded: more than {max} face indices"
            )));
        }
    }
    Ok(())
}

/// Group a run's visible faces by effective material slot (`0` = none).
///
/// An instance's inherited material paints only faces that carry no material
/// of their own — the SketchUp §4q composition rule.
fn group_faces<'a>(
    mesh: &'a skpwright::Mesh,
    inherited_material: u16,
    layer_visible: &dyn Fn(u16) -> bool,
    stats: &mut SkpStats,
) -> BTreeMap<u16, Vec<&'a skpwright::MeshFace>> {
    let mut groups: BTreeMap<u16, Vec<&skpwright::MeshFace>> = BTreeMap::new();
    for face in &mesh.faces {
        if face.hidden {
            stats.hidden_faces_skipped += 1;
            continue;
        }
        if !layer_visible(face.layer) {
            stats.invisible_layer_faces_skipped += 1;
            continue;
        }
        let own = face.front_material.filter(|slot| *slot != 0);
        let effective = own.unwrap_or(inherited_material);
        groups.entry(effective).or_default().push(face);
    }
    groups
}

/// `(colour, opacity)` of a material slot; `(None, None)` when it is unknown.
pub fn material_style(model: &skpwright::Model, slot: u16) -> (Option<[u8; 3]>, Option<f32>) {
    style_of(model.material_of(slot))
}

/// `(colour, opacity)` of one material.
///
/// A textured material contributes its average colour (the value SketchUp's own
/// COLLADA export writes as the diffuse colour), because this adapter does not
/// sample the image. Opacity is reported only when the material is genuinely
/// translucent; `1.0` and out-of-range values stay `ByLayer`.
pub fn style_of(material: Option<&skpwright::Material>) -> (Option<[u8; 3]>, Option<f32>) {
    let Some(material) = material else {
        return (None, None);
    };
    let (rgba, opacity) = match material {
        skpwright::Material::Solid { rgba, opacity, .. } => (*rgba, *opacity),
        skpwright::Material::Textured {
            avg_rgba, opacity, ..
        } => (avg_rgba.unwrap_or([255, 255, 255, 255]), *opacity),
    };
    let color = Some([rgba[0], rgba[1], rgba[2]]);
    // `contains` is false for NaN, so a non-finite opacity stays `ByLayer`.
    let opacity = (0.0..1.0).contains(&opacity).then_some(opacity as f32);
    (color, opacity)
}

#[derive(Default)]
struct GroupBuffers {
    vertices: Vec<Point3>,
    normals: Vec<Point3>,
    triangles: Vec<[u32; 3]>,
}

/// Triangulate one face and append it to `out`, in world metres.
fn emit_face(
    mesh: &skpwright::Mesh,
    face: &skpwright::MeshFace,
    world: &[f64; 16],
    out: &mut GroupBuffers,
) -> Result<(), FaceSkip> {
    let normal = unit(face.normal).ok_or(FaceSkip::Malformed)?;
    // Right-handed in-plane basis: `u x v == normal`, so a counter-clockwise
    // 2-D ring is counter-clockwise around the face normal.
    let seed = if normal[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let u = unit(cross(seed, normal)).ok_or(FaceSkip::Malformed)?;
    let v = cross(normal, u);

    let origin = point(mesh, *face.outer.first().ok_or(FaceSkip::Malformed)?)?;
    let project = |p: [f64; 3]| {
        let d = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
        [dot(d, u), dot(d, v)]
    };

    let mut rings: Vec<Vec<[f64; 2]>> = Vec::with_capacity(1 + face.holes.len());
    let mut outer = Vec::with_capacity(face.outer.len());
    for index in &face.outer {
        outer.push(project(point(mesh, *index)?));
    }
    rings.push(outer);
    for hole in &face.holes {
        let mut ring = Vec::with_capacity(hole.len());
        for index in hole {
            ring.push(project(point(mesh, *index)?));
        }
        rings.push(ring);
    }

    // A hole-free face is an ordinary simple polygon, and ear clipping gives it
    // the minimal n-2 triangles. Only a face **with holes** needs the even-odd
    // filler, whose y-band decomposition is correct but emits many slivers:
    // using it for every face inflated a 13k-face model to ~130k triangles.
    //
    // Ear clipping is only accepted when its covered area matches the ring's
    // own signed area. A ring it cannot fill consistently — non-simple,
    // collinear-heavy, or over the point budget — falls back to the even-odd
    // filler, which validates its result against the ring nesting itself. If
    // both refuse, the face is reported and omitted; it is never approximated.
    let (positions, triangles) = if face.holes.is_empty() {
        match ear_fill(&rings[0]) {
            Some(triangles) => (rings.swap_remove(0), triangles),
            None => even_odd_fill(&rings)?,
        }
    } else {
        even_odd_fill(&rings)?
    };
    if triangles.is_empty() {
        return Err(FaceSkip::Untriangulable);
    }

    let base = out.vertices.len();
    for p in &positions {
        let local = [
            origin[0] + u[0] * p[0] + v[0] * p[1],
            origin[1] + u[1] * p[0] + v[1] * p[1],
            origin[2] + u[2] * p[0] + v[2] * p[1],
        ];
        let world_point = apply_point(world, local);
        out.vertices.push(Point3 {
            x: world_point[0],
            y: world_point[1],
            z: world_point[2],
        });
    }

    let vertex_count = positions.len() as u32;
    let mut world_triangles = Vec::with_capacity(triangles.len());
    for triangle in &triangles {
        let [a, b, c] = *triangle;
        if a == b || b == c || a == c {
            continue;
        }
        if a >= vertex_count || b >= vertex_count || c >= vertex_count {
            return Err(FaceSkip::Malformed);
        }
        world_triangles.push([base as u32 + a, base as u32 + b, base as u32 + c]);
    }
    if world_triangles.is_empty() {
        return Err(FaceSkip::Untriangulable);
    }

    // Derive the normal from the transformed geometry itself instead of
    // transforming the stored normal: a mirroring instance (negative
    // determinant) reverses the winding, and the inverse-transpose rule then
    // signs the normal the other way. Reading it off the triangles keeps the
    // normal and the winding consistent by construction, whatever the instance
    // transform is.
    let world_normal = world_triangles
        .iter()
        .find_map(|triangle| {
            let a = out.vertices[triangle[0] as usize];
            let b = out.vertices[triangle[1] as usize];
            let c = out.vertices[triangle[2] as usize];
            unit(cross(
                [b.x - a.x, b.y - a.y, b.z - a.z],
                [c.x - a.x, c.y - a.y, c.z - a.z],
            ))
        })
        .or_else(|| unit(apply_direction(world, normal)))
        .unwrap_or(normal);

    for _ in 0..positions.len() {
        out.normals.push(Point3 {
            x: world_normal[0],
            y: world_normal[1],
            z: world_normal[2],
        });
    }
    out.triangles.extend(world_triangles);
    Ok(())
}

/// Ear-clip a hole-free ring and accept the result only if it covers exactly
/// the ring's area. Overlapping or missing triangles from a non-simple ring
/// change that sum, so a mismatch is a refusal rather than a wrong fill.
fn ear_fill(ring: &[[f64; 2]]) -> Option<Vec<[u32; 3]>> {
    let triangles = triangulate(ring);
    if triangles.is_empty() {
        return None;
    }
    let expected = shoelace(ring).abs();
    let covered: f64 = triangles
        .iter()
        .map(|t| {
            triangle_area_2d(
                ring[t[0] as usize],
                ring[t[1] as usize],
                ring[t[2] as usize],
            )
        })
        .sum();
    if (covered - expected).abs() > expected.max(1.0) * 1e-9 {
        return None;
    }
    Some(triangles)
}

/// A triangulated face in its own plane: 2-D positions plus triangle indices
/// into them.
type FaceFill = (Vec<[f64; 2]>, Vec<[u32; 3]>);

/// Even-odd fill for rings with holes (and for rings ear clipping refused).
fn even_odd_fill(rings: &[Vec<[f64; 2]>]) -> Result<FaceFill, FaceSkip> {
    let fill = fill_rings(rings).map_err(|_| FaceSkip::Untriangulable)?;
    if fill.triangles.is_empty() {
        return Err(FaceSkip::Untriangulable);
    }
    Ok((fill.vertices, fill.triangles))
}

fn shoelace(ring: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum * 0.5
}

fn triangle_area_2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() * 0.5
}

fn point(mesh: &skpwright::Mesh, index: u32) -> Result<[f64; 3], FaceSkip> {
    mesh.vertices
        .get(index as usize)
        .copied()
        .ok_or(FaceSkip::Malformed)
}

fn unit(value: [f64; 3]) -> Option<[f64; 3]> {
    let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
    if !length.is_finite() || length < 1e-12 {
        return None;
    }
    Some([value[0] / length, value[1] / length, value[2] / length])
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Row-major 4x4 point transform (`p' = M · [p, 1]`).
fn apply_point(m: &[f64; 16], p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
    ]
}

/// Row-major 4x4 direction transform (no translation).
fn apply_direction(m: &[f64; 16], v: [f64; 3]) -> [f64; 3] {
    [
        m[0] * v[0] + m[1] * v[1] + m[2] * v[2],
        m[4] * v[0] + m[5] * v[1] + m[6] * v[2],
        m[8] * v[0] + m[9] * v[1] + m[10] * v[2],
    ]
}

/// Determinant of the upper-left 3x3 block; negative means the transform
/// mirrors, which flips face orientation.
pub fn determinant3(m: &[f64; 16]) -> f64 {
    m[0] * (m[5] * m[10] - m[6] * m[9]) - m[1] * (m[4] * m[10] - m[6] * m[8])
        + m[2] * (m[4] * m[9] - m[5] * m[8])
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY_WORLD: [f64; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    fn mesh_with(vertices: Vec<[f64; 3]>, faces: Vec<skpwright::MeshFace>) -> skpwright::Mesh {
        skpwright::Mesh {
            vertices,
            faces,
            ..Default::default()
        }
    }

    fn face(outer: Vec<u32>) -> skpwright::MeshFace {
        skpwright::MeshFace {
            outer,
            ..Default::default()
        }
    }

    fn quad() -> (skpwright::Mesh, skpwright::MeshFace) {
        let vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        (mesh_with(vertices, vec![]), face(vec![0, 1, 2, 3]))
    }

    fn triangle_area(a: Point3, b: Point3, c: Point3) -> f64 {
        let ab = [b.x - a.x, b.y - a.y, b.z - a.z];
        let ac = [c.x - a.x, c.y - a.y, c.z - a.z];
        let n = cross(ab, ac);
        (dot(n, n)).sqrt() * 0.5
    }

    fn total_area(out: &GroupBuffers) -> f64 {
        out.triangles
            .iter()
            .map(|t| {
                triangle_area(
                    out.vertices[t[0] as usize],
                    out.vertices[t[1] as usize],
                    out.vertices[t[2] as usize],
                )
            })
            .sum()
    }

    /// Every triangle's geometric normal must agree with the stored vertex
    /// normal, or back-face culling would hide geometry SketchUp shows.
    fn assert_winding_matches_normals(out: &GroupBuffers) {
        for triangle in &out.triangles {
            let a = out.vertices[triangle[0] as usize];
            let b = out.vertices[triangle[1] as usize];
            let c = out.vertices[triangle[2] as usize];
            let geometric = cross(
                [b.x - a.x, b.y - a.y, b.z - a.z],
                [c.x - a.x, c.y - a.y, c.z - a.z],
            );
            let normal = out.normals[triangle[0] as usize];
            let normal = [normal.x, normal.y, normal.z];
            assert!(
                dot(geometric, normal) > 0.0,
                "triangle winding disagrees with its vertex normal"
            );
        }
    }

    #[test]
    fn a_quad_becomes_two_triangles_with_the_face_normal() {
        let (mesh, face) = quad();
        let mut out = GroupBuffers::default();
        emit_face(&mesh, &face, &IDENTITY_WORLD, &mut out).expect("a planar quad fills");

        assert_eq!(out.triangles.len(), 2);
        assert!(
            (total_area(&out) - 1.0).abs() < 1e-9,
            "area must be preserved"
        );
        for normal in &out.normals {
            assert!((normal.z - 1.0).abs() < 1e-9, "flat face normal is +Z");
        }
        assert_winding_matches_normals(&out);
    }

    #[test]
    fn a_face_with_a_hole_keeps_the_hole_open() {
        // 4x4 outer square with a 2x2 hole: 12 m² of fill, nothing inside it.
        let vertices = vec![
            [-2.0, -2.0, 0.0],
            [2.0, -2.0, 0.0],
            [2.0, 2.0, 0.0],
            [-2.0, 2.0, 0.0],
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ];
        let mesh = mesh_with(vertices, vec![]);
        let mut with_hole = face(vec![0, 1, 2, 3]);
        with_hole.holes = vec![vec![4, 5, 6, 7]];

        let mut out = GroupBuffers::default();
        emit_face(&mesh, &with_hole, &IDENTITY_WORLD, &mut out).expect("donut fills");

        assert!(
            (total_area(&out) - 12.0).abs() < 1e-6,
            "outer minus hole area, got {}",
            total_area(&out)
        );
        for triangle in &out.triangles {
            let pts: Vec<Point3> = triangle.iter().map(|i| out.vertices[*i as usize]).collect();
            let cx = (pts[0].x + pts[1].x + pts[2].x) / 3.0;
            let cy = (pts[0].y + pts[1].y + pts[2].y) / 3.0;
            assert!(
                !(cx > -0.999 && cx < 0.999 && cy > -0.999 && cy < 0.999),
                "a triangle lies inside the hole at ({cx}, {cy})"
            );
        }
        assert_winding_matches_normals(&out);
    }

    #[test]
    fn a_mirroring_instance_flips_winding_and_normal() {
        let (mesh, face) = quad();
        let mirrored_world = [
            -1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        assert!(determinant3(&mirrored_world) < 0.0);
        let mut out = GroupBuffers::default();
        emit_face(&mesh, &face, &mirrored_world, &mut out).expect("a mirrored quad fills");

        for normal in &out.normals {
            assert!(
                (normal.z + 1.0).abs() < 1e-9,
                "mirroring negates the normal"
            );
        }
        assert_winding_matches_normals(&out);
        let xs: Vec<f64> = out.vertices.iter().map(|p| p.x).collect();
        assert!(
            xs.iter().all(|x| *x <= 0.0),
            "mirrored vertices are on -X: {xs:?}"
        );
    }

    #[test]
    fn a_translated_instance_moves_its_vertices() {
        let (mesh, face) = quad();
        let world = [
            1.0, 0.0, 0.0, 10.0, //
            0.0, 1.0, 0.0, 20.0, //
            0.0, 0.0, 1.0, 30.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let mut out = GroupBuffers::default();
        emit_face(&mesh, &face, &world, &mut out).expect("a translated quad fills");
        for vertex in &out.vertices {
            assert!((vertex.x - 10.0).abs() < 1e-9 || (vertex.x - 11.0).abs() < 1e-9);
            assert!((vertex.y - 20.0).abs() < 1e-9 || (vertex.y - 21.0).abs() < 1e-9);
            assert!((vertex.z - 30.0).abs() < 1e-9);
        }
    }

    #[test]
    fn an_invisible_tag_and_a_hidden_face_are_pruned() {
        let mut visible = face(vec![0, 1, 2, 3]);
        visible.layer = 5;
        let mut on_invisible_tag = face(vec![0, 1, 2, 3]);
        on_invisible_tag.layer = 6;
        let mut hidden = face(vec![0, 1, 2, 3]);
        hidden.hidden = true;
        let mesh = mesh_with(
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            vec![visible, on_invisible_tag, hidden],
        );
        let mut stats = SkpStats::default();
        let visible_tag = |slot: u16| slot != 6;
        let groups = group_faces(&mesh, 0, &visible_tag, &mut stats);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[&0].len(), 1);
        assert_eq!(stats.hidden_faces_skipped, 1);
        assert_eq!(stats.invisible_layer_faces_skipped, 1);
    }

    #[test]
    fn an_inherited_material_paints_only_unpainted_faces() {
        let mut own = face(vec![0, 1, 2, 3]);
        own.front_material = Some(7);
        let inherited = face(vec![0, 1, 2, 3]);
        let mesh = mesh_with(
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            vec![own, inherited],
        );
        let mut stats = SkpStats::default();
        let groups = group_faces(&mesh, 3, &|_| true, &mut stats);

        assert_eq!(groups[&7].len(), 1, "the face's own material wins");
        assert_eq!(
            groups[&3].len(),
            1,
            "the rest inherit the instance material"
        );
    }

    #[test]
    fn a_degenerate_face_is_reported_instead_of_approximated() {
        // Three collinear vertices: the filler refuses rather than inventing fill.
        let mesh = mesh_with(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            vec![],
        );
        let mut degenerate = face(vec![0, 1, 2]);
        degenerate.normal = [0.0, 0.0, 1.0];
        let mut out = GroupBuffers::default();
        let result = emit_face(&mesh, &degenerate, &IDENTITY_WORLD, &mut out);

        assert_eq!(result, Err(FaceSkip::Untriangulable));
        assert!(out.vertices.is_empty() && out.triangles.is_empty());
    }

    #[test]
    fn a_ring_index_outside_the_run_is_malformed() {
        let mesh = mesh_with(vec![[0.0, 0.0, 0.0]], vec![]);
        let mut out = GroupBuffers::default();
        let result = emit_face(&mesh, &face(vec![0, 1, 2]), &IDENTITY_WORLD, &mut out);
        assert_eq!(result, Err(FaceSkip::Malformed));
    }

    #[test]
    fn a_zero_length_normal_is_malformed() {
        let (mesh, mut face) = quad();
        face.normal = [0.0, 0.0, 0.0];
        let mut out = GroupBuffers::default();
        let result = emit_face(&mesh, &face, &IDENTITY_WORLD, &mut out);
        assert_eq!(result, Err(FaceSkip::Malformed));
    }
}
