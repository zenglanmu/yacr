// Accelerated (vertex-pulling) instanced line pipeline — anti-aliased SDF
// strokes.
//
// ONE draw covers **every** instanced line segment of the resident scene, across
// all per-chunk origin streams, by pulling the per-instance record from a
// read-only vertex-stage storage buffer. This is the capability-gated
// "accelerated" tier: it requires a storage buffer readable from the vertex
// stage, which WebGL2 does not have (hence the tier gate in
// `Renderer::initialize_with_device`). It is deliberately **not** part of the
// WebGL2-safe universal path (`line_instanced.wgsl`).
//
// The vertex stage expands the same world-space anti-aliased segment quad as
// `line_instanced.wgsl` (see `docs/adr/0009-antialiased-line-strokes.md`) and the
// fragment stage computes the same distance-to-segment SDF, so the pulled class
// carries the identical 1.0 px of linear ink. There is **no vertex buffer**: the
// six quad corners are selected from a constant table by `@builtin(vertex_index)`.
//
// Equivalence with `line_instanced.wgsl` is byte-for-byte where the arithmetic is
// identical: the per-instance record carries the exact same
// `p0_width`/`p1_pad`/`rgba`/`id_flags` values (the CPU `f64` reduction
// `(world - origin) as f32` still happens at upload, unchanged), and the vertex
// stage reconstructs the **same** `mat4x4<f32>` the universal path reads from the
// per-stream uniform: `camera.c0/c1/c2` are the origin-independent columns and
// `chunk_transforms[info.z]` is the per-chunk translation column
// (`FrameProjection::batch_matrix_f64(origin)`'s last column, still reduced in
// `f64` on the CPU). The multiply is therefore the identical f32 op sequence.
// `camera.c3.x` carries the world units per pixel (the SDF ramp width).
//
// `info.x` is the owning batch's renderer pick key (see `PickTable`); it is
// passed to the fragment stage flat and read by the selection-flags block
// (group 1), exactly like `line_instanced.wgsl`. `info.z` is the per-chunk
// origin index into `chunk_transforms`.
//
// Kept in sync with `LINE_PULLED_SHADER` in `src/plan.rs` via `include_str!`.

struct PullCamera {
    // Columns 0..=2 of the per-stream transform; column 3 (the translation) is
    // per chunk and read from `chunk_transforms`, so one camera uniform serves
    // every stream.
    c0: vec4<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
    // x = world units per pixel (the SDF ramp width), yzw reserved.
    c3: vec4<f32>,
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

// One pulled line segment. Field order/padding matches the CPU
// `plan::PulledLine` (`#[repr(C)]`, 64 bytes) so the storage layout is exact.
// The fourth field is named `info` because WGSL reserves the identifier `meta`.
struct PulledLine {
    p0_width: vec4<f32>,
    p1_pad: vec4<f32>,
    rgba: vec4<f32>,
    // x = renderer pick key, y = INSTANCE_FLAG_* bits, z = chunk index.
    info: vec4<u32>,
};
@group(2) @binding(0) var<storage, read> instances: array<PulledLine>;
// Per-chunk translation column (`FrameProjection::batch_matrix_f64(origin)`
// last column), rewritten each frame exactly like the universal path's
// per-stream uniform slot.
@group(2) @binding(1) var<storage, read> chunk_transforms: array<vec4<f32>>;

// The shared unit quad's six corners (two triangles), selected by the vertex
// index. Identical to the shared `unit_quad_buffer` the universal paths bind.
const CORNERS: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 1.0),
);

// Expand a segment `p0 -> p1` around the unit-quad `corner` (in `[-1, 1]^2`) into
// a world-space quad covering the anti-aliased band. Shared verbatim with
// `line.wgsl` / `line_instanced.wgsl` (kept in sync by hand).
fn segment_quad_world(p0: vec3<f32>, p1: vec3<f32>, corner: vec2<f32>, wpp: f32) -> vec3<f32> {
    let seg = p1 - p0;
    let len = length(seg);
    let dir = select(vec3<f32>(1.0, 0.0, 0.0), seg / len, len > 1e-8);
    // Perpendicular in the drawing plane (`xy`), matching the SDF conics'
    // planar world-space expansion; fall back to the `y` axis for a segment
    // along `z`, whose in-plane perpendicular is undefined.
    let planar = vec2<f32>(-dir.y, dir.x);
    let normal = select(
        vec3<f32>(0.0, 1.0, 0.0),
        normalize(vec3<f32>(planar, 0.0)),
        length(planar) > 1e-4,
    );
    let margin = max(2.0 * wpp, 1e-6);
    let half = wpp + margin;
    let along = 0.5 * len + half;
    let center = 0.5 * (p0 + p1);
    return center + corner.x * dir * along + corner.y * normal * half;
}

fn segment_distance(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>) -> f32 {
    let ab = b - a;
    let denom = dot(ab, ab);
    let t = select(0.0, clamp(dot(p - a, ab) / denom, 0.0, 1.0), denom > 1e-12);
    return length(p - (a + t * ab));
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) @interpolate(flat) p0: vec3<f32>,
    @location(2) @interpolate(flat) p1: vec3<f32>,
    @location(3) color: vec4<f32>,
    // The pick key is an integer, so it must be passed flat.
    @location(4) @interpolate(flat) key: u32,
};

@vertex
fn vs_main(
    // Shared unit-quad corner index (0..5): the two triangles.
    @builtin(vertex_index) vertex_index: u32,
    // Index of this segment in the whole-scene instance storage buffer.
    @builtin(instance_index) instance_index: u32,
) -> VsOut {
    let inst = instances[instance_index];
    let corner = CORNERS[vertex_index] * 2.0 - 1.0;
    let a = inst.p0_width.xyz;
    let b = inst.p1_pad.xyz;
    let world = segment_quad_world(a, b, corner, camera.c3.x);
    let t = chunk_transforms[inst.info.z];
    let transform = mat4x4<f32>(camera.c0, camera.c1, camera.c2, t);
    var out: VsOut;
    out.clip = transform * vec4<f32>(world, 1.0);
    out.world_pos = world;
    out.p0 = a;
    out.p1 = b;
    out.color = inst.rgba;
    out.key = inst.info.x;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = segment_distance(in.world_pos, in.p0, in.p1);
    let wpp = camera.c3.x;
    let w = select(fwidth(d), wpp, wpp > 0.0);
    let coverage = saturate(1.0 - d / max(w, 1e-8));
    if (coverage <= 0.0) {
        discard;
    }
    if (flags_ctl.enabled != 0u && (flag_bits(in.key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    return flag_tint(vec4<f32>(in.color.rgb, in.color.a * coverage), in.key);
}
