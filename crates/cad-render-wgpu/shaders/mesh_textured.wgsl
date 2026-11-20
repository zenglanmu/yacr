// Textured mesh pipeline (triangle list, depth tested, back-face culled).
//
// Kept in sync with `MESH_TEXTURED_SHADER` in `src/plan/shaders.rs` via
// `include_str!`. This is a **separate** pipeline from `mesh.wgsl`: the
// untextured mesh channels keep their exact previous layout and state
// (ADR-0038 D9), and only a batch that carries a base-colour texture and UV0 is
// drawn here.
//
// Vertex attributes: position (`@location(0)`), normal (`@location(1)`), the
// per-vertex colour (`@location(2)`, white when the batch has no gradient) and
// UV0 (`@location(3)`, `Float32x2`, only ever supplied with a texture).
//
// Groups: 0 = the shared camera uniform (`transform` maps world space to clip
// space, `tint.rgb` is the per-batch colour, `tint.a` the per-batch alpha,
// `key_flags.x` the renderer pick key); 1 = the selection-flags table, at the
// same group index as every other colour pipeline; 2 = the sampled texture and
// its filtering sampler (the same bind-group layout the raster-image pipeline
// uses, so both share one texture allocation path).
//
// Colour: `sampled.rgb * in.color * camera.tint.rgb`, alpha
// `sampled.a * camera.tint.a`.
//
// There is deliberately **no lighting term**: the sampled texel is authored
// albedo for a base-colour map, and multiplying it by the N·L diffuse term of
// `mesh.wgsl` would darken a texture the source did not ask to shade. This is a
// declared display convention, not physically based rendering; normal and other
// PBR channels are out of scope (ADR-0038 §7).
//
// Not implemented here, and therefore not claimed: `alphaMode = MASK` cutout
// (needs a `discard` threshold and the importer records the loss as
// `import.gltf.alpha_mask_not_imported`), UV1+, per-corner UVs and
// `KHR_texture_transform`.
//
// WebGL2-safe: one `texture_2d<f32>`, one filtering sampler, an integer
// `texture_2d<u32>` for the flags table and plain vertex attributes — no storage
// buffers, no arrays of textures.
//
// `camera.key_flags.x` is the batch's renderer pick key, read by the selection
// flags block (group 1). With `flags_ctl.enabled == 0` (default) the output is
// exactly the sampled-and-tinted product.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = constant per-batch colour; a = constant per-batch alpha.
    tint: vec4<f32>,
    // x = renderer pick key, y = INSTANCE_FLAG_* bits, z/w reserved.
    key_flags: vec4<u32>,
    // rgb = unit vector toward the light, in world space; a = 1 when the frame's
    // clear colour is light (the ink-flip flag). Kept in the uniform so the
    // layout is shared with every other pipeline; this shader only reads `.a` to
    // document that a textured surface is not ink-flipped (see `fs_main`).
    light: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

// --- selection flags (group 1) ---------------------------------------------
struct FlagsCtl {
    enabled: u32,
    // Texels per row; the key is addressed as (key % width, key / width).
    width: u32,
    _pad0: u32,
    _pad1: u32,
};
@group(1) @binding(0) var flags_table: texture_2d<u32>;
@group(1) @binding(1) var<uniform> flags_ctl: FlagsCtl;

const FLAG_SELECTED: u32 = 1u;
const FLAG_HOVERED: u32 = 2u;
const FLAG_HIDDEN: u32 = 4u;
// Mechanical placeholder tints so the toggle is observable; the final selection
// visual system is owned by the follow-up UI slice.
const SELECTED_TINT: vec3<f32> = vec3<f32>(1.0, 0.55, 0.12);
const HOVERED_TINT: vec3<f32> = vec3<f32>(0.30, 0.85, 1.0);

fn flag_bits(key: u32) -> u32 {
    let k = i32(key);
    let w = i32(flags_ctl.width);
    let xy = vec2<i32>(k % w, k / w);
    return textureLoad(flags_table, xy, 0).r;
}

fn flag_tint(color: vec4<f32>, key: u32) -> vec4<f32> {
    if (flags_ctl.enabled == 0u) {
        return color;
    }
    let bits = flag_bits(key);
    if ((bits & FLAG_SELECTED) != 0u) {
        return vec4<f32>(SELECTED_TINT, color.a);
    }
    if ((bits & FLAG_HOVERED) != 0u) {
        return vec4<f32>(HOVERED_TINT, color.a);
    }
    return color;
}

// --- base-colour texture (group 2) ------------------------------------------
@group(2) @binding(0) var mesh_texture: texture_2d<f32>;
@group(2) @binding(1) var mesh_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    // Part of the mesh vertex layout (one buffer per attribute, exactly as the
    // untextured mesh pipeline). This shader is deliberately unlit, so the normal
    // is accepted and not consumed; keeping it in the signature documents that
    // slot 1 is intentional rather than accidentally missing.
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
) -> VsOut {
    var out: VsOut;
    out.position = camera.transform * vec4<f32>(position, 1.0);
    out.color = color;
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let key = camera.key_flags.x;
    if (flags_ctl.enabled != 0u && (flag_bits(key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    let sampled = textureSample(mesh_texture, mesh_sampler, in.uv);
    // Batch tint (rgb) and alpha; the per-vertex colour is a gradient factor and
    // carries no alpha channel of its own, so alpha is the texture's times the
    // batch's.
    let rgb = sampled.rgb * in.color * camera.tint.rgb;
    let alpha = sampled.a * camera.tint.a;
    return flag_tint(vec4<f32>(rgb, alpha), key);
}
