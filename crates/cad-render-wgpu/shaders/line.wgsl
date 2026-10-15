// Line pipeline (polyline / wireframe overlay).
//
// Kept in sync with `LINE_SHADER` in `src/plan.rs` via `include_str!`. The
// uniform is `transform` (mat4x4) followed by `tint` and `key_flags`; `tint.rgb`
// is the constant per-batch colour (normalized sRGB, sanitized by the
// scene/renderer) and `tint.a` is the constant per-batch alpha.
//
// `key_flags.x` is the batch's renderer pick key (see `PickTable`); it is read by
// the **selection-flags** block below (group 1). With `flags_ctl.enabled == 0`
// (the default) the fragment output is exactly what this shader produced before
// the flags block existed, so the existing highlight-overlay batches keep
// working with no double-highlight.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = constant per-batch colour; a = constant per-batch alpha.
    tint: vec4<f32>,
    // x = renderer pick key, y = INSTANCE_FLAG_* bits, z/w reserved.
    key_flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

// --- selection flags (group 1) ---------------------------------------------
// A small integer table indexed by the pick key: one texel per resident batch
// (packed into rows when the key space exceeds the texture width). `enabled` is
// the global toggle; while it is 0 the shader ignores the table entirely.
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

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return camera.transform * vec4<f32>(position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    let key = camera.key_flags.x;
    if (flags_ctl.enabled != 0u && (flag_bits(key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    return flag_tint(vec4<f32>(camera.tint.rgb, camera.tint.a), key);
}
