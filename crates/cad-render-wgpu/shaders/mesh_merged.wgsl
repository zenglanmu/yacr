// Merged-mesh pipeline (triangle list, depth tested, back-face culled).
//
// Used only by flat-merged mesh runs (see `build_mesh_chunks`): several adjacent
// same-pipeline opaque `Mesh` batches are concatenated into one static vertex
// buffer and drawn as a single indexed draw, so the dominant glyph-mesh draw
// class collapses. The shading body is byte-for-byte the same expression as
// `mesh.wgsl`; only the vertex input differs.
//
// Two things make the merged draw equivalent to the per-batch draws it replaces:
//
// * the per-batch constant tint is *baked* into the per-vertex colour at upload
//   (`tint.rgb * vertex_colour`, an f32 product), and the shared uniform tint is
//   written as opaque white, so `camera.tint.rgb * in.color` reproduces the
//   per-batch product. This is mathematically identical to the legacy per-pixel
//   `tint * colour`, but **not bit-identical in general**: the legacy shader
//   evaluates `tint.rgb * interpolate(colour)` per fragment while this evaluates
//   `interpolate(tint.rgb * colour)`. When the per-vertex colours vary across a
//   triangle (a gradient / HATCH ramp) and the tint is not white, the two orders
//   re-associate the f32 product and can round differently. Byte-identity is
//   therefore claimed only for a constant-colour mesh (the interpolated colour
//   is then exact and `1.0 * x == x`) or a white tint; a varying-colour ramp is
//   fenced by a pixel test with a documented tolerance (it is byte-identical on
//   the reference adapter, but that is empirical, not a guarantee);
// * the merged run only ever contains opaque batches (`alpha` clamps to exactly
//   `1.0`), so a single `tint.a` is correct for every member.
//
// `entity_flags` carries the low 32 bits of the source `EntityId` (from the
// batch's *first* source — a multi-source batch only exposes that one id here)
// and reserved flags per vertex. It is declared (so the pipeline supplies the
// attribute) but not yet consumed by this slice's shader, exactly like
// `line_instanced.wgsl`'s `id_flags`. Note this is new per-vertex metadata for a
// later picking phase; the legacy per-batch uniform never carried a pick key
// (identity was implicit in the draw).
//
// Kept in sync with `MESH_MERGED_SHADER` in `src/plan.rs` via `include_str!`.

struct Camera {
    transform: mat4x4<f32>,
    // rgb = constant per-batch colour (white for a merged chunk, whose colour is
    // baked per vertex); a = constant per-batch alpha (1.0 for merged chunks).
    tint: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // [0] = low 32 bits of the source `EntityId`; [1] = reserved flags. Not
    // consumed by this slice's shader.
    @location(3) entity_flags: vec2<u32>,
) -> VsOut {
    var out: VsOut;
    out.position = camera.transform * vec4<f32>(position, 1.0);
    out.world_normal = normal;
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let light_dir = vec3<f32>(0.0, 0.0, -1.0);
    let diffuse = max(dot(n, -light_dir), 0.0);
    let base = camera.tint.rgb * in.color;
    let lit = base * (0.25 + 0.75 * diffuse);
    return vec4<f32>(lit, camera.tint.a);
}
