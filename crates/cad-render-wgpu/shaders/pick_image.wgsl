// ID-pass image pipeline.
//
// Draws the same textured quad `image.wgsl` draws (same transform, no culling)
// but writes the image's renderer pick key instead of a sampled colour. The
// pipeline shares the color image pipeline's group order (camera, flags table,
// sampled texture), so the texture stays at `@group(2)`; the texture and the
// flags table are declared for layout compatibility but never read here (the id
// pass writes a raw key, not a tinted colour). Used only by the on-demand id
// pass; see `pick_line.wgsl` for the assembly contract.

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // x = this image's renderer pick key, y = INSTANCE_FLAG_* bits.
    key_flags: vec4<u32>,
    // rgb = unit vector toward the light, in world space (the same space as
    // `world_normal`); a = reserved 0. It is a per-frame camera property: the
    // 2D view keeps world +Z, the 3D view uses the reversed view direction.
    light: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

@group(2) @binding(0) var image_texture: texture_2d<f32>;
@group(2) @binding(1) var image_sampler: sampler;

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
