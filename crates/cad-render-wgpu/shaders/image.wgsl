// Image pipeline (textured quad, triangle list, depth tested).
//
// Kept in sync with `IMAGE_SHADER` in `src/plan.rs` via `include_str!`.
//
// Group 0 is the shared camera uniform: `transform` maps world space to clip
// space, `tint` is the per-image colour/alpha factor and `key_flags.x` the
// image's renderer pick key. Group 1 is the selection-flags table (see below);
// group 2 is the raster image itself: a sampled `texture_2d<f32>` and a filtering
// sampler. The flags group precedes the texture group so every geometry pipeline
// in this crate binds the flags table at the same group index.
//
// There is deliberately no lighting model here. The fragment output is the
// sampled RGBA after the per-image display adjustment (monochrome, contrast,
// brightness, fade — a declared approximation, see `fs_main`) and then the
// `tint` factor (rgb * tint.rgb, a * tint.a); no physically based rendering,
// environment lighting or specular response is claimed.
//
// With `flags_ctl.enabled == 0` (the default) the output is exactly what this
// shader produced before the flags block existed.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = constant per-image colour factor; a = constant per-image alpha.
    tint: vec4<f32>,
    // x = renderer pick key, y = INSTANCE_FLAG_* bits, z = the image
    // brightness/contrast/fade pack, w = the image monochrome bit + background
    // colour pack (see `fs_main`). Every other pipeline reads z as its line AA
    // ramp / SDF margin and ignores w.
    key_flags: vec4<u32>,
    // rgb = unit vector toward the light, in world space (the same space as
    // `world_normal`); a = reserved 0. It is a per-frame camera property: the
    // 2D view keeps world +Z, the 3D view uses the reversed view direction.
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

@group(2) @binding(0) var image_texture: texture_2d<f32>;
@group(2) @binding(1) var image_sampler: sampler;

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
    let key = camera.key_flags.x;
    if (flags_ctl.enabled != 0u && (flag_bits(key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    let texel = textureSample(image_texture, image_sampler, in.uv);

    // Per-image display adjustment, decoded from the bytes the host packs into
    // `key_flags.z` / `key_flags.w`:
    //
    //   z = fade | contrast << 8 | brightness << 16   (each 0..=100, 8 bits)
    //   w = monochrome | bg_r << 8 | bg_g << 16 | bg_b << 24
    //
    // `bg_*` is the target's clear (background) colour as 8-bit channels, which
    // is what `fade` blends toward. Neutral values (50/50/0/off) are an exact
    // identity on the sampled texel.
    //
    // This is a DECLARED APPROXIMATION of AutoCAD's image transfer, not its
    // exact curve. The order is fixed: monochrome (luma) -> contrast ->
    // brightness -> fade. Each stage is applied in the sampled (linear) space
    // the texture and clear colour already use, so fade at 100 lands exactly on
    // the background colour.
    let fade = f32(camera.key_flags.z & 0xFFu) / 100.0;
    let contrast = f32((camera.key_flags.z >> 8u) & 0xFFu) / 50.0;
    let brightness = f32((camera.key_flags.z >> 16u) & 0xFFu) / 50.0;
    let monochrome = (camera.key_flags.w & 0xFFu) != 0u;
    let background = vec3<f32>(
        f32((camera.key_flags.w >> 8u) & 0xFFu) / 255.0,
        f32((camera.key_flags.w >> 16u) & 0xFFu) / 255.0,
        f32((camera.key_flags.w >> 24u) & 0xFFu) / 255.0,
    );

    var rgb = texel.rgb;
    if (monochrome) {
        rgb = vec3<f32>(dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722)));
    }
    rgb = (rgb - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5);
    rgb = clamp(rgb * brightness, vec3<f32>(0.0), vec3<f32>(1.0));
    rgb = mix(rgb, background, clamp(fade, 0.0, 1.0));

    return flag_tint(
        vec4<f32>(rgb * camera.tint.rgb, texel.a * camera.tint.a),
        key,
    );
}
