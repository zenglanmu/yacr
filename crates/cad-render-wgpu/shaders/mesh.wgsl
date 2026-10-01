// Mesh pipeline (triangle list, depth tested, back-face culled).
//
// Kept in sync with `MESH_SHADER` in `src/lib.rs` via `include_str!`.
//
// Shading is deliberately explicit and minimal: a fixed headlight along -Z
// gives an N.L diffuse term plus a constant ambient floor. No physically based
// rendering, environment lighting or specular response is claimed.
//
// Winding: the mirrored variant of this pipeline selects `front_face = Cw`;
// the shader is identical for both.

struct Camera {
    transform: mat4x4<f32>,
    // x = constant per-batch alpha; y..w reserved.
    tint: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
) -> VsOut {
    var out: VsOut;
    out.position = camera.transform * vec4<f32>(position, 1.0);
    out.world_normal = normal;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let light_dir = vec3<f32>(0.0, 0.0, -1.0);
    let diffuse = max(dot(n, -light_dir), 0.0);
    let base = vec3<f32>(0.72, 0.75, 0.80);
    let lit = base * (0.25 + 0.75 * diffuse);
    return vec4<f32>(lit, camera.tint.x);
}
