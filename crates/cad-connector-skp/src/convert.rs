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
//!
//! Real `MeshEdge` records are carried alongside (ADR-0049): a material group
//! whose visible face rings are all backed by records becomes a
//! [`cad_domain::Body3d`] body with its own boundary edges, and the remaining
//! visible records become standalone wire bodies. [`crate::edges`] owns that
//! filtering and indexing; this module only decides the carrier.

use std::collections::{BTreeMap, BTreeSet};

use cad_connector_core::{ImportLimits, MeshImport};
use cad_domain::hatch::{fill_rings, triangulate};
use cad_domain::{CadError, Mesh, Point3};

use crate::edges::{face_ring_pairs, Boundary, EdgeStats, RunEdges, VertexPair};
use crate::texture::{self, TextureSlots};
use skpwright::UvXform;

/// Maximum instance nesting depth walked. A corrupt or hostile file must not
/// turn the scene walk into unbounded recursion.
pub const MAX_DEPTH: usize = 64;

/// Edge segments one import may emit when the caller leaves
/// [`ImportLimits::max_mesh_edges`] unset.
///
/// This is the reader's own budget, never a truncation: exceeding it is
/// `import.skp.limits_exceeded` with the real count.
pub const DEFAULT_MAX_MESH_EDGES: usize = 2_000_000;

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
    /// The group's real boundary edges, in world metres (two points each).
    ///
    /// Empty means the group is inserted as a plain
    /// [`cad_domain::SemanticGeometry::Mesh`]; a non-empty set means every
    /// visible face ring of the group was backed by `MeshEdge` records and the
    /// group is inserted as a [`cad_domain::Body3d`] body (ADR-0049).
    pub edges: Vec<Vec<Point3>>,
    /// `None` = no material anywhere on this group (`ByLayer`).
    pub color: Option<[u8; 3]>,
    /// `Some(opacity)` only when the material is genuinely translucent.
    pub opacity: Option<f32>,
    /// Effective material slot this group was built from (`0` = none).
    pub material: u16,
    /// Triangles contributed (equal to `mesh.triangles.len()`).
    pub triangles: usize,
}

/// One standalone wire entity: visible edges no visible face boundary covers.
#[derive(Debug, Clone)]
pub struct PlacedEdges {
    /// World-metre two-point polylines, deduplicated within the group.
    pub edges: Vec<Vec<Point3>>,
    /// `None` = the edge's effective material slot has no colour.
    pub color: Option<[u8; 3]>,
    pub opacity: Option<f32>,
    /// Effective edge material slot (`0` = none).
    pub material: u16,
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
    /// Edge records and the segments they became (ADR-0049). Every record is
    /// either stored or in exactly one skip counter; nothing is a silent drop.
    pub edges: EdgeStats,
    /// Materials in the model.
    pub materials: usize,
    /// Tags (layers) in the model.
    pub layers: usize,
    /// Materials the model declares as textured (embedded image or external
    /// path).
    pub textured_materials: usize,
    /// Faces emitted inside a group that really carries its texture.
    pub textured_faces: usize,
    /// Material groups whose `Mesh` carries a texture key.
    pub textured_groups: usize,
    /// Material groups whose material is textured but whose `Mesh` stays
    /// untextured (external image, refused registration, no UV transform).
    pub untextured_textured_groups: usize,
    /// Textured materials that reference an external image file instead of
    /// embedded bytes.
    pub external_textures: usize,
    /// Textured faces for which no §4v UV transform resolved.
    pub uv_unavailable: usize,
    /// Faces painted with a textured back-face material (not imported).
    pub back_textured_faces: usize,
    /// The first texture registration refusal, verbatim; its `import.texture.*`
    /// marker becomes the diagnostic code.
    pub texture_refusal: Option<String>,
}

/// The plottable result of a scene walk.
#[derive(Debug, Default)]
pub struct SkpScene {
    /// Face/material groups (each inserted as a mesh or a body).
    pub placed: Vec<PlacedMesh>,
    /// Standalone wire entities (edges no visible face boundary covers).
    pub edges: Vec<PlacedEdges>,
    pub stats: SkpStats,
}

impl SkpScene {
    /// Entities this scene will insert.
    pub fn entity_count(&self) -> usize {
        self.placed.len() + self.edges.len()
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
    import: &mut MeshImport,
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
    let mut edges = Vec::new();
    let mut textures = TextureSlots::default();
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
            import,
            &mut textures,
            &mut placed,
            &mut edges,
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
            import,
            &mut textures,
            &mut placed,
            &mut edges,
            &mut stats,
        )?;
    }

    Ok(SkpScene {
        placed,
        edges,
        stats,
    })
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
    import: &mut MeshImport,
    textures: &mut TextureSlots,
    placed: &mut Vec<PlacedMesh>,
    edges: &mut Vec<PlacedEdges>,
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
            import,
            textures,
            placed,
            edges,
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
            import,
            textures,
            placed,
            edges,
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
    import: &mut MeshImport,
    textures: &mut TextureSlots,
    placed: &mut Vec<PlacedMesh>,
    edges: &mut Vec<PlacedEdges>,
    stats: &mut SkpStats,
) -> cad_domain::CadResult<()> {
    let Some(run) = model.geometry.get(run_index) else {
        return Ok(());
    };
    stats.placed_runs += 1;
    // Back-face textures are not imported (the face's front side is the one this
    // adapter paints); count them so the caller can say so once.
    stats.back_textured_faces += run
        .mesh
        .faces
        .iter()
        .filter(|face| !face.hidden && layer_visible(face.layer))
        .filter(|face| texture::has_textured_back(model, face))
        .count();
    let groups = group_faces(&run.mesh, inherited_material, layer_visible, stats);
    let run_edges = RunEdges::new(&run.mesh, layer_visible, &mut stats.edges);
    // Ring pairs a visible face boundary already accounts for, split by the
    // carrier that draws them: a preview body stores its face-ring records, and a
    // mesh-carrier group draws its triangulated surface boundary. Those faces'
    // edges must not also become standalone wires, and each covered record is
    // counted in exactly one bucket.
    let mut covered_by_body: BTreeSet<VertexPair> = BTreeSet::new();
    let mut covered_by_mesh: BTreeSet<VertexPair> = BTreeSet::new();

    for (material, faces) in groups {
        let (color, opacity) = material_style(model, material);
        let plan = texture::plan_group(model, material, &faces, import, textures);
        stats.external_textures += usize::from(plan.external_image);
        if let Some(refusal) = plan.refusal {
            if stats.texture_refusal.is_none() {
                stats.texture_refusal = Some(refusal);
            }
        }
        stats.uv_unavailable += plan.uv_unavailable;
        let uv_ok = plan.key.is_some();
        // The boundary is decided before triangulation: a group's real edges do
        // not depend on whether its faces tessellate (ADR-0049).
        let boundary = run_edges.boundary(&run.mesh, &faces, world);
        let mut out = GroupBuffers::default();
        let mut meshed: BTreeSet<VertexPair> = BTreeSet::new();
        for (index, face) in faces.iter().enumerate() {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            let uv = if uv_ok {
                plan.xforms[index].as_ref()
            } else {
                None
            };
            match emit_face(&run.mesh, face, world, uv, &mut out) {
                Ok(()) => {
                    stats.faces_emitted += 1;
                    if uv.is_some() {
                        stats.textured_faces += 1;
                    }
                    meshed.extend(face_ring_pairs(face));
                }
                Err(FaceSkip::Malformed) => stats.faces_malformed += 1,
                Err(FaceSkip::Untriangulable) => stats.faces_untriangulable += 1,
            }
            check_face_budget(stats, limits)?;
        }
        if out.triangles.is_empty() {
            if plan.material_is_textured {
                stats.untextured_textured_groups += 1;
            }
            // No triangles anywhere in this group: its faces contribute no
            // visible boundary, so their records stay available as wires.
            continue;
        }
        let mut mesh = Mesh {
            vertices: out.vertices,
            triangles: out.triangles,
            normals: out.normals,
            face_sources: Vec::new(),
            colors: Vec::new(),
            uvs: out.uvs,
            texture: None,
            pbr: None,
        };
        if uv_ok && mesh.uvs.len() == mesh.vertices.len() {
            mesh.texture = plan.key;
            stats.textured_groups += 1;
        } else {
            // A group without a usable texture is exactly what it was before
            // ADR-0038: average colour, no UVs (a UV set with no texture would
            // only be dead weight the renderer has no use for).
            mesh.uvs.clear();
            if plan.material_is_textured {
                stats.untextured_textured_groups += 1;
            }
        }
        let edges = match boundary {
            Boundary::Complete { edges, curves } if !edges.is_empty() => {
                // A body carries the group's whole ring boundary, including a
                // face whose triangulation failed but whose edges are real.
                covered_by_body.extend(faces.iter().flat_map(|face| face_ring_pairs(face)));
                stats.edges.curves_flattened += curves;
                edges
            }
            Boundary::Complete { .. } => {
                // Complete records but nothing visible survived the filter: the
                // honest carrier is the mesh (never an edge-less body).
                stats.edges.groups_empty_boundary += 1;
                covered_by_mesh.extend(meshed);
                Vec::new()
            }
            Boundary::MissingRecords(missing) => {
                // A ring segment is not a SketchUp edge: keep the mesh carrier
                // and let ADR-0045 derive the wireframe from the triangles.
                stats.edges.groups_without_records += 1;
                stats.edges.unrecorded_boundary_segments += missing;
                covered_by_mesh.extend(meshed);
                Vec::new()
            }
        };
        stats.edges.emitted += edges.len();
        stats.edges.boundary += edges.len();
        check_edge_budget(stats, limits)?;
        placed.push(PlacedMesh {
            triangles: mesh.triangles.len(),
            mesh,
            edges,
            color,
            opacity,
            material,
        });
    }

    // Visible records no visible face boundary covers: loose line work and the
    // edges of faces whose triangulation failed.
    for (material, wires) in run_edges.wires(
        &run.mesh,
        &covered_by_body,
        &covered_by_mesh,
        world,
        &mut stats.edges,
    ) {
        stats.edges.duplicate_skipped += wires.duplicates;
        stats.edges.zero_length_skipped += wires.degenerate;
        if wires.edges.is_empty() {
            continue;
        }
        stats.edges.curves_flattened += wires.curves;
        stats.edges.emitted += wires.edges.len();
        stats.edges.wire += wires.edges.len();
        check_edge_budget(stats, limits)?;
        let (color, opacity) = material_style(model, material);
        edges.push(PlacedEdges {
            edges: wires.edges,
            color,
            opacity,
            material,
        });
    }
    Ok(())
}

fn check_edge_budget(stats: &SkpStats, limits: &ImportLimits) -> cad_domain::CadResult<()> {
    let max = limits.max_mesh_edges.unwrap_or(DEFAULT_MAX_MESH_EDGES);
    if stats.edges.emitted > max {
        return Err(CadError::InvalidInput(format!(
            "import.skp.limits_exceeded: {} edge segment(s) over the {max} segment budget",
            stats.edges.emitted
        )));
    }
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

#[derive(Default)]
struct GroupBuffers {
    vertices: Vec<Point3>,
    normals: Vec<Point3>,
    triangles: Vec<[u32; 3]>,
    /// Per-vertex UV0, 1:1 with `vertices`; filled only when a §4v transform is
    /// supplied, so the group is textured exactly when this is non-empty.
    uvs: Vec<[f64; 2]>,
}

/// `(colour, opacity)` of one material.
///
/// A textured material contributes its average colour (the value SketchUp's own
/// COLLADA export writes as the diffuse colour), which is what a group whose
/// image could not be registered keeps. Opacity is reported only when the
/// material is genuinely translucent; `1.0` and out-of-range values stay
/// `ByLayer`.
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

/// Triangulate one face and append it to `out`, in world metres.
///
/// `uv` is the face's §4v transform when the group is textured: it is evaluated
/// on each **world** vertex (metres), which is the frame `UvXform::apply` is
/// documented for and the same point the mesh stores.
fn emit_face(
    mesh: &skpwright::Mesh,
    face: &skpwright::MeshFace,
    world: &[f64; 16],
    uv: Option<&UvXform>,
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
        if let Some(xform) = uv {
            out.uvs.push(xform.apply(world_point));
        }
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
pub(crate) fn apply_point(m: &[f64; 16], p: [f64; 3]) -> [f64; 3] {
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
mod tests;
