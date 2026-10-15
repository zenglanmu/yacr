// ID-pass SDF conic pipeline.
//
// Draws exactly the geometry `arc.wgsl` draws (the shared unit quad expanded by
// the per-instance buffer, the same transform) with the same SDF, but the
// fragment stage writes the per-instance pick key instead of a colour. A fragment
// is only written where the SDF coverage is at least 0.5, i.e. where the fragment
// centre is within **±0.5 px** of the analytic curve (the triangle filter reaches
// 0.5 at `d = 0.5·w`, one pixel being `w = world_per_px`), so the documented
// ±0.5 px AA-fringe pick tolerance holds: a pick lands on the analytic curve, not
// on the discarded quad. Used only by the on-demand id pass.
//
// The module is **assembled** at pipeline creation: this file provides the
// uniform, the vertex stage and `fs_key`, and a fragment tail declares
// `@fragment fn fs_main(...)` for the active id-target format (see `pick.rs`).
// Native-only, so it is excluded from the GLSL ES 300 back-translation list like
// `pick_line.wgsl`.

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // x = pick key (unused; the per-instance key wins), z = world units per pixel
    // (bitcast f32; the SDF ramp width).
    key_flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

const ARC_FLAG_FULL_CIRCLE: u32 = 8u;
const TAU: f32 = 6.283185307179586;
const QUAD_MARGIN_REL: f32 = 0.02;
// See `arc.wgsl`: the quad margin is floored at ~2 world pixels so a small
// on-screen circle's fringe is not clipped. Kept identical to the colour pass so
// the id pass rasterises the same quad.
fn world_per_px() -> f32 {
    return bitcast<f32>(camera.key_flags.z);
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) center_r: vec4<f32>,
    @location(2) uvec: vec4<f32>,
    @location(3) vvec: vec4<f32>,
    @location(4) color: vec4<f32>,
    @location(5) @interpolate(flat) id_meta: vec2<u32>,
};

@vertex
fn vs_main(
    @location(0) unit: vec3<f32>,
    @location(1) center_r: vec4<f32>,
    @location(2) uvec: vec4<f32>,
    @location(3) vvec: vec4<f32>,
    @location(4) rgba: vec4<f32>,
    @location(5) id_flags: vec2<u32>,
) -> VsOut {
    let corner = unit.xy * 2.0 - 1.0;
    let r = center_r.w;
    let margin = max(QUAD_MARGIN_REL, 2.0 * world_per_px() / max(r, 1e-8));
    let grow = 1.0 + margin;
    let world =
        center_r.xyz + corner.x * (grow * center_r.w * uvec.xyz) + corner.y * (grow * vvec.xyz);
    let clip = camera.transform * vec4<f32>(world, 1.0);
    var out: VsOut;
    out.clip = clip;
    out.world_pos = world;
    out.center_r = center_r;
    out.uvec = uvec;
    out.vvec = vvec;
    out.color = rgba;
    out.id_meta = id_flags;
    return out;
}

fn wrap_tau(a: f32) -> f32 {
    return (a / TAU - floor(a / TAU)) * TAU;
}

// Distance to the conic (a true distance field; see `arc.wgsl`).
fn conic_distance(in: VsOut) -> f32 {
    let r = in.center_r.w;
    let q = in.world_pos - in.center_r.xyz;
    let x = dot(q, in.uvec.xyz);
    let y = dot(q, normalize(in.vvec.xyz));
    let d_ring = length(vec2<f32>(x, y)) - r;
    let start = in.uvec.w;
    let sweep = in.vvec.w;
    let full = (in.id_meta.y & ARC_FLAG_FULL_CIRCLE) != 0u;
    let t = atan2(y, x);
    let rel = wrap_tau(t - start);
    let inside = full || select(TAU - rel <= -sweep, rel <= sweep, sweep >= 0.0);
    let e0 = vec2<f32>(r * cos(start), r * sin(start));
    let e1 = vec2<f32>(r * cos(start + sweep), r * sin(start + sweep));
    let d_endpoint = min(
        length(vec2<f32>(x, y) - e0),
        length(vec2<f32>(x, y) - e1),
    );
    return select(d_endpoint, abs(d_ring), inside);
}

// The pick key this fragment reports: the per-instance key, written only where
// the SDF coverage is at least 0.5. With the triangle filter
// `coverage = saturate(1 - d / max(fwidth(d), world_per_px))`, `coverage >= 0.5`
// is exactly `d <= 0.5 * w`, i.e. the fragment centre is within **±0.5 px** of
// the analytic curve — the documented pick tolerance.
fn fs_key(in: VsOut) -> u32 {
    let d = conic_distance(in);
    let wpp = world_per_px();
    let w = select(fwidth(d), wpp, wpp > 0.0);
    let coverage = saturate(1.0 - d / max(w, 1e-8));
    if (coverage < 0.5) {
        discard;
    }
    return in.id_meta.x;
}
