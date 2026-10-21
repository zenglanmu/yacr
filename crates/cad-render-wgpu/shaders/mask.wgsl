// Wipeout mask pass (ADR 0022 C1).
//
// A WIPEOUT must hide everything painted beneath it. The shared one-pass frame
// cannot do that with a background-coloured mesh: meshes use depth compare
// `Less` with depth writes and lines `LessEqual` with depth writes, so a filled
// quad drawn later at the same world z is *rejected* by the depth test. The mask
// therefore gets its own final pass:
//
//   * pipeline depth state: `depth_compare = Always`, `depth_write_enabled =
//     false` (see `Renderer::initialize_with_device`), so every mask fragment is
//     painted regardless of what is already in the depth buffer;
//   * the fragment output is the renderer's clear colour, carried in the
//     shared camera uniform's `color` field (the renderer owns the background;
//     `cad-representation` never names a colour);
//   * geometry is the mask polygon tessellated by the same even-odd fill solid
//     HATCH uses, so a concave boundary is filled exactly and does not leak.
//
// The draw order between masks and geometry is deliberately "masks last" for
// this slice (ADR 0022 D2): draw_order is not threaded to the representation
// layer yet, so no interleaving is claimed.
//
// Kept in sync with `MASK_SHADER` in `src/plan/shaders.rs` via `include_str!`.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = the clear colour this mask paints (linear, matching the render
    // target's clear value); a = 1.0 so the pass replaces the covered pixels.
    color: vec4<f32>,
    // x = renderer pick key, y = INSTANCE_FLAG_* bits, z/w reserved. Unused: a
    // mask is never pickable.
    key_flags: vec4<u32>,
    // rgb = unit vector toward the light, in world space (the same space as
    // `world_normal`); a = reserved 0. It is a per-frame camera property: the
    // 2D view keeps world +Z, the 3D view uses the reversed view direction.
    light: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return camera.transform * vec4<f32>(position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return camera.color;
}
