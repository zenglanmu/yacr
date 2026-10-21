// ID-pass line pipeline (legacy per-batch AA quad stroke).
//
// This shader draws exactly the geometry `line.wgsl` draws, with exactly the
// same transform and primitive state, but the fragment stage writes the batch's
// renderer pick key instead of a colour. It is used only by the on-demand id
// pass (`Renderer::pick_at`), never by a colour frame.
//
// The geometry is the same pre-expanded triangle-list quad (endpoints `p0`/`p1`
// plus a corner), and the SDF is the same distance-to-segment field; a fragment
// is only written where the triangle filter's coverage is at least 0.5, i.e.
// where the fragment centre is within **±0.5 px** of the analytic segment (the
// triangle filter reaches 0.5 at `d = 0.5·w`, one pixel being `w =
// world_per_px`), so the documented ±0.5 px AA-fringe pick tolerance holds and
// the id pass agrees with the colour pass.
//
// The module is **assembled** at pipeline creation: this file provides the
// uniform, the vertex stage and `fs_key`, and a fragment tail declares
// `@fragment fn fs_main(...)` for the active id-target format (see `pick.rs`).
// Both tails are validated offline (see `tests/wgsl_validation.rs`).

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // x = this batch's renderer pick key, y = INSTANCE_FLAG_* bits,
    // z = world units per pixel (bitcast f32; the SDF ramp width).
    key_flags: vec4<u32>,
    // rgb = unit vector toward the light, in world space (the same space as
    // `world_normal`); a = reserved 0. It is a per-frame camera property: the
    // 2D view keeps world +Z, the 3D view uses the reversed view direction.
    light: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

fn world_per_px() -> f32 {
    return bitcast<f32>(camera.key_flags.z);
}

// Expand a segment `p0 -> p1` around the unit-quad `corner` (in `[-1, 1]^2`).
// Kept identical to `line.wgsl` so the id pass rasterizes the same quad.
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
};

@vertex
fn vs_main(
    @location(0) p0: vec3<f32>,
    @location(1) p1: vec3<f32>,
    @location(2) corner_uv: vec2<f32>,
) -> VsOut {
    let corner = corner_uv * 2.0 - 1.0;
    let world = segment_quad_world(p0, p1, corner, world_per_px());
    var out: VsOut;
    out.clip = camera.transform * vec4<f32>(world, 1.0);
    out.world_pos = world;
    out.p0 = p0;
    out.p1 = p1;
    return out;
}

// The pick key this fragment reports: the batch's own key from its uniform
// (a legacy per-batch draw is never coalesced, so the uniform is exact), written
// only where the SDF coverage is at least 0.5 (the ±0.5 px pick tolerance).
fn fs_key(in: VsOut) -> u32 {
    let d = segment_distance(in.world_pos, in.p0, in.p1);
    let wpp = world_per_px();
    let w = select(fwidth(d), wpp, wpp > 0.0);
    let coverage = saturate(1.0 - d / max(w, 1e-8));
    if (coverage < 0.5) {
        discard;
    }
    return camera.key_flags.x;
}
