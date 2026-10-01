// Line pipeline (polyline / wireframe overlay).
//
// Kept in sync with `LINE_SHADER` in `src/lib.rs` via `include_str!`. The
// uniform is `transform` (mat4x4) followed by `tint`; `tint.x` is the constant
// per-batch alpha (1.0 until entity transparency is carried by the scene).

struct Camera {
    transform: mat4x4<f32>,
    // x = constant per-batch alpha; y..w reserved.
    tint: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return camera.transform * vec4<f32>(position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.85, 0.88, 0.92, camera.tint.x);
}
