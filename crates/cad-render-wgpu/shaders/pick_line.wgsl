// ID-pass line pipeline (legacy per-batch `LineList`).
//
// This shader draws exactly the geometry `line.wgsl` draws, with exactly the
// same transform and primitive state, but the fragment stage writes the batch's
// renderer pick key instead of a colour. It is used only by the on-demand id
// pass (`Renderer::pick_at`), never by a colour frame.
//
// The module is **assembled** at pipeline creation: this file provides the
// uniform, the vertex stage and `fs_key`, and a fragment tail declares
// `@fragment fn fs_main(...)` for the active id-target format (see `pick.rs`).
// Both tails are validated offline (see `tests/wgsl_validation.rs`).

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // x = this batch's renderer pick key, y = INSTANCE_FLAG_* bits.
    key_flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> VsOut {
    var out: VsOut;
    out.clip = camera.transform * vec4<f32>(position, 1.0);
    return out;
}

// The pick key this fragment reports: the batch's own key from its uniform
// (a legacy per-batch draw is never coalesced, so the uniform is exact).
fn fs_key(in: VsOut) -> u32 {
    return camera.key_flags.x;
}
