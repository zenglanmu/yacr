//! Headless geometry logic for the CAD pipelines (spec §5.2, §8; audit F14).
//!
//! Everything here is pure CPU math with no `wgpu` types, so it can be unit
//! tested on a host without a GPU. The renderer consumes these decisions when it
//! builds vertex/uniform data. In particular this module owns:
//!
//! * the camera projection for the 3D orbit path (`Camera3d::view_projection`),
//! * the mirrored-transform winding decision,
//! * normal repair (`repaired_normals`) for `Mesh` data with missing or
//!   inconsistent normals, and
//! * the per-frame vertex/triangle budget accounting.

use cad_domain::{Mesh, Point3};
use cad_scene::{FrameBudget, FrameUsage, RenderBatch, RenderTopology};

/// Camera used by the CAD viewport, in world units (2D plane view).
///
/// Kept here for the existing line path; the 3D orbit path uses [`Camera3d`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera2d {
    pub center: Point3,
    pub world_per_px: f64,
    pub z_plane: f32,
}

impl Default for Camera2d {
    fn default() -> Self {
        Camera2d {
            center: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            world_per_px: 1.0,
            z_plane: 0.0,
        }
    }
}

/// A 3D orbit camera: an eye, a target and an up vector in world space.
///
/// Spec §5.2/§3.3 F13/F14: the 3D view needs a real projection before standard
/// views, orbit and 2D/3D switching are meaningful. This is an explicit
/// right-handed look-at + perspective projection; no hidden-line removal is
/// claimed (F14: "非完整隐藏线算法").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera3d {
    pub eye: Point3,
    pub target: Point3,
    pub up: Point3,
    /// Vertical field of view in radians.
    pub fov_y: f64,
    /// Distance to the near plane, in world units.
    pub near: f64,
    /// Distance to the far plane, in world units.
    pub far: f64,
}

impl Camera3d {
    /// A camera looking straight down world `-Z` (the CAD plan orientation).
    pub fn plan_view(eye: Point3, target: Point3) -> Self {
        Camera3d {
            eye,
            target,
            up: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            fov_y: std::f64::consts::FRAC_PI_4,
            near: 1.0,
            far: 1.0e7,
        }
    }

    /// Right-handed view-projection matrix, column-major (WGSL `mat4x4<f32>`).
    ///
    /// Returns `None` when the configuration is degenerate (eye == target, up
    /// parallel to the view direction, or a non-positive near/far span) rather
    /// than producing NaNs the GPU would render as garbage.
    pub fn view_projection(&self, aspect: f64) -> Option<[[f32; 4]; 4]> {
        if !self.is_usable(aspect) {
            return None;
        }
        let view = self.look_at()?;
        let proj = self.perspective(aspect);
        Some(mat4_mul(&proj, &view))
    }

    /// Whether the camera has a usable projection for the given aspect.
    pub fn is_usable(&self, aspect: f64) -> bool {
        if !point_is_finite(self.eye) || !point_is_finite(self.target) || !point_is_finite(self.up)
        {
            return false;
        }
        if !aspect.is_finite() || aspect <= 0.0 {
            return false;
        }
        if !self.fov_y.is_finite() || self.fov_y <= 0.0 || self.fov_y >= std::f64::consts::PI {
            return false;
        }
        if !self.near.is_finite()
            || !self.far.is_finite()
            || self.near <= 0.0
            || self.far <= self.near
        {
            return false;
        }
        let forward = normalize(sub(self.target, self.eye));
        if length(forward) < 1e-12 {
            return false;
        }
        let side = cross(forward, self.up);
        if length(side) < 1e-12 {
            // Up is parallel to the view direction: no stable basis.
            return false;
        }
        true
    }

    fn look_at(&self) -> Option<[[f64; 4]; 4]> {
        let forward = normalize(sub(self.target, self.eye));
        if length(forward) < 1e-12 {
            return None;
        }
        let side = normalize(cross(forward, self.up));
        if length(side) < 1e-12 {
            return None;
        }
        let up = cross(side, forward);
        // Right-handed, looking down -Z in view space (wgpu convention).
        let z = Point3 {
            x: -forward.x,
            y: -forward.y,
            z: -forward.z,
        };
        Some([
            [side.x, up.x, z.x, 0.0],
            [side.y, up.y, z.y, 0.0],
            [side.z, up.z, z.z, 0.0],
            [
                -dot(side, self.eye),
                -dot(up, self.eye),
                -dot(z, self.eye),
                1.0,
            ],
        ])
    }

    fn perspective(&self, aspect: f64) -> [[f64; 4]; 4] {
        let f = 1.0 / (self.fov_y / 2.0).tan();
        let nf = 1.0 / (self.near - self.far);
        // wgpu clip space: z in [0, 1].
        [
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, self.far * nf, -1.0],
            [0.0, 0.0, self.near * self.far * nf, 0.0],
        ]
    }
}

/// Decide whether triangle winding must be flipped for back-face culling.
///
/// A negative-determinant transform (a mirror, e.g. a negative `INSERT` scale)
/// reverses the on-screen winding of every triangle, so a front-face setting
/// derived from the un-mirrored data would cull the visible side. This is the
/// explicit decision the audit (F14 "镜像绕序") found missing.
pub fn winding_is_flipped(mirrored: bool) -> bool {
    mirrored
}

/// Whether the GPU front face should be `Ccw` (`true`) or `Cw` (`false`).
///
/// Normal, un-mirrored CAD meshes are wound counter-clockwise, so the default is
/// `Ccw`. A mirrored batch flips this.
pub fn front_face_ccw(mirrored: bool) -> bool {
    !winding_is_flipped(mirrored)
}

/// Recompute usable per-vertex normals for a batch's mesh topology.
///
/// Rules (documented, F14 "缺失/不一致法向"):
/// * a normal vector is used as-is when it is finite and non-degenerate and the
///   batch has one per vertex;
/// * otherwise (missing, wrong count, NaN, or a zero normal) the vertex normal
///   is recomputed from the indexed triangles via [`cad_geometry::compute_vertex_normals`];
/// * if recomputation still yields a zero normal (a vertex touched only by
///   degenerate triangles), a fixed fallback normal is used so the shader never
///   sees NaN or a zero-length vector.
///
/// The returned vector is always `batch.vertices.len()` long.
pub fn repaired_normals(batch: &RenderBatch) -> Vec<[f32; 3]> {
    const FALLBACK: [f32; 3] = [0.0, 0.0, 1.0];
    let n = batch.vertices.len();
    let consistent = batch.normals.len() == n
        && batch
            .normals
            .iter()
            .all(|v| vector_is_usable([v[0] as f64, v[1] as f64, v[2] as f64]));
    if consistent {
        return batch
            .normals
            .iter()
            .map(|v| normalize3(*v).unwrap_or(FALLBACK))
            .collect();
    }
    // Rebuild an f64 `Mesh` from the batch's own f32 relative coordinates and
    // let the shared geometry helper do the area-weighted average.
    let mesh = Mesh {
        vertices: batch
            .vertices
            .iter()
            .map(|v| Point3 {
                x: v[0] as f64,
                y: v[1] as f64,
                z: v[2] as f64,
            })
            .collect(),
        triangles: batch.indices.clone(),
        normals: Vec::new(),
        face_sources: Vec::new(),
    };
    let computed = cad_geometry::compute_vertex_normals(&mesh);
    computed
        .iter()
        .map(|nn| normalize3([nn.x as f32, nn.y as f32, nn.z as f32]).unwrap_or(FALLBACK))
        .collect()
}

/// Whether a batch needs its normals repaired (missing, wrong length or bad).
pub fn normals_need_repair(batch: &RenderBatch) -> bool {
    batch.normals.len() != batch.vertices.len()
        || batch
            .normals
            .iter()
            .any(|v| !vector_is_usable([v[0] as f64, v[1] as f64, v[2] as f64]))
}

/// Deterministic (first, second) vertex indices for every edge of a batch's
/// triangle topology.
///
/// Used to build a `LineList` index buffer on top of the shared mesh vertex
/// buffer, so the wireframe overlay does not duplicate vertex data. The pair for
/// an edge is ordered `min < max` and the list is sorted and de-duplicated, so
/// the same mesh always produces the same index buffer.
pub fn sorted_edge_indices(indices: &[[u32; 3]]) -> Vec<u32> {
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for tri in indices {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            if a == b {
                continue;
            }
            edges.push((a.min(b), a.max(b)));
        }
    }
    edges.sort_unstable();
    edges.dedup();
    let mut out = Vec::with_capacity(edges.len() * 2);
    for (a, b) in edges {
        out.push(a);
        out.push(b);
    }
    out
}

/// Accumulate a frame's vertex/triangle usage against a budget.
///
/// Returns the list of batches that fit and a single explicit over-budget
/// reason, or `None`, when a limit was crossed. The offending batch and every
/// batch after it are *not* silently dropped: the caller receives the reason and
/// can surface it. The counts are the same ones the GPU will submit.
pub fn plan_frame<'a>(
    batches: &'a [RenderBatch],
    budget: &FrameBudget,
) -> (Vec<&'a RenderBatch>, FrameUsage, Option<OverBudget>) {
    let mut usage = FrameUsage::default();
    let mut accepted = Vec::with_capacity(batches.len());
    for batch in batches {
        let vertices = batch.vertices.len();
        let triangles = if batch.topology == RenderTopology::Mesh {
            batch.triangle_count()
        } else {
            0
        };
        match budget.charge(&mut usage, vertices, triangles) {
            Ok(()) => accepted.push(batch),
            Err(exceeded) => {
                let skipped = batches.len() - accepted.len();
                return (
                    accepted,
                    usage,
                    Some(OverBudget {
                        category: exceeded.category,
                        requested: exceeded.requested,
                        limit: exceeded.limit,
                        skipped_batches: skipped,
                    }),
                );
            }
        }
    }
    (accepted, usage, None)
}

/// An explicit over-budget report produced by [`plan_frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverBudget {
    /// `"vertices"` or `"triangles"`.
    pub category: &'static str,
    pub requested: usize,
    pub limit: usize,
    /// How many batches were not submitted because the budget ran out.
    pub skipped_batches: usize,
}

// ---------------------------------------------------------------------------
// Draw order + transparency policy (F14)
// ---------------------------------------------------------------------------
//
// These are pure CPU decisions so they can be tested without a GPU. The renderer
// feeds them the same values it uploads, and the two draw passes follow the
// resulting plan. The full policy, including the evidence boundary, is written
// in `docs/render-order.md`.

/// Alpha bucket a batch falls into after clamping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaClass {
    /// `alpha >= 1`: drawn in the opaque pass, depth order controlled by the
    /// depth buffer.
    Opaque,
    /// `0 < alpha < 1`: drawn after opaque geometry, back-to-front by centroid
    /// distance so blending composites correctly.
    Transparent,
    /// `alpha <= 0`: contributes nothing; it is skipped and reported, never
    /// silently counted as a normal draw.
    Invisible,
}

/// Clamp a batch alpha into `[0, 1]`.
///
/// Policy for out-of-range and non-finite input, documented in
/// `docs/render-order.md`:
/// * `NaN` (and infinities are clamped by the same rule) is treated as fully
///   **opaque** (`1.0`) — the conservative choice: an unreadable opacity must not
///   make geometry disappear;
/// * `alpha < 0` clamps to `0` (invisible), `alpha > 1` clamps to `1` (opaque).
pub fn clamp_alpha(alpha: f32) -> f32 {
    if alpha.is_nan() {
        return 1.0;
    }
    alpha.clamp(0.0, 1.0)
}

/// Bucket a raw alpha value (it is clamped first).
pub fn classify_alpha(alpha: f32) -> AlphaClass {
    let a = clamp_alpha(alpha);
    if a <= 0.0 {
        AlphaClass::Invisible
    } else if a >= 1.0 {
        AlphaClass::Opaque
    } else {
        AlphaClass::Transparent
    }
}

/// The ordering-relevant view of one batch.
///
/// `centroid` is world-space; `draw_order` is the caller's paint order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchOrderEntry {
    pub draw_order: i64,
    pub alpha: f32,
    /// World-space centroid, used only by the transparent back-to-front sort.
    pub centroid: [f32; 3],
}

/// Indices of a batch set partitioned and ordered for submission.
///
/// All three vectors hold indices into the slice passed to [`plan_draw_order`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawOrderPlan {
    /// Opaque batches, ascending `draw_order` (stable: upload order breaks ties).
    pub opaque: Vec<usize>,
    /// Transparent (`0 < alpha < 1`) batches. Back-to-front by centroid distance
    /// from the camera when one is supplied; otherwise ascending `draw_order`.
    pub transparent: Vec<usize>,
    /// `alpha <= 0` batches: not drawn at all.
    pub invisible: Vec<usize>,
}

impl DrawOrderPlan {
    /// Number of batches that will actually be submitted.
    pub fn drawn(&self) -> usize {
        self.opaque.len() + self.transparent.len()
    }
}

/// Stable draw-order plan for a frame.
///
/// Tie-break, documented and tested:
/// 1. opaque batches sort by ascending `draw_order`;
/// 2. transparent batches sort back-to-front by descending squared distance from
///    `camera` (so the farthest batch is drawn first);
/// 3. equal keys keep the input (upload) order, because every sort is stable and
///    the input index is the final comparator term.
///
/// `camera = None` means the caller has no camera position (for example a 2D
/// frame without a depth axis): transparent batches then fall back to ascending
/// `draw_order` only. This is deterministic but **not** depth-correct; the caller
/// can detect the fallback by inspecting the returned plan and the renderer
/// records it as an explicit limitation.
pub fn plan_draw_order(entries: &[BatchOrderEntry], camera: Option<[f32; 3]>) -> DrawOrderPlan {
    let mut opaque = Vec::new();
    let mut transparent = Vec::new();
    let mut invisible = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match classify_alpha(entry.alpha) {
            AlphaClass::Opaque => opaque.push(index),
            AlphaClass::Transparent => transparent.push(index),
            AlphaClass::Invisible => invisible.push(index),
        }
    }

    // 1. Opaque: ascending draw order. `sort_by_key` is stable, so equal keys
    //    retain upload order (the input index order).
    opaque.sort_by_key(|&index| entries[index].draw_order);

    // 2. Transparent: back-to-front, then draw order, then upload index.
    match camera {
        Some(eye) => transparent.sort_by(|&a, &b| {
            let da = squared_distance(entries[a].centroid, eye);
            let db = squared_distance(entries[b].centroid, eye);
            // Descending distance: farther batches first so nearer ones blend on
            // top. `partial_cmp` falls back to `Equal` on a non-finite distance,
            // and the draw_order/index terms keep the result deterministic.
            db.partial_cmp(&da)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| entries[a].draw_order.cmp(&entries[b].draw_order))
                .then_with(|| a.cmp(&b))
        }),
        None => transparent.sort_by_key(|&index| entries[index].draw_order),
    }

    DrawOrderPlan {
        opaque,
        transparent,
        invisible,
    }
}

/// Squared Euclidean distance between two world-space points.
///
/// Squared distance is monotonic in distance, so it orders identically while
/// avoiding the square root.
pub fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

fn mat4_mul(a: &[[f64; 4]; 4], b: &[[f64; 4]; 4]) -> [[f32; 4]; 4] {
    // Column-major (WGSL): (a*b)[col][row] = sum_k a[k][row] * b[col][k].
    let mut out = [[0.0f32; 4]; 4];
    for (col, out_col) in out.iter_mut().enumerate() {
        for (row, cell) in out_col.iter_mut().enumerate() {
            let mut sum = 0.0f64;
            for k in 0..4 {
                sum += a[k][row] * b[col][k];
            }
            *cell = sum as f32;
        }
    }
    out
}

fn vector_is_usable(v: [f64; 3]) -> bool {
    v.iter().all(|c| c.is_finite()) && (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) > 1e-24
}

fn normalize3(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = ((v[0] as f64).powi(2) + (v[1] as f64).powi(2) + (v[2] as f64).powi(2)).sqrt();
    if !len.is_finite() || len < 1e-12 {
        return None;
    }
    Some([
        (v[0] as f64 / len) as f32,
        (v[1] as f64 / len) as f32,
        (v[2] as f64 / len) as f32,
    ])
}

fn point_is_finite(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn length(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: Point3) -> Point3 {
    let l = length(a);
    if l <= 0.0 {
        a
    } else {
        Point3 {
            x: a.x / l,
            y: a.y / l,
            z: a.z / l,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::{EntityId, SelectionRef};
    use cad_scene::RenderTopology;

    fn point(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn mesh_batch(mirrored: bool) -> RenderBatch {
        RenderBatch {
            local_origin: point(0.0, 0.0, 0.0),
            topology: RenderTopology::Mesh,
            vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: Vec::new(),
            indices: vec![[0, 1, 2]],
            edges: Vec::new(),
            mirrored,
            alpha: 1.0,
            color: cad_scene::DEFAULT_BATCH_COLOR,
            color_unresolved: true,
            lineweight: 0.0,
            lineweight_unresolved: true,
            sources: vec![SelectionRef {
                document: cad_domain::DocumentId(1),
                entity: EntityId(1),
                instance: Default::default(),
                sub_element: None,
            }],
            draw_order: 0,
        }
    }

    #[test]
    fn winding_flip_follows_the_mirror_flag() {
        assert!(!winding_is_flipped(false));
        assert!(winding_is_flipped(true));
        assert!(front_face_ccw(false));
        assert!(!front_face_ccw(true));
    }

    #[test]
    fn missing_normals_are_recomputed_to_plus_z() {
        let batch = mesh_batch(false);
        assert!(normals_need_repair(&batch));
        let n = repaired_normals(&batch);
        assert_eq!(n.len(), 3);
        for v in &n {
            assert!((v[2] - 1.0).abs() < 1e-6, "expected +Z, got {v:?}");
        }
    }

    #[test]
    fn consistent_finite_normals_are_kept_and_normalized() {
        let mut batch = mesh_batch(false);
        batch.normals = vec![[0.0, 0.0, 2.0], [0.0, 0.0, 2.0], [0.0, 0.0, 2.0]];
        assert!(!normals_need_repair(&batch));
        let n = repaired_normals(&batch);
        for v in &n {
            assert!((v[2] - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn non_finite_normal_triggers_repair_and_never_leaks_nan() {
        let mut batch = mesh_batch(false);
        batch.normals = vec![[f32::NAN, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]];
        assert!(normals_need_repair(&batch));
        let n = repaired_normals(&batch);
        for v in &n {
            assert!(v.iter().all(|c| c.is_finite()), "got {v:?}");
        }
    }

    #[test]
    fn degenerate_mesh_gets_fallback_normal() {
        // A zero-area triangle: recomputation yields no direction.
        let mut batch = mesh_batch(false);
        batch.vertices = vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        let n = repaired_normals(&batch);
        for v in &n {
            assert!(v.iter().all(|c| c.is_finite()));
            assert!((length(point(v[0] as f64, v[1] as f64, v[2] as f64)) - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn edge_indices_are_sorted_and_deduplicated() {
        // Two triangles sharing edge (1,2).
        let edges = sorted_edge_indices(&[[0, 1, 2], [2, 1, 3]]);
        // Unique edges: (0,1),(1,2),(0,2),(1,3),(2,3) => 10 indices.
        assert_eq!(edges.len(), 10);
        assert_eq!(edges, vec![0, 1, 0, 2, 1, 2, 1, 3, 2, 3]);
        // Deterministic for the same input.
        assert_eq!(sorted_edge_indices(&[[0, 1, 2], [2, 1, 3]]), edges);
    }

    #[test]
    fn plan_frame_accepts_within_budget() {
        let batches = vec![mesh_batch(false)];
        let budget = FrameBudget {
            max_vertices: 100,
            max_triangles: 100,
        };
        let (accepted, usage, over) = plan_frame(&batches, &budget);
        assert_eq!(accepted.len(), 1);
        assert_eq!(usage.vertices, 3);
        assert_eq!(usage.triangles, 1);
        assert!(over.is_none());
    }

    #[test]
    fn plan_frame_reports_over_budget_and_skipped_batches() {
        let batches = vec![mesh_batch(false), mesh_batch(false), mesh_batch(false)];
        let budget = FrameBudget {
            max_vertices: 4,
            max_triangles: 100,
        };
        let (accepted, usage, over) = plan_frame(&batches, &budget);
        assert_eq!(accepted.len(), 1);
        assert_eq!(usage.vertices, 3);
        let over = over.expect("expected an over-budget report");
        assert_eq!(over.category, "vertices");
        assert_eq!(over.limit, 4);
        assert_eq!(over.skipped_batches, 2);
    }

    #[test]
    fn plan_view_projects_nearer_geometry_to_smaller_depth() {
        let camera = Camera3d::plan_view(point(0.0, 0.0, 10.0), point(0.0, 0.0, 0.0));
        let vp = camera.view_projection(1.0).expect("usable camera");
        let near = apply(vp, point(0.0, 0.0, 0.0));
        let far = apply(vp, point(0.0, 0.0, -10.0));
        assert!(near[2] < far[2], "near {near:?} should be < far {far:?}");
        // Clip w is the distance along the view axis (10 and 20 here).
        assert!((near[3] - 10.0).abs() < 1e-4, "w {near:?}");
        assert!((far[3] - 20.0).abs() < 1e-4, "w {far:?}");
        // Normalized device depth of the near point is inside [0, 1].
        let ndc_near = near[2] / near[3];
        assert!((0.0..=1.0).contains(&ndc_near), "ndc {ndc_near}");
    }

    #[test]
    fn degenerate_camera_has_no_projection() {
        let camera = Camera3d::plan_view(point(0.0, 0.0, 0.0), point(0.0, 0.0, 0.0));
        assert!(!camera.is_usable(1.0));
        assert!(camera.view_projection(1.0).is_none());
        let bad_up = Camera3d {
            eye: point(0.0, 0.0, 10.0),
            target: point(0.0, 0.0, 0.0),
            up: point(0.0, 0.0, 1.0), // parallel to the view direction
            ..Camera3d::plan_view(point(0.0, 0.0, 10.0), point(0.0, 0.0, 0.0))
        };
        assert!(bad_up.view_projection(1.0).is_none());
    }

    fn apply(m: [[f32; 4]; 4], p: Point3) -> [f32; 4] {
        let v = [p.x as f32, p.y as f32, p.z as f32, 1.0];
        let mut out = [0.0f32; 4];
        for (r, cell) in out.iter_mut().enumerate() {
            *cell = m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2] + m[3][r] * v[3];
        }
        out
    }

    // --- Draw order + transparency policy (F14) ---

    fn entry(draw_order: i64, alpha: f32, x: f32) -> BatchOrderEntry {
        BatchOrderEntry {
            draw_order,
            alpha,
            centroid: [x, 0.0, 0.0],
        }
    }

    #[test]
    fn alpha_is_clamped_and_nan_is_opaque() {
        assert_eq!(clamp_alpha(0.5), 0.5);
        assert_eq!(clamp_alpha(-1.0), 0.0);
        assert_eq!(clamp_alpha(2.0), 1.0);
        // An unreadable opacity must not delete geometry: NaN is opaque.
        assert_eq!(clamp_alpha(f32::NAN), 1.0);
        assert_eq!(clamp_alpha(f32::INFINITY), 1.0);
        assert_eq!(clamp_alpha(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn alpha_classification_covers_the_three_buckets() {
        assert_eq!(classify_alpha(1.0), AlphaClass::Opaque);
        assert_eq!(classify_alpha(2.0), AlphaClass::Opaque);
        assert_eq!(classify_alpha(f32::NAN), AlphaClass::Opaque);
        assert_eq!(classify_alpha(0.25), AlphaClass::Transparent);
        assert_eq!(classify_alpha(0.0), AlphaClass::Invisible);
        assert_eq!(classify_alpha(-0.5), AlphaClass::Invisible);
    }

    #[test]
    fn opaque_batches_sort_by_draw_order_and_keep_upload_order_on_ties() {
        // Draw orders: [5, 1, 1, 3]; the two `1`s must keep upload order (1 before 2).
        let entries = [
            entry(5, 1.0, 0.0),
            entry(1, 1.0, 0.0),
            entry(1, 1.0, 0.0),
            entry(3, 1.0, 0.0),
        ];
        let plan = plan_draw_order(&entries, None);
        assert_eq!(plan.opaque, vec![1, 2, 3, 0]);
        assert!(plan.transparent.is_empty());
        assert!(plan.invisible.is_empty());
        assert_eq!(plan.drawn(), 4);
    }

    #[test]
    fn transparent_batches_are_back_to_front_by_camera_distance() {
        // Camera at x = 0; nearer batch at x = 1, farther at x = 10.
        let entries = [entry(0, 0.5, 1.0), entry(0, 0.5, 10.0), entry(0, 0.5, 5.0)];
        let plan = plan_draw_order(&entries, Some([0.0, 0.0, 0.0]));
        // Farthest first: index 1 (x=10), then 2 (x=5), then 0 (x=1).
        assert_eq!(plan.transparent, vec![1, 2, 0]);
    }

    #[test]
    fn transparent_distance_ties_fall_back_to_draw_order_then_index() {
        // Same distance; draw_order decides, then upload index.
        let entries = [entry(9, 0.5, 3.0), entry(1, 0.5, 3.0), entry(1, 0.5, 3.0)];
        let plan = plan_draw_order(&entries, Some([0.0, 0.0, 0.0]));
        assert_eq!(plan.transparent, vec![1, 2, 0]);
    }

    #[test]
    fn without_a_camera_transparent_batches_use_draw_order_only() {
        let entries = [entry(4, 0.5, 1.0), entry(2, 0.5, 99.0)];
        let plan = plan_draw_order(&entries, None);
        // Distance is ignored; ascending draw order, stable.
        assert_eq!(plan.transparent, vec![1, 0]);
    }

    #[test]
    fn zero_alpha_batches_are_partitioned_out_not_drawn() {
        let entries = [entry(0, 1.0, 0.0), entry(0, 0.0, 0.0), entry(0, 0.4, 0.0)];
        let plan = plan_draw_order(&entries, Some([0.0, 0.0, 0.0]));
        assert_eq!(plan.opaque, vec![0]);
        assert_eq!(plan.transparent, vec![2]);
        assert_eq!(plan.invisible, vec![1]);
        assert_eq!(plan.drawn(), 2);
    }

    #[test]
    fn opaque_always_precedes_transparent_regardless_of_draw_order() {
        // A transparent batch with a huge draw_order must still follow opaque.
        let entries = [entry(1_000_000, 0.5, 0.0), entry(0, 1.0, 0.0)];
        let plan = plan_draw_order(&entries, Some([0.0, 0.0, 0.0]));
        assert_eq!(plan.opaque, vec![1]);
        assert_eq!(plan.transparent, vec![0]);
    }

    #[test]
    fn plan_is_deterministic_for_the_same_input() {
        let entries = [
            entry(5, 0.5, 2.0),
            entry(1, 1.0, 0.0),
            entry(0, 0.0, 0.0),
            entry(5, 0.5, 8.0),
        ];
        let a = plan_draw_order(&entries, Some([1.0, 2.0, 3.0]));
        let b = plan_draw_order(&entries, Some([1.0, 2.0, 3.0]));
        assert_eq!(a, b);
        // Sanity: the partition covers every input exactly once.
        let mut all: Vec<usize> = a
            .opaque
            .iter()
            .chain(a.transparent.iter())
            .chain(a.invisible.iter())
            .copied()
            .collect();
        all.sort_unstable();
        assert_eq!(all, vec![0, 1, 2, 3]);
    }

    #[test]
    fn moving_camera_changes_the_transparent_order() {
        let entries = [entry(0, 0.5, -10.0), entry(0, 0.5, 10.0)];
        let from_left = plan_draw_order(&entries, Some([-100.0, 0.0, 0.0]));
        let from_right = plan_draw_order(&entries, Some([100.0, 0.0, 0.0]));
        // From the left, x=10 is farther; from the right, x=-10 is farther.
        assert_eq!(from_left.transparent, vec![1, 0]);
        assert_eq!(from_right.transparent, vec![0, 1]);
    }
}
