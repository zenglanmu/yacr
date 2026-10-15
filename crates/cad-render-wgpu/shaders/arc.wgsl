// SDF conic pipeline (slice B): one instance per analytic circle/arc.
//
// The shared unit quad (slot 0, `[0,1]^2`) is expanded to the conic's
// full-circle extent in the instance's `u`/`v` plane; the fragment shader
// computes the distance to the ring (or, off the arc's angular span, to its
// nearest endpoint) and discards outside the anti-aliased band.
//
// One instance = one bounding quad, so the mandatory `discard` is what keeps an
// arc's quad from writing depth and erasing the geometry behind it.
//
// The per-instance data is plain instance-rate vertex attributes (no vertex-stage
// storage buffer), so this pipeline is WebGL2-safe. See
// `docs/adr/0008-sdf-conic-rendering.md`.
//
// Kept in sync with `ARC_SHADER` in `src/plan.rs` via `include_str!`.

struct Camera {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // x = renderer pick key (unused: the per-instance key wins), y = flags,
    // z = world units per pixel (bitcast f32; the SDF ramp width), w reserved.
    key_flags: vec4<u32>,
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
// Mechanical placeholder tints, matching `line_instanced.wgsl`; the final
// selection visual system is owned by the follow-up UI slice.
const SELECTED_TINT: vec3<f32> = vec3<f32>(1.0, 0.55, 0.12);
const HOVERED_TINT: vec3<f32> = vec3<f32>(0.30, 0.85, 1.0);

// Reused instance-flag bit: this conic is a closed full circle.
const ARC_FLAG_FULL_CIRCLE: u32 = 8u;
const TAU: f32 = 6.283185307179586;
// Relative world-space margin added to the quad so the anti-aliased outer fringe
// is not clipped at the quad boundary. The SDF discards the overdraw. The margin
// is applied in **world space** (not clip space): a clip-space-only expansion
// stretches the interpolated `world_pos` field and moves the ring outward.
//
// A pure percentage under-covers small on-screen circles: the fringe is an
// absolute ~1 px band while `0.02 * r` shrinks with the radius, so an on-screen
// radius below ~50 px loses the fringe at the four axis points (worse zoomed
// out). The vertex stage therefore floors the margin at ~2 world pixels using
// the camera's world-per-pixel (`camera.key_flags.z`); the relative term still
// wins for large circles.
const QUAD_MARGIN_REL: f32 = 0.02;
// World units per pixel, passed through the otherwise-unused `key_flags.z` word
// as `bitcast<f32>`; 0.0 when unset, which makes the floor below a no-op.
fn world_per_px() -> f32 {
    return bitcast<f32>(camera.key_flags.z);
}

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

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // World-space position of this fragment (the quad's plane), for the SDF.
    @location(0) world_pos: vec3<f32>,
    // xyz = center (stream-relative), w = major radius.
    @location(1) center_r: vec4<f32>,
    // xyz = unit major direction, w = start angle (rad).
    @location(2) uvec: vec4<f32>,
    // xyz = minor semi-diameter vector, w = signed sweep (rad).
    @location(3) vvec: vec4<f32>,
    // rgb = colour, a = alpha.
    @location(4) color: vec4<f32>,
    // [0] = pick key, [1] = INSTANCE_FLAG_* | ARC_FLAG_FULL_CIRCLE.
    @location(5) @interpolate(flat) id_meta: vec2<u32>,
};

@vertex
fn vs_main(
    // Shared unit quad in [0, 1]^2.
    @location(0) unit: vec3<f32>,
    // xyz = center (stream-relative), w = major radius.
    @location(1) center_r: vec4<f32>,
    // xyz = unit major direction, w = start angle.
    @location(2) uvec: vec4<f32>,
    // xyz = minor semi-diameter vector, w = signed sweep.
    @location(3) vvec: vec4<f32>,
    // Sanitised linear sRGB colour (rgb) and clamped alpha (a).
    @location(4) rgba: vec4<f32>,
    // [0] = renderer pick key, [1] = INSTANCE_FLAG_* | ARC_FLAG_FULL_CIRCLE.
    @location(5) id_flags: vec2<u32>,
) -> VsOut {
    let corner = unit.xy * 2.0 - 1.0;
    let r = center_r.w;
    // Relative margin, floored so the ~1 px anti-aliased fringe always fits
    // inside the quad even for a small on-screen circle.
    let margin = max(QUAD_MARGIN_REL, 2.0 * world_per_px() / max(r, 1e-8));
    let grow = 1.0 + margin;
    let world =
        center_r.xyz + corner.x * (grow * center_r.w * uvec.xyz) + corner.y * (grow * vvec.xyz);
    let clip = camera.transform * vec4<f32>(world, 1.0);
    var out: VsOut;
    out.clip = clip;
    out.world_pos = world;
    out.center_r = center_r;
    out.uvec = uvec;
    out.vvec = vvec;
    out.color = rgba;
    out.id_meta = id_flags;
    return out;
}

fn wrap_tau(a: f32) -> f32 {
    return (a / TAU - floor(a / TAU)) * TAU;
}

// Distance to the conic, computed in uniform control flow so `fwidth` below is
// well defined. The field is a true distance field (unit gradient) except at the
// arc's endpoint kink.
fn conic_distance(in: VsOut) -> f32 {
    let r = in.center_r.w;
    let q = in.world_pos - in.center_r.xyz;
    let x = dot(q, in.uvec.xyz);
    // `vvec` is carried as its full semi-diameter vector; its direction gives the
    // slice-1 circle's second axis.
    let y = dot(q, normalize(in.vvec.xyz));
    let d_ring = length(vec2<f32>(x, y)) - r;
    let start = in.uvec.w;
    let sweep = in.vvec.w;
    let full = (in.id_meta.y & ARC_FLAG_FULL_CIRCLE) != 0u;
    let t = atan2(y, x);
    let rel = wrap_tau(t - start);
    let inside = full || select(TAU - rel <= -sweep, rel <= sweep, sweep >= 0.0);
    let e0 = vec2<f32>(r * cos(start), r * sin(start));
    let e1 = vec2<f32>(r * cos(start + sweep), r * sin(start + sweep));
    let d_endpoint = min(
        length(vec2<f32>(x, y) - e0),
        length(vec2<f32>(x, y) - e1),
    );
    // The two branches are equal on the boundary ray, so this is continuous
    // across the angular boundary; `select` keeps it branchless.
    return select(d_endpoint, abs(d_ring), inside);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = conic_distance(in);
    // The distance field has unit gradient, so one screen pixel is exactly
    // `world_per_px` world units. Use that as the ramp width: `fwidth(d)` sums
    // |∂d/∂x|+|∂d/∂y| (up to 1.414x too wide) and, worse, collapses to ~0 where
    // the ring straddles a pixel pair symmetrically — which would discard the
    // four axis points of a grid-aligned circle. `fwidth` is only the fallback
    // when the uniform word is unset.
    let wpp = world_per_px();
    let w = select(fwidth(d), wpp, wpp > 0.0);
    // Triangle (linear) filter over the distance-to-curve field: full intensity
    // at the curve, ramping to zero one pixel out. (The old
    // `saturate(0.5 - d / w)` is the filled-shape boundary formula and caps the
    // peak at 0.5 — a half-intensity, dashed hairline.)
    //
    // Ink semantics: the triangle integrates to exactly its half-width `w`
    // (`∫ saturate(1 - |d|/w) dd = w`), so `w = world_per_px` gives **1.0 px of
    // ink** — byte-for-byte the visual weight of the hard-aliased 1 px straight
    // lines. The peak is phase-dependent (0.5 between two pixel rows/columns,
    // 1.0 through a pixel centre); that is correct anti-aliasing — peak and ink
    // cannot both be had. Integrating the sRGB-encoded channel instead of this
    // linear coverage inflates the soft stroke by ~√2; measure ink linearly.
    let coverage = saturate(1.0 - d / max(w, 1e-8));
    // Mandatory: one instance = one bounding quad; without this discard every
    // quad pixel would write depth and erase the geometry behind it.
    if (coverage <= 0.0) {
        discard;
    }
    let key = in.id_meta.x;
    if (flags_ctl.enabled != 0u && (flag_bits(key) & FLAG_HIDDEN) != 0u) {
        discard;
    }
    return flag_tint(vec4<f32>(in.color.rgb, in.color.a * coverage), key);
}
