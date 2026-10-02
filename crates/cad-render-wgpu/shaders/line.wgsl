// Line pipeline (polyline / wireframe overlay).
//
// Kept in sync with `LINE_SHADER` in `src/lib.rs` via `include_str!`. The
// uniform is `transform` (mat4x4) followed by `tint`; `tint.rgb` is the constant
// per-batch colour (normalized sRGB, sanitized by the scene/renderer) and
// `tint.a` is the constant per-batch alpha.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = constant per-batch colour; a = constant per-batch alpha.
    tint: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return camera.transform * vec4<f32>(position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(camera.tint.rgb, camera.tint.a);
}
