// ID-pass instanced line pipeline — anti-aliased SDF strokes.
//
// Draws the same instances `line_instanced.wgsl` draws (the shared unit quad
// expanded by the per-instance buffer) with the same transform and the same
// distance-to-segment SDF, but the fragment stage writes the per-instance pick
// key instead of a colour. A fragment is only written where the SDF coverage is
// at least 0.5, i.e. where the fragment centre is within **±0.5 px** of the
// analytic segment — the documented pick tolerance, matching `pick_arc.wgsl`.
// Used only by the on-demand id pass; see `pick_line.wgsl` for the assembly
// contract.

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // z = world units per pixel (bitcast f32; the SDF ramp width).
    key_flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

fn world_per_px() -> f32 {
    return bitcast<f32>(camera.key_flags.z);
}

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
    @location(3) @interpolate(flat) key: u32,
};

@vertex
fn vs_main(
    @location(0) unit: vec3<f32>,
    @location(1) p0_width: vec4<f32>,
    @location(2) p1_pad: vec4<f32>,
    @location(3) rgba: vec4<f32>,
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
    out.key = id_flags.x;
    return out;
}

fn fs_key(in: VsOut) -> u32 {
    let d = segment_distance(in.world_pos, in.p0, in.p1);
    let wpp = world_per_px();
    let w = select(fwidth(d), wpp, wpp > 0.0);
    let coverage = saturate(1.0 - d / max(w, 1e-8));
    if (coverage < 0.5) {
        discard;
    }
    return in.key;
}
