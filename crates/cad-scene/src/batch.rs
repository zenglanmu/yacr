//! batch module.

use super::*;
use std::sync::Arc;

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
    /// Analytic circles/arcs carried in [`RenderBatch::conics`] and drawn as
    /// anti-aliased SDF instances (slice B; see
    /// `docs/adr/0008-sdf-conic-rendering.md`). `vertices` is empty; the
    /// renderer packs each conic into a 72-byte `ArcInstance`.
    Conics,
}

/// One analytic circle/arc a [`RenderTopology::Conics`] batch draws as an SDF
/// instance.
///
/// This is the **f64 semantic record**; the renderer packs it into a 72-byte
/// `ArcInstance` (center reduced against the chunk origin, unit major direction,
/// minor semi-diameter vector, start angle and signed sweep). Only slice-1
/// circles/arcs reach here — an ellipse (`|major| != |minor|`) stays
/// chord-tessellated (slice 2, see `docs/adr/0008-sdf-conic-rendering.md`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderConic {
    /// Circle center in world space.
    pub center: Point3,
    /// Semi-diameter vector along `cos(t)`.
    pub major: Point3,
    /// Conjugate semi-diameter vector along `sin(t)`.
    pub minor: Point3,
    /// Parameter angle (radians) at `t = 0`.
    pub start: f64,
    /// Signed sweep (radians); negative is a clockwise arc.
    pub sweep: f64,
    /// `true` for a closed full circle.
    pub full: bool,
}

impl RenderConic {
    /// World-space AABB `(min, max)` of the full conic extent.
    ///
    /// Every point is `center + major·cos t + minor·sin t`, so per axis the
    /// extent is `center ± (|major_k| + |minor_k|)` (a superset of the arc's own
    /// extent, which is what the chunk splitter needs). `None` when any
    /// component is non-finite.
    pub fn world_aabb(&self) -> Option<([f64; 3], [f64; 3])> {
        let values = [
            self.center.x,
            self.center.y,
            self.center.z,
            self.major.x,
            self.major.y,
            self.major.z,
            self.minor.x,
            self.minor.y,
            self.minor.z,
        ];
        if !values.iter().all(|v| v.is_finite()) {
            return None;
        }
        let mut min = [0.0f64; 3];
        let mut max = [0.0f64; 3];
        let center = [self.center.x, self.center.y, self.center.z];
        let major = [self.major.x, self.major.y, self.major.z];
        let minor = [self.minor.x, self.minor.y, self.minor.z];
        for axis in 0..3 {
            let extent = major[axis].abs() + minor[axis].abs();
            min[axis] = center[axis] - extent;
            max[axis] = center[axis] + extent;
        }
        Some((min, max))
    }
}

/// The PBR channels a mesh batch draws with (ADR-0039 D6).
///
/// This is the GPU-facing form of [`cad_domain::MeshPbr`]: the `f64` domain
/// values are narrowed to `f32` and the four document texture keys become
/// [`ResourceKey`]s (exactly like [`RenderBatch::texture`]). It exists **only**
/// when the domain mesh declared PBR — `None` means "the source declared no
/// PBR", never "the default material" (a glTF material with only a base colour
/// stays on the unlit `mesh_textured` path).
///
/// Absent texture slots are **channel-disabled**: the renderer binds the exact
/// multiplicative identity for that slot (white for base colour / occlusion,
/// black for emissive, flat `(0.5, 0.5, 1)` for the normal map), so a missing
/// channel is a mathematically neutral sample, not fabricated data.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPbr {
    /// Metallic factor in `[0, 1]`.
    pub metallic: f32,
    /// Roughness factor in `[0, 1]`.
    pub roughness: f32,
    /// **Linear** RGB emissive factor, each component in `[0, 1]`.
    pub emissive: [f32; 3],
    /// Ambient-occlusion strength in `[0, 1]`.
    pub occlusion_strength: f32,
    /// Metallic-roughness map key (`g` = roughness, `b` = metallic), or `None`.
    pub metallic_roughness: Option<ResourceKey>,
    /// Tangent-space normal map key (UV0), or `None`.
    pub normal: Option<ResourceKey>,
    /// Emissive map key (sRGB), or `None`.
    pub emissive_texture: Option<ResourceKey>,
    /// Ambient-occlusion map key (`r`), or `None`.
    pub occlusion: Option<ResourceKey>,
}

impl RenderPbr {
    /// Every distinct texture key this material references (four slots, so at
    /// most four entries; slot order, not sorted).
    pub fn texture_keys(&self) -> impl Iterator<Item = &ResourceKey> {
        [
            &self.metallic_roughness,
            &self.normal,
            &self.emissive_texture,
            &self.occlusion,
        ]
        .into_iter()
        .flatten()
    }

    /// `true` when no texture slot is set (scalar factors only).
    pub fn has_textures(&self) -> bool {
        self.texture_keys().next().is_some()
    }

    /// `true` when the shader needs a per-vertex tangent frame: only the normal
    /// map perturbs a normal, so only it requires tangents (ADR-0039 D6). The
    /// renderer generates them at pack time; a batch without them draws with the
    /// normal-map channel disabled.
    pub fn needs_tangents(&self) -> bool {
        self.normal.is_some()
    }

    /// Bytes of the four texture keys' UTF-8 strings (safe relative names, so
    /// bounded and exact).
    pub fn key_bytes(&self) -> usize {
        self.texture_keys().map(|key| key.as_str().len()).sum()
    }

    /// Tangent-stream bytes for a batch of `vertex_count` vertices: one
    /// `Float32x4` per vertex, and only when the material needs tangents.
    pub fn tangent_bytes(&self, vertex_count: usize) -> usize {
        if self.needs_tangents() {
            vertex_count * TANGENT_STRIDE as usize
        } else {
            0
        }
    }

    /// CPU/upload bytes this material adds beyond the mesh geometry: the four
    /// key strings plus the tangent stream when the normal map needs one.
    pub fn approx_bytes(&self, vertex_count: usize) -> usize {
        self.key_bytes() + self.tangent_bytes(vertex_count)
    }
}

/// Bytes of one packed tangent: `Float32x4` (`xyz` = tangent, `w` = handedness).
pub const TANGENT_STRIDE: u32 = 16;

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
    /// Per-vertex UV0 in texture space, one entry per `vertices` entry.
    ///
    /// Only a batch with a sampler carries UVs — the base-colour texture
    /// ([`Self::texture`], ADR-0038 D8) or any surviving PBR map (ADR-0039 D6).
    /// An untextured, non-PBR batch keeps this empty (its UVs, if a source had
    /// any, would be unreadable by the GPU anyway). The invariant is
    /// `(texture.is_some() || pbr has a texture) ⇒ uvs.len() == vertices.len()`,
    /// and otherwise both are empty; [`SceneCache::build`] sanitises a violating
    /// mesh into the untextured form instead of panicking.
    pub uvs: Vec<[f32; 2]>,
    /// The document-owned base-colour texture key, or `None` for an untextured
    /// batch.
    ///
    /// The domain `Mesh` carries the key as a `String`; this boundary converts it
    /// to a [`ResourceKey`] (the type the renderer's decoded-image cache is keyed
    /// by). A textured batch is a per-batch draw: it is never flat-merged into a
    /// shared vertex stream and never enters the pulled mesh class, because both
    /// bind one uniform for the whole draw and cannot express a per-batch texture
    /// (ADR-0038 D8 — a documented batching regression, not a silent one).
    pub texture: Option<ResourceKey>,
    /// The declared PBR channel set, or `None` when the source declared no PBR
    /// material (ADR-0039 D6).
    ///
    /// A batch with `pbr.is_some()` is drawn through the separate `mesh_pbr`
    /// channel: it is never flat-merged into a shared vertex stream and never
    /// enters the pulled mesh class (both bake one uniform per draw and cannot
    /// express a per-batch PBR material). That is a documented batching
    /// regression, exactly like the ADR-0038 D8 texture rule.
    ///
    /// Sanitised by [`SceneCache::build`]: a non-finite or out-of-range scalar
    /// drops the whole `pbr` (there is no honest partial value); a PBR texture
    /// slot without a non-empty UV0 stream drops just that key and keeps the
    /// scalar factors.
    pub pbr: Option<RenderPbr>,
    /// Triangle topology indices (`[i0, i1, i2]` into `vertices`). Empty for
    /// line topologies; for meshes it lets the renderer index rather than repeat
    /// vertices.
    pub indices: Vec<[u32; 3]>,
    /// Edge positions for the optional wireframe overlay, relative to
    /// `local_origin`. Empty when no wireframe is requested. The list is a flat
    /// set of **independent endpoint pairs** (two consecutive positions = one
    /// segment, the `LineSegments` pairing rule), not a polyline: the renderer
    /// draws exactly these positions and never joins adjacent pairs.
    ///
    /// For a plain mesh batch the scene derives them from the triangle topology
    /// as the mesh's **surface boundary**: an edge is emitted only when the kept
    /// triangles use it an odd number of times, so a tessellation diagonal
    /// (shared by the two triangles of one face) cancels and is never drawn. For
    /// a body (`DisplayPrimitive::MeshWithStoredEdges`) it copies the body's
    /// stored edges instead. An imported PBR mesh ([`RenderBatch::pbr`] set)
    /// deliberately carries **none** (ADR-0044): its shaded surface is the draw,
    /// so even a boundary wireframe is not emitted.
    pub edges: Vec<[f32; 3]>,
    /// Whether this batch is the display class of a shaped text glyph.
    ///
    /// Set by [`SceneCache`] from
    /// [`cad_representation::DisplayPrimitive::MeshWithoutEdgeOverlay`] only.
    /// It is an explicit field because a mesh with no wireframe is no longer
    /// distinguishable from a glyph by its edge list: an imported closed mesh
    /// (a welded STL/glTF/OBJ surface) legitimately has **no boundary edges** and
    /// must stay visible during camera motion instead of being hidden as text.
    pub text_glyph: bool,
    /// Analytic circles/arcs drawn as SDF instances. Non-empty only for
    /// [`RenderTopology::Conics`], whose `vertices` is empty. Each entry is a
    /// slice-1 circle/arc (see [`RenderConic`]).
    pub conics: Vec<RenderConic>,
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
    /// The scene copies this from [`cad_representation::DisplayFragment::draw_order`],
    /// i.e. the source entity's `DbEntity::draw_order`. Compact merging requires
    /// an equal order, so a batch never mixes fragments at different orders.
    pub draw_order: i64,
}

impl RenderBatch {
    /// A metadata-only copy: every geometry payload is dropped, every scalar and
    /// style field is kept.
    ///
    /// A resident scene may release this CPU geometry once the GPU owns a
    /// committed upload. What survives is exactly what non-upload readers need:
    /// counts/topology/origin, style and draw order, the texture/PBR keys
    /// (`mesh_texture_keys`), and the pick-source metadata is **dropped** because
    /// the renderer's `PickTable` already owns it after upload. Geometry readers
    /// must never treat this copy as an empty batch; the scene marks the payload
    /// released and refuses a re-materialisation explicitly.
    pub fn metadata_only(&self) -> Self {
        Self {
            local_origin: self.local_origin,
            topology: self.topology,
            vertices: Vec::new(),
            normals: Vec::new(),
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: self.texture.clone(),
            pbr: self.pbr.clone(),
            indices: Vec::new(),
            edges: Vec::new(),
            text_glyph: self.text_glyph,
            conics: Vec::new(),
            mirrored: self.mirrored,
            alpha: self.alpha,
            color: self.color,
            color_unresolved: self.color_unresolved,
            lineweight: self.lineweight,
            lineweight_unresolved: self.lineweight_unresolved,
            linetype: self.linetype.clone(),
            linetype_unresolved: self.linetype_unresolved,
            sources: Vec::new(),
            draw_order: self.draw_order,
        }
    }

    /// A metadata-only placement of a shared block template.
    ///
    /// The heavy arrays come from `self` (the block-local template) at staging;
    /// the placement stores the world origin (pure translation), its own draw
    /// order and its own selection sources. Geometry readers must resolve it
    /// through the resident scene's template table, never treat the empty
    /// arrays as an empty batch.
    pub fn template_placeholder(
        &self,
        translation: [f64; 3],
        draw_order: i64,
        sources: Vec<cad_domain::SelectionRef>,
    ) -> Self {
        let mut batch = self.metadata_only();
        batch.local_origin = Point3 {
            x: self.local_origin.x + translation[0],
            y: self.local_origin.y + translation[1],
            z: self.local_origin.z + translation[2],
        };
        batch.draw_order = draw_order;
        batch.sources = sources;
        batch
    }

    pub fn triangle_count(&self) -> usize {
        if self.topology != RenderTopology::Mesh {
            0
        } else if !self.indices.is_empty() {
            self.indices.len()
        } else {
            self.vertices.len() / 3
        }
    }

    /// Whether this is the display class of a shaped text glyph.
    ///
    /// The renderer uses this to hide text during camera motion (navigation
    /// LOD). The representation primitive owns the decision: only
    /// [`cad_representation::DisplayPrimitive::MeshWithoutEdgeOverlay`] (standard
    /// and vendor text shaping) sets [`RenderBatch::text_glyph`], so this reads
    /// the explicit class instead of inferring it from `edges`/`pbr`.
    ///
    /// The inference is gone because an imported closed mesh has no boundary
    /// edges either (its every edge is shared) and would otherwise be hidden as
    /// text during navigation.
    pub fn is_text_glyph(&self) -> bool {
        self.text_glyph
    }

    pub fn approx_bytes(&self) -> usize {
        (self.vertices.len() + self.normals.len() + self.edges.len() + self.colors.len()) * 12
            + self.uvs.len() * 8
            + self.indices.len() * 12
            + self.conics.len() * std::mem::size_of::<RenderConic>()
            // PBR: the four texture-key strings plus the generated tangent stream
            // (16 B per vertex when the normal map needs a tangent frame).
            + self
                .pbr
                .as_ref()
                .map_or(0, |pbr| pbr.approx_bytes(self.vertices.len()))
    }

    /// The exact packed GPU upload size of this batch, in bytes.
    ///
    /// This mirrors `cad-render-wgpu::Renderer::upload`: positions and edge
    /// positions are 12 bytes per vertex; a mesh **always** uploads one normal
    /// per vertex (repaired when the scene's normals are missing or wrong), so
    /// that normal buffer is `vertices.len()` vectors even when
    /// [`RenderBatch::normals`] is empty; and triangle indices are `[u32; 3]`.
    /// [`SceneBudget::upload_bytes_per_frame`] is charged this amount.
    ///
    /// An enabled wireframe overlay charges its **positions** here (12 bytes
    /// each, already counted in `edges`), not a derived `u32` index buffer: the
    /// renderer draws the `edges` positions directly (as anti-aliased line quads
    /// or packed instances).
    ///
    /// A textured batch additionally uploads its UV0 stream (`@location(3)`,
    /// `Float32x2`, 8 bytes per vertex); it is counted only when the batch
    /// actually carries a texture, mirroring the exact upload in
    /// `Renderer::prepare_upload_batches`.
    pub fn upload_size_bytes(&self) -> usize {
        if self.topology == RenderTopology::Conics {
            // One 72-byte SDF instance per conic (see `ArcInstance` in
            // `cad-render-wgpu`); mirrors `CONIC_INSTANCE_BYTES`.
            return self.conics.len() * CONIC_INSTANCE_BYTES as usize;
        }
        let positions = self.vertices.len() * 12;
        let edges = self.edges.len() * 12;
        let normals = if self.topology == RenderTopology::Mesh {
            self.vertices.len() * 12
        } else {
            self.normals.len() * 12
        };
        let triangle_index_bytes = self.indices.len() * 12;
        // A mesh always binds a per-vertex colour attribute (white when the
        // batch carries no gradient), so its 12 bytes/vertex are uploaded too.
        let vertex_color_bytes = if self.topology == RenderTopology::Mesh {
            self.vertices.len() * 12
        } else {
            0
        };
        // Textured meshes upload one `Float32x2` per vertex; untextured batches
        // upload no UV buffer at all (this is exactly 0 for them).
        let uv_bytes = self.uvs.len() * 8;
        // A PBR batch additionally uploads a `Float32x4` tangent stream (16 B per
        // vertex) when its normal map needs a tangent frame; the four texture-key
        // strings are charged too, so the residency accounting covers the whole
        // material record (ADR-0039 D6).
        let pbr_bytes = self
            .pbr
            .as_ref()
            .map_or(0, |pbr| pbr.approx_bytes(self.vertices.len()));
        positions
            + normals
            + edges
            + triangle_index_bytes
            + vertex_color_bytes
            + uv_bytes
            + pbr_bytes
    }

    /// Vertex-space bytes: positions, normals and edge positions (12 bytes each),
    /// plus UV0 (8 bytes) for a textured batch, plus the PBR material record
    /// (four key strings and the 16-byte tangent stream when one is generated).
    pub fn vertex_bytes(&self) -> usize {
        (self.vertices.len() + self.normals.len() + self.edges.len()) * 12
            + self.uvs.len() * 8
            + self
                .pbr
                .as_ref()
                .map_or(0, |pbr| pbr.approx_bytes(self.vertices.len()))
    }

    /// World-space centroid used as the transparent-pass depth-sort key.
    ///
    /// It is `local_origin + mean(vertices)`. The mean is taken in `f64` before
    /// narrowing so large coordinates do not lose precision. A batch with no
    /// vertices falls back to its `local_origin`. A conic batch (empty vertices)
    /// uses the mean of its conic centers.
    pub fn centroid(&self) -> [f32; 3] {
        if self.vertices.is_empty() && !self.conics.is_empty() {
            let n = self.conics.len() as f64;
            let mut sx = 0.0f64;
            let mut sy = 0.0f64;
            let mut sz = 0.0f64;
            for conic in &self.conics {
                sx += conic.center.x;
                sy += conic.center.y;
                sz += conic.center.z;
            }
            return [(sx / n) as f32, (sy / n) as f32, (sz / n) as f32];
        }
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

/// Every distinct texture key the given batches reference, deduplicated and
/// sorted, so an uploader can resolve exactly the resident set.
///
/// This is the single source of truth the host uses before
/// `Renderer::upload_mesh_textures`: it walks the batches it already holds
/// instead of keeping a second texture table that could drift. A batch
/// contributes its base-colour key plus the four PBR slots it declared
/// (metallic-roughness, normal, emissive, occlusion — ADR-0039 D7). The result is
/// sorted (a [`BTreeSet`] first) so an upload report, a diagnostic listing and a
/// test assertion are all deterministic; an untextured, non-PBR scene yields an
/// empty vector.
pub fn mesh_texture_keys<'a>(
    batches: impl IntoIterator<Item = &'a RenderBatch>,
) -> Vec<ResourceKey> {
    let mut keys = std::collections::BTreeSet::new();
    for batch in batches {
        if let Some(texture) = &batch.texture {
            keys.insert(texture.clone());
        }
        if let Some(pbr) = &batch.pbr {
            for key in pbr.texture_keys() {
                keys.insert(key.clone());
            }
        }
    }
    keys.into_iter().collect()
}

/// Sanitise a raw alpha value from a display fragment into `[0, 1]`.
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

/// Byte size of one packed SDF conic instance on the GPU (`ArcInstance` in
/// `cad-render-wgpu`: four `vec4<f32>` + one `vec2<u32>`).
///
/// Public so a caller can reason about the exact conic-stream byte charge
/// without duplicating the layout; the renderer's `ARC_INSTANCE_STRIDE` is
/// fenced against it by a compile-time assertion.
pub const CONIC_INSTANCE_BYTES: u32 = 72;

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

/// A WIPEOUT mask carried separately from geometry batches.
///
/// A mask is not triangle/line geometry the frame plan orders: it must paint
/// **last**, with `depth_compare: Always` and no depth write, in the renderer's
/// clear colour (ADR 0022 D1). The scene therefore carries the world-space
/// boundary polygon and lets the renderer tessellate it (even-odd, concave-safe)
/// and issue its own final pass.
///
/// The polygon is preserved exactly as the representation produced it (no
/// fan-triangulation here, which would leak a concave boundary), and the
/// draw-order interleaving promised by ADR 0022 D2 is **not** implemented yet:
/// masks always paint after every other class. `draw_order` carries the WIPEOUT
/// entity's real display order — no longer a constant — so the follow-up can
/// segment the mask pass at the right position without another data-loss fix.
///
/// `inverted` (DXF clip mode Inside) is carried through unchanged. The renderer
/// expresses the complement by adding a viewport-covering ring to the same
/// even-odd fill, so the scene never needs the viewport and stays view-independent.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskBatch {
    /// Closed world-space boundary polygon (three or more vertices).
    pub polygon: Arc<[Point3]>,
    /// `true` for clip mode Inside: mask everything *outside* the polygon.
    pub inverted: bool,
    /// Resolved boundary-frame colour (`WIPEOUTFRAME` 1/2). `None` means the
    /// frame is not drawn (`WIPEOUTFRAME 0`); `Some(rgb)` is the sanitised
    /// linear colour the renderer strokes the closed boundary with, after the
    /// fill. The fill itself always paints the clear colour, never this.
    pub frame: Option<[f32; 3]>,
    pub draw_order: i64,
    pub sources: Vec<SelectionRef>,
}

impl MaskBatch {
    /// CPU bytes this mask holds in the scene cache.
    pub fn approx_bytes(&self) -> usize {
        self.polygon.len() * std::mem::size_of::<Point3>()
            + self.sources.len() * std::mem::size_of::<SelectionRef>()
            + self.frame.map_or(0, |_| std::mem::size_of::<[f32; 3]>())
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
    /// Display-only brightness/contrast/fade/monochrome transfer, copied from
    /// the entity's [`ImageAdjust`] so the image pass can apply it.
    pub adjust: ImageAdjust,
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
