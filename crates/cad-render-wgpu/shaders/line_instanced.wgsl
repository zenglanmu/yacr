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
// Kept in sync with `LINE_INSTANCED_SHADER` in `src/plan.rs` via `include_str!`.
// Group 0 is the shared geometry uniform; the instanced path only consumes
// `transform` (stream-relative world -> clip). `tint` is unused here but kept so
// the instanced pipeline can share the geometry uniform's bind group layout.

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
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
    // First-class pick key: [0] is the low 32 bits of the source `EntityId`,
    // [1] is a reserved flags bitfield. Not consumed by this slice's shader;
    // declared so the pipeline supplies the attribute the later picking phase
    // will read.
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
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
