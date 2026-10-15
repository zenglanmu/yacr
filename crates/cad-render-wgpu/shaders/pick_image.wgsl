// ID-pass image pipeline.
//
// Draws the same textured quad `image.wgsl` draws (same transform, no culling)
// but writes the image's renderer pick key instead of a sampled colour. The
// texture group is still declared so the pipeline can share the image bind group
// layout; it is never sampled here. Used only by the on-demand id pass; see
// `pick_line.wgsl` for the assembly contract.

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // x = this image's renderer pick key, y = INSTANCE_FLAG_* bits.
    key_flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

@group(1) @binding(0) var image_texture: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
) -> VsOut {
    var out: VsOut;
    out.clip = camera.transform * vec4<f32>(position, 1.0);
    return out;
}

fn fs_key(in: VsOut) -> u32 {
    return camera.key_flags.x;
}
