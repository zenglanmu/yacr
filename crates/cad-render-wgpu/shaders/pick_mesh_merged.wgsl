// ID-pass merged-mesh pipeline.
//
// Draws the same triangles `mesh_merged.wgsl` draws from the flat-merge
// interleaved layout, but writes the per-vertex pick key instead of a shaded
// colour. Used only by the on-demand id pass; see `pick_line.wgsl` for the
// assembly contract.

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    key_flags: vec4<u32>,
    // rgb = unit vector toward the light, in world space (the same space as
    // `world_normal`); a = reserved 0. It is a per-frame camera property: the
    // 2D view keeps world +Z, the 3D view uses the reversed view direction.
    light: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) key: u32,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) entity_flags: vec2<u32>,
) -> VsOut {
    var out: VsOut;
    out.clip = camera.transform * vec4<f32>(position, 1.0);
    out.key = entity_flags.x;
    return out;
}

fn fs_key(in: VsOut) -> u32 {
    return in.key;
}
