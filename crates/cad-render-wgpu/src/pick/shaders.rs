use super::*;

/// Fragment tail for the `R32Uint` id target: one `u32` channel carries the key.
pub(crate) const PICK_FS_U32: &str = r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) u32 {
    return fs_key(in);
}
"#;

/// Fragment tail for the packed RGBA8 id target: the key is split into four
/// `unorm8` channels, low byte in `r`. The division by 255 is exact for every
/// 8-bit value (each is representable in `f32` and the unorm8 conversion is
/// round-to-nearest), so the readback recovers the key bit-for-bit.
pub(crate) const PICK_FS_PACKED: &str = r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let key = fs_key(in);
    return vec4<f32>(
        f32(key & 0xFFu),
        f32((key >> 8u) & 0xFFu),
        f32((key >> 16u) & 0xFFu),
        f32((key >> 24u) & 0xFFu),
    ) / 255.0;
}
"#;

// Shader bodies: uniform + vertex stage + `fs_key`, assembled with a fragment
// tail above (see `pick.rs`'s module docs). Both tails are validated offline.
pub(crate) const PICK_LINE_SHADER: &str = include_str!("../../shaders/pick_line.wgsl");

pub(crate) const PICK_LINE_INSTANCED_SHADER: &str =
    include_str!("../../shaders/pick_line_instanced.wgsl");

pub(crate) const PICK_ARC_SHADER: &str = include_str!("../../shaders/pick_arc.wgsl");

pub(crate) const PICK_MESH_SHADER: &str = include_str!("../../shaders/pick_mesh.wgsl");

pub(crate) const PICK_MESH_MERGED_SHADER: &str =
    include_str!("../../shaders/pick_mesh_merged.wgsl");

pub(crate) const PICK_IMAGE_SHADER: &str = include_str!("../../shaders/pick_image.wgsl");

/// Assemble one id-pass shader module source for `format`.
pub fn pick_shader_source(body: &str, format: IdTargetFormat) -> String {
    let tail = match format {
        IdTargetFormat::R32Uint => PICK_FS_U32,
        IdTargetFormat::PackedRgba8 => PICK_FS_PACKED,
    };
    format!("{body}\n{tail}")
}

/// The id-pass shader bodies, for the offline WGSL validation test.
pub(crate) const PICK_SHADER_BODIES: [(&str, &str); 6] = [
    ("pick_line.wgsl", PICK_LINE_SHADER),
    ("pick_line_instanced.wgsl", PICK_LINE_INSTANCED_SHADER),
    ("pick_arc.wgsl", PICK_ARC_SHADER),
    ("pick_mesh.wgsl", PICK_MESH_SHADER),
    ("pick_mesh_merged.wgsl", PICK_MESH_MERGED_SHADER),
    ("pick_image.wgsl", PICK_IMAGE_SHADER),
];

/// Every id-pass shader module source assembled for `format`, as `(name,
/// source)` pairs.
///
/// The bodies alone are not complete WGSL modules: the fragment entry point is
/// generated per id-target format. Exposed so offline validation can check the
/// exact text the pipelines compile, including that tail.
pub fn pick_id_shader_sources(format: IdTargetFormat) -> Vec<(String, String)> {
    PICK_SHADER_BODIES
        .iter()
        .map(|(name, body)| ((*name).to_string(), pick_shader_source(body, format)))
        .collect()
}
