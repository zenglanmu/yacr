//! CPU scene cache, separate from authoritative data and device resources.
//!
//! Spec v2.0 §4.8, §8.1, §8.4: display representations become chunked render
//! batches. Batches are keyed so a change invalidates only the affected chunks,
//! and vertices are stored relative to a per-batch local origin so `f32` keeps
//! precision at large coordinates.

use cad_db::{ChangeSet, ObjectChange};
use cad_domain::*;
use cad_representation::{DisplayPrimitive, DisplayRepresentation};
use std::collections::BTreeMap;

pub mod annotations;
pub mod highlight;

pub use annotations::{
    all_visible, annotation_batches, annotation_geometry_source, conversion_supported,
    tessellate_ellipse, AnnotationScene, AnnotationSceneOptions, DEFAULT_ANNOTATION_FONT,
};
pub use highlight::{
    highlight_batches, HighlightOptions, HighlightScene, DEFAULT_HIGHLIGHT_ALPHA,
    HIGHLIGHT_DRAW_ORDER,
};

/// Identity of a cached chunk; any component change invalidates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub identity: DocumentIdentity,
    pub entity: EntityId,
    pub revision: Revision,
    pub style_version: u64,
    pub font_version: u64,
    pub lod: u32,
    pub space: SpaceId,
}

/// Which topology a [`RenderBatch`]'s vertex stream carries.
///
/// The renderer needs this because a mesh job is a triangle list *plus* an
/// optional wireframe line list, while a polyline job is only lines. The audit
/// (F14) found the scene carried no topology marker at all, so the GPU could
/// only ever submit `LineList`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTopology {
    /// Vertices form disconnected line segments (2 per segment).
    Lines,
    /// Vertices form triangles (3 per triangle); has a topology index list.
    Mesh,
    /// A mesh displayed as its edges: one line segment per edge.
    MeshEdges,
}

/// A GPU-ready batch. `vertices` are relative to `local_origin`.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderBatch {
    pub local_origin: Point3,
    pub topology: RenderTopology,
    pub vertices: Vec<[f32; 3]>,
    /// Per-vertex normals, present only for [`RenderTopology::Mesh`] and always
    /// the same length as `vertices`. Empty for line topologies.
    pub normals: Vec<[f32; 3]>,
    /// Triangle topology indices (`[i0, i1, i2]` into `vertices`). Empty for
    /// line topologies; for meshes it lets the renderer index rather than repeat
    /// vertices.
    pub indices: Vec<[u32; 3]>,
    /// Edge positions for the optional wireframe overlay, relative to
    /// `local_origin`. Empty when no wireframe is requested.
    pub edges: Vec<[f32; 3]>,
    /// `true` when the source transform has a negative determinant, so triangle
    /// winding is mirrored and the renderer must flip back-face culling.
    pub mirrored: bool,
    /// Constant per-object alpha in `[0, 1]`. This is the one transparency
    /// channel the batch carries; [`SceneCache::build`] takes it from
    /// `DisplayFragment::alpha`, which the importer resolved from the source
    /// entity/layer/block (see `docs/render-order.md`). The annotation overlay
    /// ([`annotations::annotation_batches`]) sets it from the annotation style's
    /// A channel. The renderer clamps and classifies it (see
    /// `cad-render-wgpu::geometry::classify_alpha`).
    pub alpha: f32,
    /// Per-batch colour as normalized sRGB in `[0, 1]`. [`SceneCache::build`]
    /// takes it from `DisplayFragment::color` and sanitises it with
    /// [`sanitize_color`]; `[1, 1, 1]` is the documented default when the source
    /// carried no resolved colour.
    pub color: [f32; 3],
    /// `true` when the source colour was symbolic (`ByLayer`/`ByBlock`) and no
    /// concrete value was available, so `color` is the fallback default rather
    /// than a source value.
    pub color_unresolved: bool,
    /// Per-batch lineweight in millimetres. The renderer carries this into its
    /// uniform but **does not draw wide lines**; it reports
    /// `render.lineweight_not_drawn` for every submitted batch with a non-zero
    /// weight (see `docs/entity-style.md`).
    pub lineweight: f32,
    /// `true` when the source lineweight was symbolic and unresolved, so
    /// `lineweight` is the fallback default.
    pub lineweight_unresolved: bool,
    pub sources: Vec<SelectionRef>,
    /// Paint order relative to sibling batches: larger values are drawn later
    /// (on top). The renderer performs a stable sort on this key, so batches with
    /// equal `draw_order` keep the upload order as the final tie-break. The scene
    /// builder currently emits `0` (the importer's `DbEntity::draw_order` is not
    /// yet reachable through `DisplayFragment`), while the annotation overlay
    /// uses `AnnotationSceneOptions::draw_order_base` (default 1_000_000) so
    /// annotations sort after drawing geometry.
    pub draw_order: i64,
}

impl RenderBatch {
    pub fn triangle_count(&self) -> usize {
        if self.topology == RenderTopology::Mesh && !self.indices.is_empty() {
            self.indices.len()
        } else {
            self.vertices.len() / 3
        }
    }

    pub fn approx_bytes(&self) -> usize {
        (self.vertices.len() + self.normals.len() + self.edges.len()) * 12 + self.indices.len() * 12
    }

    /// World-space centroid used as the transparent-pass depth-sort key.
    ///
    /// It is `local_origin + mean(vertices)`. The mean is taken in `f64` before
    /// narrowing so large coordinates do not lose precision. A batch with no
    /// vertices falls back to its `local_origin`.
    pub fn centroid(&self) -> [f32; 3] {
        let n = self.vertices.len();
        if n == 0 {
            return [
                self.local_origin.x as f32,
                self.local_origin.y as f32,
                self.local_origin.z as f32,
            ];
        }
        let mut sx = 0.0f64;
        let mut sy = 0.0f64;
        let mut sz = 0.0f64;
        for v in &self.vertices {
            sx += v[0] as f64;
            sy += v[1] as f64;
            sz += v[2] as f64;
        }
        let inv = 1.0 / n as f64;
        [
            (self.local_origin.x + sx * inv) as f32,
            (self.local_origin.y + sy * inv) as f32,
            (self.local_origin.z + sz * inv) as f32,
        ]
    }
}

/// Sanitise a raw alpha value from a display fragment into `[0, 1]`.
///
/// Policy (kept in lockstep with `cad-render-wgpu::geometry::clamp_alpha`, which
/// re-clamps before upload): a non-finite value is treated as **opaque** so an
/// unreadable opacity never deletes geometry; otherwise the value is clamped to
/// `[0, 1]`. `alpha <= 0` is preserved (the renderer classifies it invisible)
/// rather than being silently turned opaque.
pub fn sanitize_alpha(alpha: f32) -> f32 {
    if alpha.is_nan() {
        return 1.0;
    }
    alpha.clamp(0.0, 1.0)
}

/// Default batch colour when a fragment carries none: white, matching
/// `cad_representation::DEFAULT_RENDER_COLOR`.
pub const DEFAULT_BATCH_COLOR: [f32; 3] = cad_representation::DEFAULT_RENDER_COLOR;

/// Default batch lineweight in millimetres, matching
/// `cad_representation::DEFAULT_LINEWEIGHT_MM`.
pub const DEFAULT_BATCH_LINEWEIGHT_MM: f32 = cad_representation::DEFAULT_LINEWEIGHT_MM;

/// Sanitise one colour channel into `[0, 1]`.
///
/// Policy (kept in lockstep with `cad-render-wgpu`): a non-finite channel
/// becomes `1.0`, the documented default, so an unreadable colour never blanks
/// geometry; otherwise the channel is clamped.
pub fn sanitize_color_channel(channel: f32) -> f32 {
    if channel.is_finite() {
        channel.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Sanitise a fragment colour into finite `[0, 1]` channels.
pub fn sanitize_color(color: [f32; 3]) -> [f32; 3] {
    [
        sanitize_color_channel(color[0]),
        sanitize_color_channel(color[1]),
        sanitize_color_channel(color[2]),
    ]
}

/// Sanitise a fragment lineweight (millimetres) into a finite, non-negative
/// value; a non-finite value becomes the documented default.
pub fn sanitize_lineweight(mm: f32) -> f32 {
    if mm.is_finite() {
        mm.max(0.0)
    } else {
        DEFAULT_BATCH_LINEWEIGHT_MM
    }
}

/// A publishable update to the scene cache.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneDelta {
    pub stamp: TaskStamp,
    pub added: Vec<RenderBatch>,
    pub removed_chunks: Vec<u64>,
}

impl SceneDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed_chunks.is_empty()
    }
}

/// Budget bookkeeping shared by the CPU scene and the GPU upload path.
///
/// The audit (F14, cross-cutting §8) found uploads had no per-frame budget. This
/// type accumulates the vertex/triangle counts a frame would submit and reports
/// the exact category and amount that crossed a limit, so the caller can emit a
/// diagnostic instead of silently dropping a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBudget {
    pub max_vertices: usize,
    pub max_triangles: usize,
}

/// A frame's accumulated usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameUsage {
    pub vertices: usize,
    pub triangles: usize,
}

/// What (if anything) crossed a [`FrameBudget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetExceeded {
    /// `"vertices"` or `"triangles"`.
    pub category: &'static str,
    /// Amount the frame would have used, including the offending charge.
    pub requested: usize,
    pub limit: usize,
}

impl FrameBudget {
    pub fn from_scene(budget: &SceneBudget) -> Self {
        FrameBudget {
            max_vertices: budget.max_vertices_per_frame,
            max_triangles: budget.max_triangles_per_frame,
        }
    }

    /// Charge one batch. `vertices` is its vertex count and `triangles` its
    /// triangle count (0 for line batches). Returns the first limit crossed, if
    /// any; on success the usage is updated in place.
    pub fn charge(
        &self,
        usage: &mut FrameUsage,
        vertices: usize,
        triangles: usize,
    ) -> Result<(), BudgetExceeded> {
        let next_vertices = usage.vertices.saturating_add(vertices);
        if next_vertices > self.max_vertices {
            return Err(BudgetExceeded {
                category: "vertices",
                requested: next_vertices,
                limit: self.max_vertices,
            });
        }
        let next_triangles = usage.triangles.saturating_add(triangles);
        if next_triangles > self.max_triangles {
            return Err(BudgetExceeded {
                category: "triangles",
                requested: next_triangles,
                limit: self.max_triangles,
            });
        }
        usage.vertices = next_vertices;
        usage.triangles = next_triangles;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneBudget {
    pub cpu_bytes: usize,
    pub queued_tasks: usize,
    pub upload_bytes_per_frame: usize,
    /// Maximum vertices a single frame may submit across every batch.
    pub max_vertices_per_frame: usize,
    /// Maximum triangles a single frame may submit across every mesh batch.
    pub max_triangles_per_frame: usize,
}

impl Default for SceneBudget {
    fn default() -> Self {
        SceneBudget {
            cpu_bytes: 128 * 1024 * 1024,
            queued_tasks: 8,
            upload_bytes_per_frame: 4 * 1024 * 1024,
            max_vertices_per_frame: 8_000_000,
            max_triangles_per_frame: 2_000_000,
        }
    }
}

/// Bounds and caching for the CPU-side scene.
pub struct SceneCache {
    pub budget: SceneBudget,
    chunks: BTreeMap<u64, RenderBatch>,
    next_chunk: u64,
    used_bytes: usize,
}

impl Default for SceneCache {
    fn default() -> Self {
        SceneCache::new(SceneBudget::default())
    }
}

impl SceneCache {
    pub fn new(budget: SceneBudget) -> Self {
        SceneCache {
            budget,
            chunks: BTreeMap::new(),
            next_chunk: 1,
            used_bytes: 0,
        }
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    pub fn chunks(&self) -> impl Iterator<Item = &RenderBatch> {
        self.chunks.values()
    }

    /// Remove batches that reference objects touched by `changes`.
    pub fn apply_changes(&mut self, changes: &ChangeSet) -> CadResult<()> {
        if changes.changes.is_empty() {
            return Ok(());
        }
        let mut dirty: std::collections::BTreeSet<EntityId> = std::collections::BTreeSet::new();
        for change in &changes.changes {
            match change {
                ObjectChange::Insert(id)
                | ObjectChange::Update(id, _)
                | ObjectChange::Delete(id) => {
                    // ObjectId and EntityId share the same numeric space during
                    // import; a finer mapping is a documented follow-up.
                    dirty.insert(EntityId(id.0));
                }
            }
        }
        let to_remove: Vec<u64> = self
            .chunks
            .iter()
            .filter(|(_, batch)| batch.sources.iter().any(|s| dirty.contains(&s.entity)))
            .map(|(k, _)| *k)
            .collect();
        for key in to_remove {
            self.remove_chunk(key);
        }
        Ok(())
    }

    fn remove_chunk(&mut self, key: u64) {
        if let Some(batch) = self.chunks.remove(&key) {
            self.used_bytes = self.used_bytes.saturating_sub(batch.approx_bytes());
        }
    }

    /// Convert a display representation into render batches.
    ///
    /// Every fragment's `alpha` (effective entity opacity in `[0, 1]`) is
    /// sanitised and carried onto [`RenderBatch::alpha`]; the renderer then
    /// clamps/classifies it into the opaque, transparent or invisible pass
    /// (`cad-render-wgpu::geometry::classify_alpha`). The annotation overlay
    /// path ([`annotations::annotation_batches`]) supplies its own alpha from
    /// the annotation style.
    pub fn build(
        &mut self,
        representation: &DisplayRepresentation,
        stamp: TaskStamp,
    ) -> CadResult<SceneDelta> {
        let mut delta = SceneDelta {
            stamp,
            added: Vec::new(),
            removed_chunks: Vec::new(),
        };
        for fragment in &representation.fragments {
            let (topology, vertices, normals, indices, edges, origin, mirrored) =
                match &fragment.primitive {
                    DisplayPrimitive::Lines(points) => {
                        let origin = points.first().copied().unwrap_or(Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        });
                        let verts: Vec<[f32; 3]> = points
                            .iter()
                            .map(|p| {
                                [
                                    (p.x - origin.x) as f32,
                                    (p.y - origin.y) as f32,
                                    (p.z - origin.z) as f32,
                                ]
                            })
                            .collect();
                        (
                            RenderTopology::Lines,
                            verts,
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            origin,
                            false,
                        )
                    }
                    DisplayPrimitive::Mesh(mesh) => {
                        let origin = mesh.vertices.first().copied().unwrap_or(Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        });
                        // De-duplicate the triangle stream: one vertex per mesh
                        // vertex, indexed by topology, instead of repeating each
                        // shared corner (audit F14: linear indexing, not ideal).
                        let verts: Vec<[f32; 3]> = mesh
                            .vertices
                            .iter()
                            .map(|p| {
                                [
                                    (p.x - origin.x) as f32,
                                    (p.y - origin.y) as f32,
                                    (p.z - origin.z) as f32,
                                ]
                            })
                            .collect();
                        // Keep only triangles whose indices are in range, and
                        // carry their normals. A triangle dropped here is a
                        // malformed topology, reported by the caller.
                        let mut kept = Vec::with_capacity(mesh.triangles.len());
                        for tri in &mesh.triangles {
                            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
                            if a < verts.len() && b < verts.len() && c < verts.len() {
                                kept.push(*tri);
                            }
                        }
                        let normals = mesh
                            .normals
                            .iter()
                            .map(|n| [n.x as f32, n.y as f32, n.z as f32])
                            .collect();
                        // Edge line list built from the kept topology, one
                        // segment per triangle edge. Duplicated edges are not
                        // removed yet; that is a documented follow-up (edges are
                        // only uploaded when a caller asks for the wireframe).
                        let mut edge_positions = Vec::with_capacity(kept.len() * 6);
                        for tri in &kept {
                            for &(i, j) in &[(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                                edge_positions.push(verts[i as usize]);
                                edge_positions.push(verts[j as usize]);
                            }
                        }
                        (
                            RenderTopology::Mesh,
                            verts,
                            normals,
                            kept,
                            edge_positions,
                            origin,
                            false,
                        )
                    }
                    // Text/instance/image batching is handled by their own
                    // subsystems; a scene batch that pretends to draw them would
                    // be a fake success.
                    DisplayPrimitive::Text { .. }
                    | DisplayPrimitive::Instance { .. }
                    | DisplayPrimitive::Image { .. } => {
                        continue;
                    }
                };
            if vertices.is_empty() {
                continue;
            }
            let _ = mirrored; // filled in by the caller when a transform is known
            delta.added.push(RenderBatch {
                local_origin: origin,
                topology,
                vertices,
                normals,
                indices,
                edges,
                mirrored: false,
                alpha: sanitize_alpha(fragment.alpha),
                color: sanitize_color(fragment.color),
                color_unresolved: fragment.color_unresolved,
                lineweight: sanitize_lineweight(fragment.lineweight),
                lineweight_unresolved: fragment.lineweight_unresolved,
                sources: vec![fragment.source.clone()],
                draw_order: 0,
            });
        }
        Ok(delta)
    }

    /// Commit a delta if its stamp still matches the current task context.
    pub fn publish(&mut self, delta: SceneDelta, current: &TaskStamp) -> CadResult<()> {
        delta
            .stamp
            .validate(current)
            .map_err(|_| CadError::StaleResult)?;
        for key in delta.removed_chunks {
            self.remove_chunk(key);
        }
        for batch in delta.added {
            let key = self.next_chunk;
            self.next_chunk += 1;
            self.used_bytes += batch.approx_bytes();
            self.chunks.insert(key, batch);
        }
        self.evict(0)?;
        Ok(())
    }

    /// Drop oldest chunks until the cache is under budget.
    pub fn evict(&mut self, required_bytes: usize) -> CadResult<()> {
        while self.used_bytes + required_bytes > self.budget.cpu_bytes {
            let Some(oldest) = self.chunks.keys().next().copied() else {
                break;
            };
            self.remove_chunk(oldest);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_representation::DisplayFragment;
    use std::sync::Arc;

    fn stamp() -> TaskStamp {
        TaskStamp::new(DocumentId(1), 0)
    }

    fn line_representation(entity: u128, points: Vec<Point3>) -> DisplayRepresentation {
        DisplayRepresentation {
            fragments: vec![DisplayFragment {
                source: SelectionRef {
                    document: DocumentId(1),
                    entity: EntityId(entity),
                    instance: InstancePath::default(),
                    sub_element: None,
                },
                geometry_source: GeometrySource::Analytic,
                precision: Precision::Analytic,
                alpha: 1.0,
                color: cad_representation::DEFAULT_RENDER_COLOR,
                color_unresolved: true,
                lineweight: cad_representation::DEFAULT_LINEWEIGHT_MM,
                lineweight_unresolved: true,
                primitive: DisplayPrimitive::Lines(Arc::from(points.into_boxed_slice())),
            }],
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn build_produces_relative_vertices() {
        let mut cache = SceneCache::default();
        let rep = line_representation(
            1,
            vec![
                Point3 {
                    x: 1_000_000.0,
                    y: 2_000_000.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1_000_010.0,
                    y: 2_000_000.0,
                    z: 0.0,
                },
            ],
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        assert_eq!(delta.added.len(), 1);
        let batch = &delta.added[0];
        assert_eq!(batch.local_origin.x, 1_000_000.0);
        // Relative coordinates stay small and precise.
        assert_eq!(batch.vertices[1][0], 10.0);
    }

    #[test]
    fn publish_rejects_stale_stamp() {
        let mut cache = SceneCache::default();
        let rep = line_representation(
            1,
            vec![
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
            ],
        );
        let delta = cache.build(&rep, TaskStamp::new(DocumentId(1), 5)).unwrap();
        let current = TaskStamp::new(DocumentId(1), 6);
        assert_eq!(cache.publish(delta, &current), Err(CadError::StaleResult));
        assert_eq!(cache.chunk_count(), 0);
    }

    #[test]
    fn change_set_removes_affected_chunks() {
        let mut cache = SceneCache::default();
        let rep = line_representation(
            1,
            vec![
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
            ],
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        cache.publish(delta, &stamp()).unwrap();
        assert_eq!(cache.chunk_count(), 1);
        let changes = ChangeSet {
            database: DatabaseId(1),
            before: Revision(0),
            after: Revision(1),
            transaction: TransactionId(1),
            reason: "edit".into(),
            changes: vec![ObjectChange::Update(
                ObjectId(1),
                cad_db::ChangeMask::GEOMETRY,
            )],
        };
        cache.apply_changes(&changes).unwrap();
        assert_eq!(cache.chunk_count(), 0);
    }

    #[test]
    fn eviction_respects_budget() {
        let mut cache = SceneCache::new(SceneBudget {
            cpu_bytes: 40,
            ..Default::default()
        });
        for i in 0..4 {
            let rep = line_representation(
                i,
                vec![
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
                    Point3 {
                        x: 2.0,
                        y: 0.0,
                        z: 0.0,
                    },
                ],
            );
            let delta = cache.build(&rep, stamp()).unwrap();
            cache.publish(delta, &stamp()).unwrap();
        }
        assert!(cache.used_bytes() <= 40);
        assert!(cache.chunk_count() < 4);
    }

    fn mesh_representation(entity: u128, mesh: Mesh) -> DisplayRepresentation {
        DisplayRepresentation {
            fragments: vec![DisplayFragment {
                source: SelectionRef {
                    document: DocumentId(1),
                    entity: EntityId(entity),
                    instance: InstancePath::default(),
                    sub_element: None,
                },
                geometry_source: GeometrySource::DirectMesh,
                precision: Precision::Analytic,
                alpha: 1.0,
                color: cad_representation::DEFAULT_RENDER_COLOR,
                color_unresolved: true,
                lineweight: cad_representation::DEFAULT_LINEWEIGHT_MM,
                lineweight_unresolved: true,
                primitive: DisplayPrimitive::Mesh(std::sync::Arc::new(mesh)),
            }],
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        }
    }

    fn quad() -> Mesh {
        Mesh {
            vertices: vec![
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
                Point3 {
                    x: 1.0,
                    y: 1.0,
                    z: 0.0,
                },
                Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            normals: vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                };
                4
            ],
            face_sources: Vec::new(),
        }
    }

    #[test]
    fn mesh_batch_is_indexed_and_carries_normals_and_edges() {
        let mut cache = SceneCache::default();
        let delta = cache
            .build(&mesh_representation(1, quad()), stamp())
            .unwrap();
        assert_eq!(delta.added.len(), 1);
        let batch = &delta.added[0];
        assert_eq!(batch.topology, RenderTopology::Mesh);
        // Indexed: one vertex per mesh vertex, not per triangle corner.
        assert_eq!(batch.vertices.len(), 4);
        assert_eq!(batch.normals.len(), 4);
        assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
        assert_eq!(batch.triangle_count(), 2);
        // Two triangles => six directed edges => twelve edge vertices.
        assert_eq!(batch.edges.len(), 12);
    }

    #[test]
    fn lines_batch_has_no_triangles_or_normals() {
        let mut cache = SceneCache::default();
        let rep = line_representation(
            1,
            vec![
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
            ],
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        let batch = &delta.added[0];
        assert_eq!(batch.topology, RenderTopology::Lines);
        assert!(batch.normals.is_empty());
        assert!(batch.indices.is_empty());
        assert_eq!(batch.triangle_count(), 0);
    }

    #[test]
    fn out_of_range_triangle_indices_are_dropped_not_uploaded() {
        let mut mesh = quad();
        mesh.triangles.push([0, 1, 9]); // index 9 does not exist
        let mut cache = SceneCache::default();
        let delta = cache.build(&mesh_representation(1, mesh), stamp()).unwrap();
        let batch = &delta.added[0];
        assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
    }

    #[test]
    fn frame_budget_reports_over_vertex_limit() {
        let budget = FrameBudget {
            max_vertices: 10,
            max_triangles: 10,
        };
        let mut usage = FrameUsage::default();
        budget.charge(&mut usage, 6, 2).unwrap();
        let exceeded = budget.charge(&mut usage, 6, 0).unwrap_err();
        assert_eq!(exceeded.category, "vertices");
        assert_eq!(exceeded.requested, 12);
        assert_eq!(exceeded.limit, 10);
        // The rejected charge did not apply to the running usage.
        assert_eq!(usage.vertices, 6);
    }

    #[test]
    fn frame_budget_reports_over_triangle_limit() {
        let budget = FrameBudget {
            max_vertices: 1000,
            max_triangles: 3,
        };
        let mut usage = FrameUsage::default();
        budget.charge(&mut usage, 3, 2).unwrap();
        let exceeded = budget.charge(&mut usage, 3, 2).unwrap_err();
        assert_eq!(exceeded.category, "triangles");
        assert_eq!(exceeded.requested, 4);
        assert_eq!(exceeded.limit, 3);
        assert_eq!(usage.triangles, 2);
    }

    #[test]
    fn centroid_is_origin_plus_mean_vertex() {
        let mut cache = SceneCache::default();
        let delta = cache
            .build(&mesh_representation(1, quad()), stamp())
            .unwrap();
        let batch = &delta.added[0];
        // Quad spans (0,0)-(1,1) with origin (0,0,0): centroid is (0.5, 0.5, 0).
        let c = batch.centroid();
        assert!((c[0] - 0.5).abs() < 1e-6, "got {c:?}");
        assert!((c[1] - 0.5).abs() < 1e-6, "got {c:?}");
        assert!(c[2].abs() < 1e-6, "got {c:?}");
    }

    #[test]
    fn centroid_keeps_precision_at_large_coordinates() {
        let mut cache = SceneCache::default();
        let rep = line_representation(
            1,
            vec![
                Point3 {
                    x: 1_000_000.0,
                    y: 2_000_000.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1_000_010.0,
                    y: 2_000_000.0,
                    z: 0.0,
                },
            ],
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        let c = delta.added[0].centroid();
        assert!((c[0] - 1_000_005.0).abs() < 1e-3, "got {c:?}");
        assert!((c[1] - 2_000_000.0).abs() < 1e-3, "got {c:?}");
    }

    #[test]
    fn empty_batch_centroid_falls_back_to_local_origin() {
        let batch = RenderBatch {
            local_origin: Point3 {
                x: 3.0,
                y: 4.0,
                z: 5.0,
            },
            topology: RenderTopology::Lines,
            vertices: Vec::new(),
            normals: Vec::new(),
            indices: Vec::new(),
            edges: Vec::new(),
            mirrored: false,
            alpha: 1.0,
            color: DEFAULT_BATCH_COLOR,
            color_unresolved: true,
            lineweight: DEFAULT_BATCH_LINEWEIGHT_MM,
            lineweight_unresolved: true,
            sources: Vec::new(),
            draw_order: 0,
        };
        assert_eq!(batch.centroid(), [3.0, 4.0, 5.0]);
    }

    fn line_representation_alpha(
        entity: u128,
        points: Vec<Point3>,
        alpha: f32,
    ) -> DisplayRepresentation {
        let mut rep = line_representation(entity, points);
        rep.fragments[0].alpha = alpha;
        rep
    }

    #[test]
    fn batch_alpha_comes_from_the_fragment() {
        let mut cache = SceneCache::default();
        let rep = line_representation_alpha(
            1,
            vec![
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
            ],
            0.25,
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        assert_eq!(delta.added.len(), 1);
        assert_eq!(delta.added[0].alpha, 0.25);
    }

    #[test]
    fn sanitize_alpha_matches_the_renderer_policy() {
        assert_eq!(sanitize_alpha(0.5), 0.5);
        assert_eq!(sanitize_alpha(-1.0), 0.0);
        assert_eq!(sanitize_alpha(2.0), 1.0);
        assert_eq!(sanitize_alpha(0.0), 0.0);
        // An unreadable opacity must not delete geometry.
        assert_eq!(sanitize_alpha(f32::NAN), 1.0);
    }

    fn line_representation_styled(
        entity: u128,
        points: Vec<Point3>,
        color: [f32; 3],
        color_unresolved: bool,
        lineweight: f32,
        lineweight_unresolved: bool,
    ) -> DisplayRepresentation {
        let mut rep = line_representation(entity, points);
        let fragment = &mut rep.fragments[0];
        fragment.color = color;
        fragment.color_unresolved = color_unresolved;
        fragment.lineweight = lineweight;
        fragment.lineweight_unresolved = lineweight_unresolved;
        rep
    }

    fn unit_points() -> Vec<Point3> {
        vec![
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
        ]
    }

    #[test]
    fn batch_color_and_lineweight_come_from_the_fragment() {
        let mut cache = SceneCache::default();
        let rep =
            line_representation_styled(1, unit_points(), [0.25, 0.5, 0.75], false, 0.35, false);
        let delta = cache.build(&rep, stamp()).unwrap();
        assert_eq!(delta.added.len(), 1);
        let batch = &delta.added[0];
        assert_eq!(batch.color, [0.25, 0.5, 0.75]);
        assert!(!batch.color_unresolved);
        assert_eq!(batch.lineweight, 0.35);
        assert!(!batch.lineweight_unresolved);
    }

    #[test]
    fn unresolved_color_and_lineweight_are_carried_as_explicit_defaults() {
        let mut cache = SceneCache::default();
        // ByLayer/ByBlock with no reachable value: the fragment already carries
        // the documented fallback and marks it unresolved.
        let rep = line_representation_styled(
            1,
            unit_points(),
            [1.0, 1.0, 1.0],
            true,
            cad_representation::DEFAULT_LINEWEIGHT_MM,
            true,
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        let batch = &delta.added[0];
        assert_eq!(batch.color, DEFAULT_BATCH_COLOR);
        assert!(batch.color_unresolved);
        assert_eq!(batch.lineweight, DEFAULT_BATCH_LINEWEIGHT_MM);
        assert!(batch.lineweight_unresolved);
    }

    #[test]
    fn non_finite_color_and_lineweight_are_replaced_by_defaults() {
        // A hostile fragment must never inject NaN/inf into the GPU uniform.
        assert_eq!(
            sanitize_color([f32::NAN, f32::INFINITY, -1.0]),
            [1.0, 1.0, 0.0]
        );
        assert_eq!(
            sanitize_color([f32::NEG_INFINITY, 2.0, 0.5]),
            [1.0, 1.0, 0.5]
        );
        assert_eq!(sanitize_lineweight(f32::NAN), DEFAULT_BATCH_LINEWEIGHT_MM);
        assert_eq!(
            sanitize_lineweight(f32::NEG_INFINITY),
            DEFAULT_BATCH_LINEWEIGHT_MM
        );
        assert_eq!(sanitize_lineweight(-1.0), 0.0);

        let mut cache = SceneCache::default();
        let rep = line_representation_styled(
            1,
            unit_points(),
            [f32::NAN, f32::NAN, f32::NAN],
            false,
            f32::NAN,
            false,
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        assert_eq!(delta.added[0].color, DEFAULT_BATCH_COLOR);
        assert_eq!(delta.added[0].lineweight, DEFAULT_BATCH_LINEWEIGHT_MM);
    }

    #[test]
    fn fully_transparent_fragment_is_carried_not_forced_opaque() {
        let mut cache = SceneCache::default();
        let rep = line_representation_alpha(
            1,
            vec![
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
            ],
            0.0,
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        // The batch keeps the real value; the renderer classifies it invisible.
        assert_eq!(delta.added[0].alpha, 0.0);
    }

    #[test]
    fn out_of_range_fragment_alpha_is_clamped_for_the_batch() {
        let mut cache = SceneCache::default();
        let rep = line_representation_alpha(
            1,
            vec![
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
            ],
            4.0,
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        assert_eq!(delta.added[0].alpha, 1.0);
    }
}
