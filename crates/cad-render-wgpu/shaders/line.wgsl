// Line pipeline (polyline / wireframe overlay) — anti-aliased SDF strokes.
//
// Visual unification (see `docs/adr/0009-antialiased-line-strokes.md`): every
// line-like primitive is drawn as an anti-aliased quad stroke with the **same**
// semantics as the SDF conics (`arc.wgsl`): a triangle (linear) coverage filter
// over the distance-to-segment field, `coverage = saturate(1 - d / w)` with
// `w = world_per_px`, i.e. exactly **1.0 px of linear ink** — the same visual
// weight the hard-aliased 1 px `LineList` used to carry, now phase-independent
// and free of stair-stepping.
//
// The legacy per-batch path uploads the line geometry **already expanded** into
// triangle-list quads (one quad = six vertices per 2-vertex segment, see
// `plan::expand_line_quads`), each vertex carrying the segment endpoints `p0`/`p1`
// and a quad corner in `[-1, 1]^2`. The vertex stage rebuilds the world-space
// quad exactly like the instanced shader (`line_instanced.wgsl`), so the two
// conversion paths rasterize the same fragments ("conversion-path equivalence").
//
// Kept in sync with `LINE_SHADER` in `src/plan.rs` via `include_str!`. The
// uniform is `transform` (mat4x4) followed by `tint` and `key_flags`; `tint.rgb`
// is the constant per-batch colour (normalized sRGB, sanitized by the
// scene/renderer) and `tint.a` is the constant per-batch alpha.
//
// `key_flags.x` is the batch's renderer pick key (see `PickTable`); it is read by
// the **selection-flags** block below (group 1). `key_flags.z` carries the world
// units per pixel (`bitcast<f32>`, 0 when unset) — the SDF ramp width.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = constant per-batch colour; a = constant per-batch alpha.
    tint: vec4<f32>,
    // x = renderer pick key, y = INSTANCE_FLAG_* bits, z = world units per pixel
    // (bitcast f32; the SDF ramp width), w reserved.
    key_flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

// --- selection flags (group 1) ---------------------------------------------
// A small integer table indexed by the pick key: one texel per resident batch
// (packed into rows when the key space exceeds the texture width). `enabled` is
// the global toggle; while it is 0 the shader ignores the table entirely.
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

// World units per pixel, passed through the otherwise-unused `key_flags.z` word
// as `bitcast<f32>`; 0.0 when unset, which selects the `fwidth` fallback below.
fn world_per_px() -> f32 {
    return bitcast<f32>(camera.key_flags.z);
}

// Expand a segment `p0 -> p1` around the unit-quad `corner` (in `[-1, 1]^2`) into
// a world-space quad that fully covers the anti-aliased band. Shared verbatim
// with `line_instanced.wgsl` / `line_pulled.wgsl` (kept in sync by hand so the
// conversion paths agree). The band is an absolute ~1 px fringe, so the margin is
// floored at ~2 world pixels rather than being purely relative.
fn segment_quad_world(p0: vec3<f32>, p1: vec3<f32>, corner: vec2<f32>, wpp: f32) -> vec3<f32> {
    let seg = p1 - p0;
    let len = length(seg);
    let dir = select(vec3<f32>(1.0, 0.0, 0.0), seg / len, len > 1e-8);
    // Any reference axis not parallel to `dir` gives a valid perpendicular.
    // Perpendicular in the drawing plane (`xy`), matching the SDF conics'
    // planar world-space expansion; fall back to the `y` axis for a segment
    // along `z`, whose in-plane perpendicular is undefined.
    let planar = vec2<f32>(-dir.y, dir.x);
    let normal = select(
        vec3<f32>(0.0, 1.0, 0.0),
        normalize(vec3<f32>(planar, 0.0)),
        length(planar) > 1e-4,
    );
    let margin = max(2.0 * wpp, 1e-6);
    // Perpendicular half-extent covers the ramp width plus the fringe margin;
    // the along-axis half-extent additionally covers the round endpoint caps.
    let half = wpp + margin;
    let along = 0.5 * len + half;
    let center = 0.5 * (p0 + p1);
    return center + corner.x * dir * along + corner.y * normal * half;
}

// Distance to the segment `a -> b` (round caps), the unit-gradient field the
// triangle filter ramps over. Identical to the instanced/pulled/pick copies.
fn segment_distance(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>) -> f32 {
    let ab = b - a;
    let denom = dot(ab, ab);
    let t = select(0.0, clamp(dot(p - a, ab) / denom, 0.0, 1.0), denom > 1e-12);
    return length(p - (a + t * ab));
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // World-space position of this fragment (the quad's plane), for the SDF.
    @location(0) world_pos: vec3<f32>,
    // Segment endpoints (identical across the quad's six vertices).
    @location(1) @interpolate(flat) p0: vec3<f32>,
    @location(2) @interpolate(flat) p1: vec3<f32>,
};

@vertex
fn vs_main(
    // xyz = segment start (batch-local), the segment this quad belongs to.
    @location(0) p0: vec3<f32>,
    // xyz = segment end (batch-local).
    @location(1) p1: vec3<f32>,
    // Quad corner in `[0, 1]^2`.
    @location(2) corner_uv: vec2<f32>,
) -> VsOut {
    let corner = corner_uv * 2.0 - 1.0;
    let world = segment_quad_world(p0, p1, corner, world_per_px());
    var out: VsOut;
    out.clip = camera.transform * vec4<f32>(world, 1.0);
    out.world_pos = world;
    out.p0 = p0;
    out.p1 = p1;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = segment_distance(in.world_pos, in.p0, in.p1);
    // One screen pixel is exactly `world_per_px` world units (unit-gradient
    // field); `fwidth` is only the fallback when the uniform word is unset.
    let wpp = world_per_px();
    let w = select(fwidth(d), wpp, wpp > 0.0);
    // Triangle (linear) filter: full intensity on the segment, ramping to zero
    // one pixel out. Integrates to exactly `w` = 1 px of linear ink.
    let coverage = saturate(1.0 - d / max(w, 1e-8));
    // Mandatory: the quad overdraws; without this discard every quad pixel would
    // write depth and erase the geometry behind it.
    if (coverage <= 0.0) {
        discard;
    }
    let key = camera.key_flags.x;
    if (flags_ctl.enabled != 0u && (flag_bits(key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    return flag_tint(vec4<f32>(camera.tint.rgb, camera.tint.a * coverage), key);
}
