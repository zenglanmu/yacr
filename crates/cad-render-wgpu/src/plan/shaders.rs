// Shaders are checked into `shaders/` so they can be validated as files and
// stay reviewable; `include_str!` keeps a single source of truth.
pub(crate) const LINE_SHADER: &str = include_str!("../../shaders/line.wgsl");

// Instanced line shader. One instance per 2-vertex `LineList` segment; the
// per-instance attributes are expanded against a shared unit segment so the
// dominant CAD primitive class collapses to one draw call per contiguous run.
pub(crate) const LINE_INSTANCED_SHADER: &str = include_str!("../../shaders/line_instanced.wgsl");

// Accelerated (vertex-pulling) instanced line shader: one draw covers every
// instanced line segment of the resident scene by pulling the per-instance
// record from a read-only vertex-stage storage buffer. Capability-gated (see
// `Renderer::initialize_with_device`); deliberately not WebGL2-safe.
pub(crate) const LINE_PULLED_SHADER: &str = include_str!("../../shaders/line_pulled.wgsl");

// Mesh shader. Shading is intentionally explicit: a single fixed headlight
// direction, N·L diffuse plus a constant ambient term. There is no claim of
// physically based rendering, environment lighting or specular response.
pub(crate) const MESH_SHADER: &str = include_str!("../../shaders/mesh.wgsl");

// Merged-mesh shader: the same shading body as `MESH_SHADER` with an extra
// per-vertex `entity_flags` attribute and a per-vertex baked colour (the
// per-batch tint is folded in at upload, see `push_mesh_vertices`).
pub(crate) const MESH_MERGED_SHADER: &str = include_str!("../../shaders/mesh_merged.wgsl");

// Accelerated (vertex-pulling) merged-mesh shader: one indexed draw covers
// every flat-merged mesh chunk by pulling vertices and triangle indices from
// read-only vertex-stage storage buffers. Capability-gated; not WebGL2-safe.
pub(crate) const MESH_PULLED_SHADER: &str = include_str!("../../shaders/mesh_pulled.wgsl");

// Raster image shader. It samples a texture and modulates it by a per-image
// alpha/colour factor; there is no lighting model.
pub(crate) const IMAGE_SHADER: &str = include_str!("../../shaders/image.wgsl");

// SDF conic shader (slice B). One instance per analytic circle/arc; the shared
// unit quad is expanded to the conic's extent and the fragment shader computes
// the signed distance (see `docs/adr/0008-sdf-conic-rendering.md`).
pub(crate) const ARC_SHADER: &str = include_str!("../../shaders/arc.wgsl");

// ---------------------------------------------------------------------------
// Instanced line stream ("entity = data", first slice)
// ---------------------------------------------------------------------------
//
// The dominant CAD primitive class is the 2-vertex `LineList` segment. Instead
// of one draw call per batch, segments are packed into a single static instance
// stream at upload time and expanded in the vertex shader against a shared unit
// segment. Instance order is the original batch order, so equal-depth paint
// semantics are preserved exactly (no re-sorting); the frame path coalesces
// adjacent instances of one stream into as few draws as the opaque order allows.
//
// The layout is WebGL2-safe: every field is a per-instance *vertex attribute*
// (`VertexStepMode::Instance`), never a vertex-stage storage buffer.
