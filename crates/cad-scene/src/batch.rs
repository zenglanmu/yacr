//! batch module.

use super::*;
use std::sync::Arc;

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
    /// Optional per-vertex sRGB colours in `[0, 1]`, one entry per `vertices`
    /// entry. Empty (the default) means "use the batch `color`", so every
    /// existing solid mesh is unchanged. Non-empty is the gradient HATCH path:
    /// the shader modulates the batch colour by this value, and the scene
    /// sanitises each channel with [`sanitize_color`].
    pub colors: Vec<[f32; 3]>,
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
    /// entity/layer/block (see `docs/render-order.md`). The renderer clamps and
    /// classifies it (see `cad-render-wgpu::geometry::classify_alpha`).
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
    /// Resolved linetype dash pattern carried for diagnostics. An empty
    /// `elements` list means continuous. Line geometry is already subdivided
    /// into dash sub-polylines in `cad-representation`, so the renderer does
    /// not consume this field; it is recorded so a host can explain why a batch
    /// is dashed (and so an unresolved `ByLayer`/`ByBlock` is visible).
    pub linetype: cad_db::LinetypePattern,
    /// `true` when the source linetype was symbolic and unresolved, so the
    /// batch is drawn continuous.
    pub linetype_unresolved: bool,
    pub sources: Vec<SelectionRef>,
    /// Paint order relative to sibling batches: larger values are drawn later
    /// (on top). The renderer performs a stable sort on this key, so batches with
    /// equal `draw_order` keep the upload order as the final tie-break.
    ///
    /// NOTE: `draw_order` is **not yet threaded** from the importer's
    /// `DbEntity::draw_order` through `DisplayFragment` into `RenderBatch`; that
    /// plumbing is formally deferred (see `docs/handoff.md`). The scene builder
    /// therefore always emits `0`, so the current effective paint order is the
    /// upload order. This field exists so the threading can land without an API
    /// change, not as a signal that it is wired up.
    pub draw_order: i64,
}

impl RenderBatch {
    pub fn triangle_count(&self) -> usize {
        if self.topology != RenderTopology::Mesh {
            0
        } else if !self.indices.is_empty() {
            self.indices.len()
        } else {
            self.vertices.len() / 3
        }
    }

    pub fn approx_bytes(&self) -> usize {
        (self.vertices.len() + self.normals.len() + self.edges.len() + self.colors.len()) * 12
            + self.indices.len() * 12
    }

    /// The exact packed GPU upload size of this batch, in bytes.
    ///
    /// This mirrors `cad-render-wgpu::Renderer::upload`: positions and edge
    /// positions are 12 bytes per vertex; a mesh **always** uploads one normal
    /// per vertex (repaired when the scene's normals are missing or wrong), so
    /// that normal buffer is `vertices.len()` vectors even when
    /// [`RenderBatch::normals`] is empty; triangle indices are `[u32; 3]`; and
    /// for a mesh the wireframe index buffer is the de-duplicated edge list as
    /// `u32`. [`SceneBudget::upload_bytes_per_frame`] is charged this amount.
    pub fn upload_size_bytes(&self) -> usize {
        let positions = self.vertices.len() * 12;
        let edges = self.edges.len() * 12;
        let normals = if self.topology == RenderTopology::Mesh {
            self.vertices.len() * 12
        } else {
            self.normals.len() * 12
        };
        let triangle_index_bytes = self.indices.len() * 12;
        let edge_index_bytes = if self.topology == RenderTopology::Mesh {
            edge_index_count(&self.indices) * 4
        } else {
            0
        };
        // A mesh always binds a per-vertex colour attribute (white when the
        // batch carries no gradient), so its 12 bytes/vertex are uploaded too.
        let vertex_color_bytes = if self.topology == RenderTopology::Mesh {
            self.vertices.len() * 12
        } else {
            0
        };
        positions + normals + edges + triangle_index_bytes + edge_index_bytes + vertex_color_bytes
    }

    /// Vertex-space bytes: positions, normals and edge positions (12 bytes each).
    pub fn vertex_bytes(&self) -> usize {
        (self.vertices.len() + self.normals.len() + self.edges.len()) * 12
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

/// Number of `u32` indices the wireframe edge buffer will contain for a mesh.
///
/// The renderer builds the edge buffer from the triangle topology, not from the
/// duplicated edge positions: `cad-render-wgpu::geometry::sorted_edge_indices`
/// emits two indices per distinct undirected edge. This computes the exact same
/// count from `indices` so [`SceneBudget::upload_bytes_per_frame`] matches what
/// is uploaded. Only the *count* is needed, so this does not allocate.
pub fn edge_index_count(indices: &[[u32; 3]]) -> usize {
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    for tri in indices {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            if a != b {
                pairs.push((a.min(b), a.max(b)));
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs.len() * 2
}

/// Sanitise a raw alpha value from a display fragment into `[0, 1]`.///
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

/// A GPU-ready image draw item, carried separately from [`RenderBatch`].
///
/// Raster images are not triangle/line geometry: the renderer must bind a
/// texture sampled from `resource` and draw `transform`'s unit square (or the
/// `clip` polygon) rather than upload vertices. The scene therefore carries
/// them in their own list so the renderer can upload textures in a later lane
/// without a placeholder geometry batch faking a draw.
///
/// The fields mirror `DisplayPrimitive::Image`: `transform` maps the texture
/// unit square `[0, 1]^2` to world space and `clip` is the surviving convex
/// polygon (with per-vertex UVs) once a viewport clipped the image. `alpha` is
/// the fragment's sanitised opacity and `sources` is the picking reference, so
/// a change invalidates the image exactly like a chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageBatch {
    pub resource: ResourceKey,
    pub transform: Transform3,
    pub clip: Option<Arc<[ImageVertex]>>,
    pub alpha: f32,
    pub draw_order: i64,
    pub sources: Vec<SelectionRef>,
}

impl ImageBatch {
    /// CPU bytes this draw item holds in the scene cache.
    ///
    /// The transform is a fixed 16-`f64` matrix; the resource key and selection
    /// sources are small strings/ids; the clip polygon is the only
    /// size-dependent part (one [`ImageVertex`] per vertex).
    pub fn approx_bytes(&self) -> usize {
        let clip_bytes = self
            .clip
            .as_ref()
            .map_or(0, |clip| clip.len() * std::mem::size_of::<ImageVertex>());
        std::mem::size_of::<Transform3>()
            + self.resource.as_str().len()
            + clip_bytes
            + self.sources.len() * std::mem::size_of::<SelectionRef>()
    }
}
