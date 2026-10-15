// Mesh pipeline (triangle list, depth tested, back-face culled).
//
// Kept in sync with `MESH_SHADER` in `src/plan.rs` via `include_str!`.
//
// Shading is deliberately explicit and minimal: a fixed headlight along -Z
// gives an N.L diffuse term plus a constant ambient floor. No physically based
// rendering, environment lighting or specular response is claimed.
//
// Winding: the mirrored variant of this pipeline selects `front_face = Cw`;
// the shader is identical for both.
//
// Colour: `camera.tint.rgb` is the per-batch colour and `in.color` is the
// per-vertex colour. A non-gradient mesh supplies white vertices, so the
// product reduces to the batch tint and existing geometry is unchanged; a
// gradient HATCH bakes its ramp into `in.color`.
//
// `camera.key_flags.x` is the batch's renderer pick key, read by the selection
// flags block (group 1). With `flags_ctl.enabled == 0` (default) the output is
// exactly the pre-flags output.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = constant per-batch colour; a = constant per-batch alpha.
    tint: vec4<f32>,
    // x = renderer pick key, y = INSTANCE_FLAG_* bits, z/w reserved.
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
    @builtin(position) position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
) -> VsOut {
    var out: VsOut;
    out.position = camera.transform * vec4<f32>(position, 1.0);
    out.world_normal = normal;
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let light_dir = vec3<f32>(0.0, 0.0, -1.0);
    let diffuse = max(dot(n, -light_dir), 0.0);
    // `tint.rgb` is the batch's own colour, modulated by the per-vertex colour
    // (white when the batch has no gradient). The diffuse term scales the
    // result and the ambient floor keeps a back-facing facet visible instead of
    // black.
    let base = camera.tint.rgb * in.color;
    let lit = base * (0.25 + 0.75 * diffuse);
    let key = camera.key_flags.x;
    if (flags_ctl.enabled != 0u && (flag_bits(key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    return flag_tint(vec4<f32>(lit, camera.tint.a), key);
}
