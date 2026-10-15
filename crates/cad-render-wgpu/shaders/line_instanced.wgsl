// Instanced line pipeline: one instance per 2-vertex `LineList` segment.
//
// The vertex buffer is a shared unit segment `(0,0,0)-(1,0,0)`; the per-instance
// vertex buffer (step mode `Instance`) carries both endpoints in the stream's
// fixed-point-relative world space, the sanitised rgba tint, the carried
// lineweight and the first-class pick key. The endpoint is chosen with
// `select(p0, p1, unit.x > 0.5)`: `select` is a bit-select, exact by
// construction on every backend. (`mix(a, b, unit.x)` happens to reproduce the
// endpoints here, but a backend may lower it to a lerp that does not, so it is
// deliberately not used.)
//
// This layout deliberately uses only per-instance *vertex attributes* (no
// vertex-stage storage buffer), so it is WebGL2-safe.
//
// `id_flags.x` is the batch's renderer pick key (see `PickTable`); it is passed
// to the fragment stage flat and read by the selection-flags block (group 1).
// With `flags_ctl.enabled == 0` (default) the output is exactly the pre-flags
// output.
//
// Kept in sync with `LINE_INSTANCED_SHADER` in `src/plan.rs` via `include_str!`.
// Group 0 is the shared geometry uniform; the instanced path only consumes
// `transform` (stream-relative world -> clip). `tint` is unused here but kept so
// the instanced pipeline can share the geometry uniform's bind group layout.

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // x = renderer pick key (unused: the per-instance key wins), y = flags.
    key_flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

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

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    // The pick key is an integer, so it must be passed flat.
    @location(1) @interpolate(flat) key: u32,
};

@vertex
fn vs_main(
    // Shared unit segment: component x is 0 or 1.
    @location(0) unit: vec3<f32>,
    // xyz = segment start (stream-relative), w = lineweight mm (carried, not
    // rasterized: wide lines are not portable across wgpu backends).
    @location(1) p0_width: vec4<f32>,
    // xyz = segment end (stream-relative), w reserved (0).
    @location(2) p1_pad: vec4<f32>,
    // Sanitised linear sRGB colour (rgb) and clamped alpha (a).
    @location(3) rgba: vec4<f32>,
    // First-class pick key: [0] is the batch's renderer pick key, [1] its
    // INSTANCE_FLAG_* bitfield.
    @location(4) id_flags: vec2<u32>,
) -> VsOut {
    // Exact endpoint selection: `unit.x` is exactly 0 or 1 (shared unit
    // segment), and `select` picks without arithmetic.
    let position = select(p0_width.xyz, p1_pad.xyz, unit.x > 0.5);
    var out: VsOut;
    out.clip = camera.transform * vec4<f32>(position, 1.0);
    // `rgba` is sanitised at upload (finite, clamped to [0, 1]), exactly like
    // the per-batch tint the legacy line shader receives.
    out.color = rgba;
    out.key = id_flags.x;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (flags_ctl.enabled != 0u && (flag_bits(in.key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    return flag_tint(in.color, in.key);
}
