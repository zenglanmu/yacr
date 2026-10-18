/// The probe shader: a fullscreen triangle with a grid, a translating colour
/// block and two corner markers. It deliberately draws no CAD geometry.
/// Fullscreen textured shader: samples a native CAD texture directly to the
/// surface (no CPU readback, no extra CAD surface). Opaque display; the CAD
/// texture is expected to be opaque (premultiplied alpha is not used here).
pub(crate) const TEXTURE_SHADER: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var cad_texture: texture_2d<f32>;
@group(0) @binding(1) var cad_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let xy = corners[vertex_index];
    var out: VsOut;
    out.position = vec4<f32>(xy, 0.0, 1.0);
    // Texture origin is top-left: clip y=+1 is the screen top, so v must be
    // (1 - y)/2. Getting this wrong flips the image vertically.
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(cad_texture, cad_sampler, in.uv);
}
"#;

pub(crate) const PROBE_SHADER: &str = r#"
struct ProbeParams {
    resolution: vec2<f32>,
    marker: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> params: ProbeParams;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let xy = corners[vertex_index];
    var out: VsOut;
    out.position = vec4<f32>(xy, 0.0, 1.0);
    out.uv = (xy + vec2<f32>(1.0, 1.0)) * 0.5;
    return out;
}

fn grid_line(coord: f32) -> f32 {
    let width = fwidth(coord);
    let distance = abs(fract(coord) - 0.5);
    return 1.0 - smoothstep(0.5 - width * 1.5, 0.5, distance);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let divisions = 10.0;
    let grid = clamp(
        grid_line(in.uv.x * divisions) + grid_line(in.uv.y * divisions),
        0.0,
        1.0,
    );
    var color = mix(vec3<f32>(0.06, 0.07, 0.10), vec3<f32>(0.30, 0.35, 0.45), grid);

    // Translating colour block: identifies frame progress, not drawing content.
    let marker = vec2<f32>(params.marker, 0.5);
    let block = step(abs(in.uv.x - marker.x), 0.06) * step(abs(in.uv.y - marker.y), 0.06);
    color = mix(color, vec3<f32>(1.0, 0.55, 0.10), block);

    // Static corner markers for an orientation check.
    let top_left = step(in.uv.x, 0.06) * step(in.uv.y, 0.06);
    let bottom_right = step(0.94, in.uv.x) * step(0.94, in.uv.y);
    color = mix(color, vec3<f32>(0.90, 0.10, 0.10), top_left);
    color = mix(color, vec3<f32>(0.10, 0.80, 0.30), bottom_right);

    return vec4<f32>(color, 1.0);
}
"#;
