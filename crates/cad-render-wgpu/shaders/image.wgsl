// Image pipeline (textured quad, triangle list, depth tested).
//
// Kept in sync with `IMAGE_SHADER` in `src/plan.rs` via `include_str!`.
//
// Group 0 is the shared camera uniform: `transform` maps world space to clip
// space and `tint` is the per-image colour/alpha factor. Group 1 is the raster
// image itself: a sampled `texture_2d<f32>` and a filtering sampler.
//
// There is deliberately no lighting model here. The fragment output is the
// sampled RGBA modulated by `tint` (rgb * tint.rgb, a * tint.a); no physically
// based rendering, environment lighting or specular response is claimed.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = constant per-image colour factor; a = constant per-image alpha.
    tint: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

@group(1) @binding(0) var image_texture: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
) -> VsOut {
    var out: VsOut;
    out.position = camera.transform * vec4<f32>(position, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(image_texture, image_sampler, in.uv);
    return vec4<f32>(texel.rgb * camera.tint.rgb, texel.a * camera.tint.a);
}
