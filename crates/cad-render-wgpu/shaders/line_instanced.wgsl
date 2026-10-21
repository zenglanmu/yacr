// Instanced line pipeline: one instance per 2-vertex `LineList` segment, drawn as
// an anti-aliased SDF quad stroke.
//
// Visual unification (see `docs/adr/0009-antialiased-line-strokes.md`): the
// shared unit **quad** `[0,1]^2` (slot 0, six vertices / two triangles) is
// expanded around the segment `p0 -> p1` into a world-space quad, and the
// fragment stage computes the distance to the **segment** and ramps a triangle
// (linear) coverage filter over it — exactly the SDF conic semantics, so every
// line class carries **1.0 px of linear ink** like the SDF arcs.
//
// The per-instance vertex buffer (step mode `Instance`) carries both endpoints
// in the stream's fixed-point-relative world space, the sanitised rgba tint, the
// carried lineweight and the first-class pick key. This layout deliberately uses
// only per-instance *vertex attributes* (no vertex-stage storage buffer), so it
// is WebGL2-safe.
//
// `id_flags.x` is the batch's renderer pick key (see `PickTable`); it is passed
// to the fragment stage flat and read by the selection-flags block (group 1).
//
// Kept in sync with `LINE_INSTANCED_SHADER` in `src/plan.rs` via `include_str!`.
// Group 0 is the shared geometry uniform; `transform` maps stream-relative world
// -> clip, `key_flags.z` carries the world units per pixel (the SDF ramp width),
// and `tint` is unused here but kept so the instanced pipeline can share the
// geometry uniform's bind group layout.

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // x = renderer pick key (unused: the per-instance key wins), y = flags,
    // z = world units per pixel (bitcast f32; the SDF ramp width), w reserved.
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

fn world_per_px() -> f32 {
    return bitcast<f32>(camera.key_flags.z);
}

// Expand a segment `p0 -> p1` around the unit-quad `corner` (in `[-1, 1]^2`) into
// a world-space quad covering the anti-aliased band. Shared verbatim with
// `line.wgsl` / `line_pulled.wgsl` (kept in sync by hand).
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
    // Shared unit quad in `[0, 1]^2` (component xy is the quad corner).
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
    let corner = unit.xy * 2.0 - 1.0;
    let a = p0_width.xyz;
    let b = p1_pad.xyz;
    let world = segment_quad_world(a, b, corner, world_per_px());
    var out: VsOut;
    out.clip = camera.transform * vec4<f32>(world, 1.0);
    out.world_pos = world;
    out.p0 = a;
    out.p1 = b;
    // `rgba` is sanitised at upload (finite, clamped to [0, 1]), exactly like
    // the per-batch tint the legacy line shader receives.
    out.color = rgba;
    out.key = id_flags.x;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = segment_distance(in.world_pos, in.p0, in.p1);
    let wpp = world_per_px();
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
