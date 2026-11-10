//! Unit 2: a single-buffer GLB -> neutral meshes in the CAD frame.
//!
//! This is the half that can be tested without any Revit file: a synthetic GLB
//! with two translated nodes proves that the **world** transform of every node
//! is applied. A naive read of mesh-local positions would stack both nodes at
//! the origin — the exact failure the STEP and IFC lanes had to prove they
//! avoided.
//!
//! Frame and units are fixed by the intermediate format, not assumed:
//!
//! * glTF 2.0 is **metres, +Y up** (spec). `rvt` writes exactly that: its
//!   element nodes are in feet, and one root node carries
//!   `Z_UP_FEET_TO_Y_UP_METRES` (`rvt` `src/ifc/gltf.rs:242`), a `0.3048`
//!   scale with a `(x, y, z) -> (x, z, -y)` axis swap. World transforms are
//!   therefore applied first, then the glTF frame is mapped back to the CAD
//!   convention `(x, y, z) -> (x, -z, y)`, so stored coordinates are **metres,
//!   +Z up**.
//! * Every node matrix is **column-major** (`gltf::scene::Transform::matrix`,
//!   `src/scene/mod.rs:13`), and a child's world matrix is
//!   `parent_world * local`.
//!
//! Only triangle primitives are converted. Non-indexed triangle soup is
//! accepted; strip/fan modes are skipped and counted so no primitive
//! disappears silently.

use cad_domain::{CadError, CadResult, Mesh, Point3};
use cad_import_core::ImportLimits;
use gltf::accessor::{Accessor, DataType, Dimensions};
use gltf::buffer::Source;
use gltf::mesh::Mode;
use gltf::{Gltf, Node, Primitive, Semantic};

use crate::{
    EMPTY_GLB, EXTERNAL_BUFFER, GLB_INVALID, INDEX_OUT_OF_RANGE, INVALID_TRANSFORM,
    MESH_WITHOUT_POSITIONS, NODE_CYCLE, NON_FINITE_VERTEX, NO_SCENE, SPARSE_ACCESSOR,
};

/// The maximum node-hierarchy depth accepted before a recursion guard trips.
/// glTF trees are shallow; this bounds recursion on hostile input.
const MAX_NODE_DEPTH: usize = 64;

/// The identity affine. Scene roots are visited with it so their local matrix
/// is applied exactly once inside [`visit_node`].
const IDENTITY: [[f64; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// One converted node/mesh instance: the caller writes one `DbEntity` for it.
#[derive(Debug)]
pub struct PlacedMesh {
    /// The glTF node name, when the file carries one.
    pub name: Option<String>,
    pub mesh: Mesh,
}

/// Aggregate losses from one GLB read, for stable diagnostics.
#[derive(Debug, Default)]
pub struct GlbScene {
    pub meshes: Vec<PlacedMesh>,
    pub nodes_visited: usize,
    pub primitives_skipped: usize,
    pub empty_nodes: usize,
    pub degenerate_triangles: usize,
    pub normals_dropped: usize,
}

/// Read a GLB and return every drawable node/mesh instance in world space.
pub fn read_glb(
    bytes: &[u8],
    limits: &ImportLimits,
    cancelled: &dyn Fn() -> bool,
) -> CadResult<GlbScene> {
    if !bytes.starts_with(b"glTF") {
        return Err(CadError::CorruptData(format!(
            "{EMPTY_GLB}: the intermediate is not a binary glTF (missing the `glTF` magic)"
        )));
    }
    let gltf = Gltf::from_slice(bytes).map_err(|error| {
        CadError::CorruptData(format!(
            "{GLB_INVALID}: the intermediate GLB is invalid: {error}"
        ))
    })?;
    let document = &gltf.document;

    // A GLB produced by `rvt` is self-contained: one binary buffer in the BIN
    // chunk and no external URI. This adapter may not fetch bytes, so an
    // external buffer is refused instead of half-read.
    for buffer in document.buffers() {
        if matches!(buffer.source(), Source::Uri(_)) {
            return Err(CadError::Unsupported(format!(
                "{EXTERNAL_BUFFER}: the GLB references an external buffer URI; only the \
                 self-contained binary chunk `rvt` emits is read"
            )));
        }
    }
    let blob = gltf.blob.as_deref();

    let Some(scene) = document
        .default_scene()
        .or_else(|| document.scenes().next())
    else {
        return Err(CadError::CorruptData(format!(
            "{NO_SCENE}: the GLB declares no scene"
        )));
    };

    let mut scene_out = GlbScene::default();
    let mut stack = Vec::new();
    for node in scene.nodes() {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        visit_node(
            blob,
            &node,
            IDENTITY,
            &mut stack,
            limits,
            cancelled,
            &mut scene_out,
        )?;
    }
    Ok(scene_out)
}

#[allow(clippy::too_many_arguments)]
fn visit_node(
    blob: Option<&[u8]>,
    node: &Node<'_>,
    parent_world: [[f64; 4]; 4],
    stack: &mut Vec<usize>,
    limits: &ImportLimits,
    cancelled: &dyn Fn() -> bool,
    out: &mut GlbScene,
) -> CadResult<()> {
    if cancelled() {
        return Err(CadError::Cancelled);
    }
    let index = node.index();
    if stack.contains(&index) {
        return Err(CadError::CorruptData(format!(
            "{NODE_CYCLE}: glTF node {index} closes a hierarchy cycle"
        )));
    }
    if stack.len() >= MAX_NODE_DEPTH {
        return Err(CadError::CorruptData(format!(
            "{INVALID_TRANSFORM}: glTF hierarchy is deeper than {MAX_NODE_DEPTH} nodes"
        )));
    }
    out.nodes_visited += 1;
    stack.push(index);

    let world = multiply(parent_world, &local_matrix(node)?);
    if !world.iter().flatten().all(|value| value.is_finite()) {
        return Err(CadError::CorruptData(format!(
            "{INVALID_TRANSFORM}: the world transform of glTF node {index} is not finite"
        )));
    }

    if let Some(mesh) = node.mesh() {
        let mut built = MeshBuild::default();
        for primitive in mesh.primitives() {
            if cancelled() {
                return Err(CadError::Cancelled);
            }
            if primitive.mode() == Mode::Triangles {
                build_primitive(blob, &primitive, &world, &mut built)?;
            } else {
                out.primitives_skipped += 1;
            }
        }
        out.degenerate_triangles += built.degenerate;
        out.normals_dropped += built.normals_dropped;
        // A node's mesh may carry several primitives. Normals are per vertex:
        // if any primitive lacks them (or its list was unusable), keeping the
        // other primitives' normals would produce a short list that no consumer
        // can index. Drop the merged list instead.
        if built.normals_absent || built.normals_dropped > 0 {
            built.mesh.normals.clear();
        }
        if built.mesh.triangles.is_empty() {
            out.empty_nodes += 1;
        } else {
            if out.meshes.len() >= limits.max_entities {
                return Err(CadError::InvalidInput(format!(
                    "the GLB has more than {} drawable node/mesh instance(s), over the entity \
                     import limit",
                    limits.max_entities
                )));
            }
            check_mesh_limits(
                node.name().unwrap_or("unnamed node"),
                built.mesh.triangles.len(),
                limits,
            )?;
            out.meshes.push(PlacedMesh {
                name: node.name().map(str::to_string),
                mesh: built.mesh,
            });
        }
    }

    for child in node.children() {
        visit_node(blob, &child, world, stack, limits, cancelled, out)?;
    }
    stack.pop();
    Ok(())
}

#[derive(Default)]
struct MeshBuild {
    mesh: Mesh,
    degenerate: usize,
    normals_dropped: usize,
    normals_absent: bool,
}

fn build_primitive(
    blob: Option<&[u8]>,
    primitive: &Primitive<'_>,
    world: &[[f64; 4]; 4],
    out: &mut MeshBuild,
) -> CadResult<()> {
    let Some(positions) = primitive
        .attributes()
        .find(|(semantic, _)| *semantic == Semantic::Positions)
        .map(|(_, accessor)| accessor)
    else {
        return Err(CadError::CorruptData(format!(
            "{MESH_WITHOUT_POSITIONS}: a glTF triangle primitive has no POSITION attribute"
        )));
    };
    let local = read_positions(&positions, blob)?;
    if local.len() > u32::MAX as usize {
        return Err(CadError::InvalidInput(format!(
            "a glTF primitive has {} vertices, over the u32 mesh index range",
            local.len()
        )));
    }
    let base = out.mesh.vertices.len();
    if base + local.len() > u32::MAX as usize {
        return Err(CadError::InvalidInput(
            "one glTF node/mesh instance exceeds the u32 mesh index range".into(),
        ));
    }

    // Normals are optional in glTF and `rvt` never emits them for its bodies.
    // A present-but-unusable list is dropped as a whole (never a partial list)
    // and counted; an absent one is simply empty.
    let normals = primitive
        .attributes()
        .find(|(semantic, _)| *semantic == Semantic::Normals)
        .map(|(_, accessor)| accessor);
    let transformed_normals = match normals {
        Some(accessor) => match (read_normals(&accessor, blob), normal_matrix(world)) {
            (Some(normals), Some(matrix)) if normals.len() == local.len() => {
                Some(transform_normals(&normals, &matrix))
            }
            _ => {
                out.normals_dropped += 1;
                None
            }
        },
        None => {
            out.normals_absent = true;
            None
        }
    };

    for point in &local {
        let world_point = apply(world, *point);
        if !world_point.x.is_finite() || !world_point.y.is_finite() || !world_point.z.is_finite() {
            return Err(CadError::CorruptData(format!(
                "{NON_FINITE_VERTEX}: a glTF vertex is non-finite after its node transform"
            )));
        }
        out.mesh.vertices.push(to_cad_frame(world_point));
    }
    if let Some(normals) = &transformed_normals {
        out.mesh.normals.extend(normals.iter().copied());
    }

    let indices = read_indices(primitive, &local, blob)?;
    if !indices.len().is_multiple_of(3) {
        return Err(CadError::CorruptData(format!(
            "{GLB_INVALID}: a triangle primitive has {} indices, not a multiple of 3",
            indices.len()
        )));
    }
    let vertex_count = local.len() as u32;
    for corner in indices.as_chunks::<3>().0 {
        if corner.iter().any(|index| *index >= vertex_count) {
            return Err(CadError::CorruptData(format!(
                "{INDEX_OUT_OF_RANGE}: a triangle references a vertex outside its \
                 {vertex_count}-vertex primitive"
            )));
        }
        let placed = [
            index_to_u32(base, corner[0])?,
            index_to_u32(base, corner[1])?,
            index_to_u32(base, corner[2])?,
        ];
        if placed[0] == placed[1]
            || placed[1] == placed[2]
            || placed[0] == placed[2]
            || is_zero_area(placed, &out.mesh.vertices)
        {
            out.degenerate += 1;
            continue;
        }
        out.mesh.triangles.push(placed);
    }
    Ok(())
}

fn read_positions(accessor: &Accessor<'_>, blob: Option<&[u8]>) -> CadResult<Vec<Point3>> {
    if accessor.data_type() != DataType::F32 || accessor.dimensions() != Dimensions::Vec3 {
        return Err(CadError::CorruptData(format!(
            "{GLB_INVALID}: a POSITION accessor is {:?}/{:?}, not F32/Vec3",
            accessor.data_type(),
            accessor.dimensions()
        )));
    }
    let (data, stride, count) = accessor_window(accessor, blob, 12)?;
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let bytes = &data[index * stride..index * stride + 12];
        out.push(read_vec3(bytes));
    }
    Ok(out)
}

fn read_normals(accessor: &Accessor<'_>, blob: Option<&[u8]>) -> Option<Vec<Point3>> {
    if accessor.data_type() != DataType::F32 || accessor.dimensions() != Dimensions::Vec3 {
        return None;
    }
    let (data, stride, count) = accessor_window(accessor, blob, 12).ok()?;
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        out.push(read_vec3(&data[index * stride..index * stride + 12]));
    }
    Some(out)
}

fn read_indices(
    primitive: &Primitive<'_>,
    positions: &[Point3],
    blob: Option<&[u8]>,
) -> CadResult<Vec<u32>> {
    let Some(accessor) = primitive.indices() else {
        // Non-indexed triangle soup.
        if !positions.len().is_multiple_of(3) {
            return Err(CadError::CorruptData(format!(
                "{GLB_INVALID}: a non-indexed triangle primitive has {} vertices, not a \
                 multiple of 3",
                positions.len()
            )));
        }
        return Ok((0..positions.len() as u32).collect());
    };
    if accessor.dimensions() != Dimensions::Scalar
        || !matches!(
            accessor.data_type(),
            DataType::U8 | DataType::U16 | DataType::U32
        )
    {
        return Err(CadError::CorruptData(format!(
            "{GLB_INVALID}: an index accessor is {:?}/{:?}, not an unsigned scalar",
            accessor.data_type(),
            accessor.dimensions()
        )));
    }
    let element = match accessor.data_type() {
        DataType::U8 => 1,
        DataType::U16 => 2,
        DataType::U32 => 4,
        _ => unreachable!("guarded above"),
    };
    let (data, stride, count) = accessor_window(&accessor, blob, element)?;
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let bytes = &data[index * stride..index * stride + element];
        out.push(match element {
            1 => u32::from(bytes[0]),
            2 => u32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
            _ => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        });
    }
    Ok(out)
}

/// Resolve an accessor to `(slice, stride, count)`, refusing sparse accessors
/// (this lane never produces them) and every read past the BIN chunk.
fn accessor_window<'a>(
    accessor: &Accessor<'_>,
    blob: Option<&'a [u8]>,
    element_size: usize,
) -> CadResult<(&'a [u8], usize, usize)> {
    if accessor.sparse().is_some() {
        return Err(CadError::Unsupported(format!(
            "{SPARSE_ACCESSOR}: a sparse glTF accessor is not supported"
        )));
    }
    let Some(view) = accessor.view() else {
        return Err(CadError::CorruptData(format!(
            "{MESH_WITHOUT_POSITIONS}: a glTF accessor has no buffer view"
        )));
    };
    if view.buffer().index() != 0 {
        return Err(CadError::CorruptData(format!(
            "{GLB_INVALID}: an accessor reads buffer {}, but a GLB carries only the binary \
             chunk in buffer 0",
            view.buffer().index()
        )));
    }
    let Some(blob) = blob else {
        return Err(CadError::CorruptData(format!(
            "{GLB_INVALID}: the GLB has no binary chunk for its accessors"
        )));
    };
    let stride = view.stride().unwrap_or(element_size);
    let start = view.offset() + accessor.offset();
    let count = accessor.count();
    let end = if count == 0 {
        start
    } else {
        start
            .checked_add(stride.saturating_mul(count - 1))
            .and_then(|value| value.checked_add(element_size))
            .ok_or_else(|| {
                CadError::CorruptData(format!("{GLB_INVALID}: a glTF accessor range overflows"))
            })?
    };
    let data = blob.get(start..end).ok_or_else(|| {
        CadError::CorruptData(format!(
            "{GLB_INVALID}: a glTF accessor reads bytes {start}..{end} past the {} byte BIN chunk",
            blob.len()
        ))
    })?;
    Ok((data, stride, count))
}

fn read_vec3(bytes: &[u8]) -> Point3 {
    Point3 {
        x: f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64,
        y: f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as f64,
        z: f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as f64,
    }
}

fn check_mesh_limits(name: &str, triangles: usize, limits: &ImportLimits) -> CadResult<()> {
    if let Some(max) = limits.max_mesh_faces {
        if triangles > max {
            return Err(CadError::InvalidInput(format!(
                "glTF node {name:?} has {triangles} triangle(s), over the {max} face import limit"
            )));
        }
    }
    if let Some(max) = limits.max_mesh_face_indices {
        let indices = triangles.saturating_mul(3);
        if indices > max {
            return Err(CadError::InvalidInput(format!(
                "glTF node {name:?} has {indices} face corner indices, over the {max} face-index \
                 import limit"
            )));
        }
    }
    Ok(())
}

/// `glTF [col][row]` -> row-major affine, in f64.
fn local_matrix(node: &Node<'_>) -> CadResult<[[f64; 4]; 4]> {
    let matrix = node.transform().matrix();
    let mut out = [[0.0f64; 4]; 4];
    for (row, output) in out.iter_mut().enumerate() {
        for (column, value) in output.iter_mut().enumerate() {
            *value = matrix[column][row] as f64;
        }
    }
    if !out.iter().flatten().all(|value| value.is_finite()) {
        return Err(CadError::CorruptData(format!(
            "{INVALID_TRANSFORM}: glTF node {} has a non-finite transform",
            node.index()
        )));
    }
    Ok(out)
}

fn multiply(left: [[f64; 4]; 4], right: &[[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let mut out = [[0.0f64; 4]; 4];
    for (row, output) in out.iter_mut().enumerate() {
        for (column, value) in output.iter_mut().enumerate() {
            *value = (0..4).map(|k| left[row][k] * right[k][column]).sum();
        }
    }
    out
}

fn apply(matrix: &[[f64; 4]; 4], point: Point3) -> Point3 {
    Point3 {
        x: matrix[0][0] * point.x + matrix[0][1] * point.y + matrix[0][2] * point.z + matrix[0][3],
        y: matrix[1][0] * point.x + matrix[1][1] * point.y + matrix[1][2] * point.z + matrix[1][3],
        z: matrix[2][0] * point.x + matrix[2][1] * point.y + matrix[2][2] * point.z + matrix[2][3],
    }
}

/// glTF (+Y up, right-handed) -> CAD (+Z up, right-handed).
fn to_cad_frame(point: Point3) -> Point3 {
    Point3 {
        x: point.x,
        y: -point.z,
        z: point.y,
    }
}

/// The inverse-transpose of the affine's linear part, or `None` when singular.
fn normal_matrix(matrix: &[[f64; 4]; 4]) -> Option<[[f64; 3]; 3]> {
    let a = [
        [matrix[0][0], matrix[0][1], matrix[0][2]],
        [matrix[1][0], matrix[1][1], matrix[1][2]],
        [matrix[2][0], matrix[2][1], matrix[2][2]],
    ];
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if !det.is_finite() || det.abs() <= f64::EPSILON {
        return None;
    }
    let inv = 1.0 / det;
    let cofactor = [
        [
            (a[1][1] * a[2][2] - a[1][2] * a[2][1]) * inv,
            (a[1][2] * a[2][0] - a[1][0] * a[2][2]) * inv,
            (a[1][0] * a[2][1] - a[1][1] * a[2][0]) * inv,
        ],
        [
            (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * inv,
            (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * inv,
            (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * inv,
        ],
        [
            (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * inv,
            (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * inv,
            (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * inv,
        ],
    ];
    cofactor
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        .then_some(cofactor)
}

fn transform_normals(normals: &[Point3], matrix: &[[f64; 3]; 3]) -> Vec<Point3> {
    normals
        .iter()
        .map(|normal| {
            let transformed = Point3 {
                x: matrix[0][0] * normal.x + matrix[0][1] * normal.y + matrix[0][2] * normal.z,
                y: matrix[1][0] * normal.x + matrix[1][1] * normal.y + matrix[1][2] * normal.z,
                z: matrix[2][0] * normal.x + matrix[2][1] * normal.y + matrix[2][2] * normal.z,
            };
            let cad = to_cad_frame(transformed);
            let length = (cad.x * cad.x + cad.y * cad.y + cad.z * cad.z).sqrt();
            if length.is_finite() && length > 0.0 {
                Point3 {
                    x: cad.x / length,
                    y: cad.y / length,
                    z: cad.z / length,
                }
            } else {
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                }
            }
        })
        .collect()
}

fn index_to_u32(base: usize, index: u32) -> CadResult<u32> {
    u32::try_from(base + index as usize)
        .map_err(|_| CadError::InvalidInput("glTF mesh exceeds the u32 index range".into()))
}

fn is_zero_area(triangle: [u32; 3], vertices: &[Point3]) -> bool {
    let p0 = vertices[triangle[0] as usize];
    let p1 = vertices[triangle[1] as usize];
    let p2 = vertices[triangle[2] as usize];
    let ux = p1.x - p0.x;
    let uy = p1.y - p0.y;
    let uz = p1.z - p0.z;
    let vx = p2.x - p0.x;
    let vy = p2.y - p0.y;
    let vz = p2.z - p0.z;
    let cx = uy * vz - uz * vy;
    let cy = uz * vx - ux * vz;
    let cz = ux * vy - uy * vx;
    cx == 0.0 && cy == 0.0 && cz == 0.0
}
