// Accelerated (vertex-pulling) merged-mesh pipeline.
//
// ONE indexed draw covers **every** flat-merged mesh chunk of the resident
// scene, across all per-chunk origin spaces, by pulling the vertex record from a
// read-only vertex-stage storage buffer and the triangle index list from a
// second storage buffer. This is the capability-gated "accelerated" tier: it
// requires storage buffers readable from the vertex stage, which WebGL2 does not
// have (hence the tier gate in `Renderer::initialize_with_device`). It is
// deliberately **not** part of the WebGL2-safe universal path
// (`mesh_merged.wgsl`).
//
// Equivalence with `mesh_merged.wgsl` is byte-for-byte where the arithmetic is
// identical: the pulled vertex carries the exact same
// position/normal/baked-colour/entity_flags values (the CPU `f64` reduction
// `(world - origin) as f32` still happens at upload, unchanged), the shading
// body is the same expression, and the vertex stage reconstructs the **same**
// `mat4x4<f32>` the universal path reads from the per-chunk uniform:
// `camera.c0/c1/c2` are the origin-independent columns and
// `chunk_transforms[info.z]` is the per-chunk translation column
// (`FrameProjection::batch_matrix_f64(origin)`'s last column, reduced in `f64`
// on the CPU). The multiply is the identical f32 op sequence.
//
// `indices[vertex_index]` is the flat triangle index list (three entries per
// triangle, in draw order), so `@builtin(vertex_index)` enumerates exactly the
// sequence `draw_indexed` would have rasterized. `info.x` is the renderer pick
// key, passed flat and read by the selection-flags block (group 1).
//
// Kept in sync with `MESH_PULLED_SHADER` in `src/plan.rs` via `include_str!`.

struct PullCamera {
    // Columns 0..=2 of the per-chunk transform; column 3 (the translation) is
    // per chunk and read from `chunk_transforms`.
    c0: vec4<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
    // x = world units per pixel (unused by the mesh path; it is the AA line SDF
    // ramp width on the pulled line pipeline), yzw reserved.
    c3: vec4<f32>,
    // rgb = unit vector toward the light, in world space (the same space as the
    // vertex normal); a = reserved 0.
    light: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: PullCamera;

// --- selection flags (group 1) ---------------------------------------------
struct FlagsCtl {
    enabled: u32,
    // Texels per row; the key is addressed as (key % width, key / width).
    width: u32,
    _pad0: u32,
    _pad1: u32,
};
@group(1) @binding(0) var flags_table: texture_2d<u32>;
@group(1) @binding(1) var<uniform> flags_ctl: FlagsCtl;

const FLAG_SELECTED: u32 = 1u;
const FLAG_HOVERED: u32 = 2u;
const FLAG_HIDDEN: u32 = 4u;
// Mechanical placeholder tints so the toggle is observable; the final selection
// visual system is owned by the follow-up UI slice.
const SELECTED_TINT: vec3<f32> = vec3<f32>(1.0, 0.55, 0.12);
const HOVERED_TINT: vec3<f32> = vec3<f32>(0.30, 0.85, 1.0);

fn flag_bits(key: u32) -> u32 {
    let k = i32(key);
    let w = i32(flags_ctl.width);
    let xy = vec2<i32>(k % w, k / w);
    return textureLoad(flags_table, xy, 0).r;
}

fn flag_tint(color: vec4<f32>, key: u32) -> vec4<f32> {
    if (flags_ctl.enabled == 0u) {
        return color;
    }
    let bits = flag_bits(key);
    if ((bits & FLAG_SELECTED) != 0u) {
        return vec4<f32>(SELECTED_TINT, color.a);
    }
    if ((bits & FLAG_HOVERED) != 0u) {
        return vec4<f32>(HOVERED_TINT, color.a);
    }
    return color;
}

// One pulled merged-mesh vertex. Field order/padding matches the CPU
// `plan::PulledMeshVertex` (`#[repr(C)]`, 64 bytes). The fourth field is named
// `info` because WGSL reserves the identifier `meta`.
struct PulledMeshVertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    color: vec4<f32>,
    // x = renderer pick key, y = INSTANCE_FLAG_* bits, z = chunk index.
    info: vec4<u32>,
};
@group(2) @binding(0) var<storage, read> vertices: array<PulledMeshVertex>;
// Flat triangle index list, rebased onto the whole-scene vertex buffer.
@group(2) @binding(1) var<storage, read> indices: array<u32>;
// Per-chunk translation column, rewritten each frame.
@group(2) @binding(2) var<storage, read> chunk_transforms: array<vec4<f32>>;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    // The pick key is an integer, so it must be passed flat.
    @location(2) @interpolate(flat) key: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    let vertex = vertices[indices[vertex_index]];
    let t = chunk_transforms[vertex.info.z];
    let transform = mat4x4<f32>(camera.c0, camera.c1, camera.c2, t);
    var out: VsOut;
    out.position = transform * vec4<f32>(vertex.position.xyz, 1.0);
    out.world_normal = vertex.normal.xyz;
    out.color = vertex.color.xyz;
    out.key = vertex.info.x;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    // Same display-convention diffuse term as `mesh.wgsl`: `camera.light.rgb` is
    // the frame's unit vector toward the light (2D: world +Z; 3D: the
    // camera-relative key light, offset from the view axis).
    let diffuse = max(dot(n, camera.light.xyz), 0.0);
    // The merged uniform tint is opaque white (the per-batch tint is baked into
    // the per-vertex colour at upload), so `1.0 * in.color == in.color` exactly
    // and the shared `camera.tint.rgb * in.color` product is reproduced.
    let base = in.color;
    let lit = base * (0.25 + 0.75 * diffuse);
    if (flags_ctl.enabled != 0u && (flag_bits(in.key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    return flag_tint(vec4<f32>(lit, 1.0), in.key);
}
