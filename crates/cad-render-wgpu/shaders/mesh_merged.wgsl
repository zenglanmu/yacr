// Merged-mesh pipeline (triangle list, depth tested, back-face culled).
//
// Used only by flat-merged mesh runs (see `build_mesh_chunks`): several adjacent
// same-pipeline opaque `Mesh` batches are concatenated into one static vertex
// buffer and drawn as a single indexed draw, so the dominant glyph-mesh draw
// class collapses. The shading body is byte-for-byte the same expression as
// `mesh.wgsl`; only the vertex input differs.
//
// Two things make the merged draw equivalent to the per-batch draws it replaces:
//
// * the per-batch constant tint is *baked* into the per-vertex colour at upload
//   (`tint.rgb * vertex_colour`, an f32 product), and the shared uniform tint is
//   written as opaque white, so `camera.tint.rgb * in.color` reproduces the
//   per-batch product. This is mathematically identical to the legacy per-pixel
//   `tint * colour`, but **not bit-identical in general**: the legacy shader
//   evaluates `tint.rgb * interpolate(colour)` per fragment while this evaluates
//   `interpolate(tint.rgb * colour)`. When the per-vertex colours vary across a
//   triangle (a gradient / HATCH ramp) and the tint is not white, the two orders
//   re-associate the f32 product and can round differently. Byte-identity is
//   therefore claimed only for a constant-colour mesh (the interpolated colour
//   is then exact and `1.0 * x == x`) or a white tint; a varying-colour ramp is
//   fenced by a pixel test with a documented tolerance (it is byte-identical on
//   the reference adapter, but that is empirical, not a guarantee);
// * the merged run only ever contains opaque batches (`alpha` clamps to exactly
//   `1.0`), so a single `tint.a` is correct for every member.
//
// `entity_flags.x` is the owning batch's renderer pick key (see `PickTable`),
// passed to the fragment stage flat and read by the selection-flags block
// (group 1). With `flags_ctl.enabled == 0` (default) the output is exactly the
// pre-flags output. The legacy per-batch uniform never carried a pick key.
//
// Kept in sync with `MESH_MERGED_SHADER` in `src/plan.rs` via `include_str!`.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = constant per-batch colour (white for a merged chunk, whose colour is
    // baked per vertex); a = constant per-batch alpha (1.0 for merged chunks).
    tint: vec4<f32>,
    // x = renderer pick key (unused: the per-vertex key wins), y = flags.
    key_flags: vec4<u32>,
    // rgb = unit vector toward the light, in world space (the same space as
    // `world_normal`); a = reserved 0. It is a per-frame camera property: the
    // 2D view keeps world +Z, the 3D view uses the reversed view direction.
    light: vec4<f32>,
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
    // The pick key is an integer, so it must be passed flat.
    @location(2) @interpolate(flat) key: u32,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // [0] = the batch's renderer pick key; [1] = its INSTANCE_FLAG_* bits.
    @location(3) entity_flags: vec2<u32>,
) -> VsOut {
    var out: VsOut;
    out.position = camera.transform * vec4<f32>(position, 1.0);
    out.world_normal = normal;
    out.color = color;
    out.key = entity_flags.x;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    // Same display-convention diffuse term as `mesh.wgsl`: `camera.light.rgb` is
    // the frame's unit vector toward the light (2D: world +Z; 3D: the
    // camera-relative key light, offset from the view axis).
    let diffuse = max(dot(n, camera.light.xyz), 0.0);
    let base = camera.tint.rgb * in.color;
    let lit = base * (0.25 + 0.75 * diffuse);
    if (flags_ctl.enabled != 0u && (flag_bits(in.key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    return flag_tint(vec4<f32>(lit, camera.tint.a), in.key);
}
