// ID-pass instanced line pipeline.
//
// Draws the same instances `line_instanced.wgsl` draws (unit segment expanded by
// the per-instance buffer) with the same transform, but the fragment stage writes
// the per-instance pick key instead of a colour. Used only by the on-demand id
// pass; see `pick_line.wgsl` for the assembly contract.

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    key_flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) key: u32,
};

@vertex
fn vs_main(
    @location(0) unit: vec3<f32>,
    @location(1) p0_width: vec4<f32>,
    @location(2) p1_pad: vec4<f32>,
    @location(3) rgba: vec4<f32>,
    @location(4) id_flags: vec2<u32>,
) -> VsOut {
    let position = select(p0_width.xyz, p1_pad.xyz, unit.x > 0.5);
    var out: VsOut;
    out.clip = camera.transform * vec4<f32>(position, 1.0);
    out.key = id_flags.x;
    return out;
}

fn fs_key(in: VsOut) -> u32 {
    return in.key;
}
