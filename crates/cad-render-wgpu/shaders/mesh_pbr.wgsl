// PBR mesh channel (ADR-0039 D6): metallic-roughness + normal/emissive/occlusion.
//
// Kept in sync with `MESH_PBR_SHADER` in `src/plan/shaders.rs` via
// `include_str!`. This is a **separate** channel from `mesh.wgsl` and
// `mesh_textured.wgsl`: only a batch whose `RenderBatch::pbr` is `Some` is drawn
// here, and the untextured/textured pipelines keep their exact previous layout
// and state.
//
// Vertex attributes:
//   slot 0 = position (`@location(0)`),
//   slot 1 = normal   (`@location(1)`),
//   slot 2 = colour   (`@location(2)`, white when the batch has no gradient),
//   slot 3 = UV0      (`@location(3)`, `Float32x2`),
//   slot 4 = tangent  (`@location(4)`, `Float32x4`, `w` = handedness).
//
// Two vertex entry points share one fragment entry point:
//   * `vs_main` consumes UV0 + tangent. The renderer uses it when the batch has
//     a UV stream **and** a CPU-generated tangent frame (so the normal map is
//     usable);
//   * `vs_uv` consumes UV0 but no tangent. It is used when the material samples
//     UV0 but has no tangent frame (no normal map, or degenerate UVs that could
//     not produce one): every other channel still samples its map, and the
//     normal-map channel is disabled instead of evaluated against an invented
//     frame;
//   * `vs_flat` consumes only position/normal/colour. It is used for a
//     **scalar-only** PBR material (RVT metallic/roughness with no UVs): the
//     pipeline then declares no UV/tangent buffer at all, so no UV or tangent is
//     fabricated for a mesh that has none.
//
// Texture groups (group 2): binding 0 = base colour, 1 = metallic-roughness,
// 2 = normal, 3 = emissive, 4 = occlusion, 5 = one filtering sampler. An absent
// channel is bound to an **exact multiplicative identity** 1x1 image:
// white for base colour / metallic-roughness / occlusion, black for emissive and
// flat `(0.5, 0.5, 1)` for the normal map. That is "channel disabled", not
// fabricated data; `pbr_factors.w` additionally carries the channel bitmask so
// the normal map is skipped (`vs_flat`, or a degenerate UV frame with no
// tangent) rather than evaluated against an invented frame.
//
// Colour space: base colour and emissive textures are uploaded as
// `Rgba8UnormSrgb` (the base-colour convention of ADR-0038), so sampling returns
// linear values. The metallic-roughness, normal and occlusion maps are uploaded
// as `Rgba8Unorm` — they are linear data, and sRGB-decoding them would corrupt
// the normal map. The output follows `mesh.wgsl`'s display convention: linear
// RGB written to an sRGB target, **no tonemap**, and the same near-white
// light-background ink flip.
//
// Shading (Cook-Torrance GGX, one directional light — `camera.light.rgb`, the
// same unit vector toward the light every mesh shader uses):
//   * `F0 = mix(vec3(0.04), baseColour, metallic)`;
//   * diffuse colour = `baseColour * (1 - metallic)`;
//   * `roughness = clamp(scale * mr.g, MIN_ROUGHNESS, 1)`,
//     `metallic = clamp(scale * mr.b, 0, 1)`;
//   * `occlusion` multiplies the diffuse term and the ambient term only;
//   * emissive `= emissiveFactor * emissiveTexture` is added;
//   * the result is multiplied by the vertex colour and the batch tint.
//
// The view vector is the eye position of the frame expressed in the batch's own
// vertex space (`camera.pbr_eye`), i.e. camera world position minus the batch
// local origin, so `eye - position` is exact for a perspective 3D camera. When
// the eye coincides with the surface (the orthographic 2D camera plane) it falls
// back to the `+Z` axis, which is the axis `camera.light` uses in 2D.
//
// Deliberately NOT implemented (ADR-0039 §3): image-based lighting, shadows,
// tonemapping, multiple lights, `KHR_materials_*` extensions, `alphaMode = MASK`
// cutout, UV1+ and `KHR_texture_transform`.
//
// WebGL2-safe: five `texture_2d<f32>` + one filtering sampler, an integer
// `texture_2d<u32>` for the flags table and plain vertex attributes — no storage
// buffers, no arrays of textures.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = constant per-batch colour; a = constant per-batch alpha.
    tint: vec4<f32>,
    // x = renderer pick key, y = INSTANCE_FLAG_* bits, z/w reserved.
    key_flags: vec4<u32>,
    // rgb = unit vector toward the light, in world space; a = 1 when the frame's
    // clear colour is light (the ink-flip flag).
    light: vec4<f32>,
    // ADR-0039 PBR payload: x = metallic, y = roughness, z = occlusion_strength,
    // w = PBR_CHANNEL_* bits as an exact small float (rounded back to u32).
    pbr_factors: vec4<f32>,
    // Linear emissive RGB (xyz), w unused.
    pbr_emissive: vec4<f32>,
    // Camera position in the batch's vertex space (xyz), w unused.
    pbr_eye: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

// --- selection flags (group 1) ---------------------------------------------
struct FlagsCtl {
    enabled: u32,
    width: u32,
    _pad0: u32,
    _pad1: u32,
};
@group(1) @binding(0) var flags_table: texture_2d<u32>;
@group(1) @binding(1) var<uniform> flags_ctl: FlagsCtl;

const FLAG_SELECTED: u32 = 1u;
const FLAG_HOVERED: u32 = 2u;
const FLAG_HIDDEN: u32 = 4u;
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

// --- PBR channel textures (group 2) ----------------------------------------
// Absent channels bind a 1x1 multiplicative identity (see the header).
@group(2) @binding(0) var base_colour_texture: texture_2d<f32>;
@group(2) @binding(1) var metallic_roughness_texture: texture_2d<f32>;
@group(2) @binding(2) var normal_texture: texture_2d<f32>;
@group(2) @binding(3) var emissive_texture: texture_2d<f32>;
@group(2) @binding(4) var occlusion_texture: texture_2d<f32>;
@group(2) @binding(5) var pbr_sampler: sampler;

// Mirror of `plan::uniforms::PBR_CHANNEL_*`.
const CHANNEL_BASE_COLOUR: u32 = 1u;
const CHANNEL_METALLIC_ROUGHNESS: u32 = 2u;
const CHANNEL_NORMAL: u32 = 4u;
const CHANNEL_EMISSIVE: u32 = 8u;
const CHANNEL_OCCLUSION: u32 = 16u;

const PI: f32 = 3.141592653589793;
// Ambient floor, the same 0.25 `mesh.wgsl` uses so a back-facing facet is never
// pure black. It is an ambient *term*, not image-based lighting.
const AMBIENT: f32 = 0.25;
// A fully smooth surface would divide by ~0 in the GGX denominator.
const MIN_ROUGHNESS: f32 = 0.04;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    // Position in the batch's vertex space, so `pbr_eye - world_position` is the
    // eye vector (both are reduced by the same batch origin).
    @location(1) world_position: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) tangent: vec3<f32>,
    // Handedness of the bitangent; flat so it is never interpolated between two
    // vertices with different frames.
    @location(5) @interpolate(flat) handedness: f32,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) tangent: vec4<f32>,
) -> VsOut {
    var out: VsOut;
    out.position = camera.transform * vec4<f32>(position, 1.0);
    out.world_normal = normal;
    out.world_position = position;
    out.color = color;
    out.uv = uv;
    out.tangent = tangent.xyz;
    out.handedness = tangent.w;
    return out;
}

// Scalar-only PBR material (no UV0, hence no texture channel and no tangent
// frame). Declares the same outputs so one fragment entry point serves both.
@vertex
fn vs_flat(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
) -> VsOut {
    var out: VsOut;
    out.position = camera.transform * vec4<f32>(position, 1.0);
    out.world_normal = normal;
    out.world_position = position;
    out.color = color;
    out.uv = vec2<f32>(0.0, 0.0);
    out.tangent = vec3<f32>(1.0, 0.0, 0.0);
    out.handedness = 1.0;
    return out;
}

// UV0 but no tangent frame: the texture channels sample their maps, the normal
// map is disabled by its channel bit (a tangent is never invented).
@vertex
fn vs_uv(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
) -> VsOut {
    var out: VsOut;
    out.position = camera.transform * vec4<f32>(position, 1.0);
    out.world_normal = normal;
    out.world_position = position;
    out.color = color;
    out.uv = uv;
    out.tangent = vec3<f32>(1.0, 0.0, 0.0);
    out.handedness = 1.0;
    return out;
}

// --- light-background ink flip (identical rule to `mesh.wgsl`) --------------
fn ink_for_background(rgb: vec3<f32>) -> vec3<f32> {
    if (camera.light.a > 0.5 && min(rgb.r, min(rgb.g, rgb.b)) >= 0.75) {
        return vec3<f32>(0.0);
    }
    return rgb;
}

// --- Cook-Torrance terms ---------------------------------------------------
fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denominator = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / max(PI * denominator * denominator, 1e-7);
}

fn geometry_schlick_ggx(n_dot_x: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return n_dot_x / (n_dot_x * (1.0 - k) + k);
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let key = camera.key_flags.x;
    if (flags_ctl.enabled != 0u && (flag_bits(key) & FLAG_HIDDEN) != 0u) {
        discard;
    }

    let channels = u32(round(clamp(camera.pbr_factors.w, 0.0, 31.0)));

    // Every channel is sampled unconditionally: an absent one is the 1x1
    // multiplicative identity, so the product is exactly neutral.
    let base_sample = textureSample(base_colour_texture, pbr_sampler, in.uv);
    let mr_sample = textureSample(metallic_roughness_texture, pbr_sampler, in.uv);
    let normal_sample = textureSample(normal_texture, pbr_sampler, in.uv);
    let emissive_sample = textureSample(emissive_texture, pbr_sampler, in.uv);
    let occlusion_sample = textureSample(occlusion_texture, pbr_sampler, in.uv);

    let base_colour = select(vec3<f32>(1.0), base_sample.rgb, (channels & CHANNEL_BASE_COLOUR) != 0u);
    let metallic = clamp(
        camera.pbr_factors.x * select(1.0, mr_sample.b, (channels & CHANNEL_METALLIC_ROUGHNESS) != 0u),
        0.0,
        1.0,
    );
    let roughness = clamp(
        camera.pbr_factors.y * select(1.0, mr_sample.g, (channels & CHANNEL_METALLIC_ROUGHNESS) != 0u),
        MIN_ROUGHNESS,
        1.0,
    );
    // glTF: `ao = 1 + strength * (sample - 1)`.
    let occlusion = 1.0 + camera.pbr_factors.z * (occlusion_sample.r - 1.0);
    let emissive = camera.pbr_emissive.rgb
        * select(vec3<f32>(0.0), emissive_sample.rgb, (channels & CHANNEL_EMISSIVE) != 0u);

    var n = normalize(in.world_normal);
    let have_tangent = (channels & CHANNEL_NORMAL) != 0u;
    if (have_tangent) {
        // Tangent-space normal mapped through the per-vertex frame. `w` rebuilds
        // the bitangent handedness without a second vertex attribute.
        let raw = in.tangent - n * dot(n, in.tangent);
        // Guard a (theoretically impossible) tangent parallel to the normal: a
        // zero vector would make `normalize` non-finite.
        let t = select(vec3<f32>(1.0, 0.0, 0.0), normalize(raw), dot(raw, raw) > 1e-8);
        let b = cross(n, t) * in.handedness;
        let tangent_normal = normal_sample.xyz * 2.0 - vec3<f32>(1.0);
        n = normalize(t * tangent_normal.x + b * tangent_normal.y + n * tangent_normal.z);
    }

    let light = normalize(camera.light.xyz);
    let to_eye = camera.pbr_eye.xyz - in.world_position;
    var v = vec3<f32>(0.0, 0.0, 1.0);
    if (dot(to_eye, to_eye) > 1e-12) {
        v = normalize(to_eye);
    }
    // `light + v` is zero only when the light points exactly opposite the view,
    // which the frame's key light never does; guard the normalise anyway so a
    // non-finite half-vector can never reach the shading terms.
    let half_sum = light + v;
    let h = select(
        vec3<f32>(0.0, 0.0, 1.0),
        normalize(half_sum),
        dot(half_sum, half_sum) > 1e-12,
    );

    let n_dot_l = max(dot(n, light), 0.0);
    let n_dot_v = max(dot(n, v), 1e-4);
    let n_dot_h = max(dot(n, h), 0.0);
    let v_dot_h = max(dot(v, h), 0.0);

    let diffuse_colour = base_colour * (1.0 - metallic);
    let f0 = mix(vec3<f32>(0.04), base_colour, metallic);
    let fresnel = fresnel_schlick(v_dot_h, f0);
    let distribution = distribution_ggx(n_dot_h, roughness);
    let geometry = geometry_schlick_ggx(n_dot_v, roughness)
        * geometry_schlick_ggx(n_dot_l, roughness);
    let specular = (distribution * geometry * fresnel) / max(4.0 * n_dot_v * n_dot_l, 1e-4);

    // Occlusion darkens the diffuse and ambient terms only (never the specular
    // reflection, which is a surface response, not an indirect one).
    let direct_diffuse = diffuse_colour * n_dot_l * occlusion;
    let ambient = diffuse_colour * AMBIENT * occlusion;
    let lit = direct_diffuse + specular + ambient + emissive;

    let rgb = lit * in.color * camera.tint.rgb;
    let alpha = base_sample.a * camera.tint.a;
    let shaded = flag_tint(vec4<f32>(rgb, alpha), key);
    return vec4<f32>(ink_for_background(shaded.rgb), shaded.a);
}
