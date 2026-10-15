// Accelerated (vertex-pulling) instanced line pipeline.
//
// ONE draw covers **every** instanced line segment of the resident scene,
// across all per-chunk origin streams, by pulling the per-instance record from a
// read-only vertex-stage storage buffer. This is the capability-gated
// "accelerated" tier: it requires a storage buffer readable from the vertex
// stage, which WebGL2 does not have (hence the tier gate in
// `Renderer::initialize_with_device`). It is deliberately **not** part of the
// WebGL2-safe universal path (`line_instanced.wgsl`).
//
// Equivalence with `line_instanced.wgsl` is byte-for-byte where the arithmetic
// is identical: the per-instance record carries the exact same
// `p0_width`/`p1_pad`/`rgba`/`id_flags` values (the CPU `f64` reduction
// `(world - origin) as f32` still happens at upload, unchanged), and the vertex
// stage reconstructs the **same** `mat4x4<f32>` the universal path reads from
// the per-stream uniform: `camera.c0/c1/c2` are the origin-independent columns
// and `chunk_transforms[info.z]` is the per-chunk translation column
// (`FrameProjection::batch_matrix_f64(origin)`'s last column, still reduced in
// `f64` on the CPU). The multiply is therefore the identical f32 op sequence.
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

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    // The pick key is an integer, so it must be passed flat.
    @location(1) @interpolate(flat) key: u32,
};

@vertex
fn vs_main(
    // Shared unit segment index: 0 or 1 (the two endpoints).
    @builtin(vertex_index) vertex_index: u32,
    // Index of this segment in the whole-scene instance storage buffer.
    @builtin(instance_index) instance_index: u32,
) -> VsOut {
    let inst = instances[instance_index];
    let unit = f32(vertex_index);
    // Exact endpoint selection, identical to the universal instanced shader:
    // `unit` is exactly 0 or 1, so `select` picks without arithmetic.
    let position = select(inst.p0_width.xyz, inst.p1_pad.xyz, unit > 0.5);
    let t = chunk_transforms[inst.info.z];
    let transform = mat4x4<f32>(camera.c0, camera.c1, camera.c2, t);
    var out: VsOut;
    out.clip = transform * vec4<f32>(position, 1.0);
    out.color = inst.rgba;
    out.key = inst.info.x;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (flags_ctl.enabled != 0u && (flag_bits(in.key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    return flag_tint(in.color, in.key);
}
