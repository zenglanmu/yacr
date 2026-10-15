//! Offline WGSL validation (spec §5.2, audit F14).
//!
//! Every shader file is parsed and validated with the same `naga` version that
//! `wgpu 30.0.1` embeds, so a syntax or type error in a shader fails this test
//! without needing a GPU. This is static validation only: it does not compile
//! the pipelines or execute a draw.

use naga::valid::{Capabilities, ValidationFlags, Validator};

use cad_render_wgpu::{pick_id_shader_sources, IdTargetFormat};

fn validate_wgsl(name: &str, source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| format!("{name}: WGSL parse error: {}", e.emit_to_string(source)))?;
    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::empty());
    validator
        .validate(&module)
        .map_err(|e| format!("{name}: WGSL validation error: {e:?}"))?;
    // A shader with no entry points would silently pass; require both stages.
    let has = |entry: &str, stage: naga::ShaderStage| {
        module
            .entry_points
            .iter()
            .any(|e| e.name == entry && e.stage == stage)
    };
    assert!(
        has("vs_main", naga::ShaderStage::Vertex),
        "{name}: missing vertex entry point vs_main"
    );
    assert!(
        has("fs_main", naga::ShaderStage::Fragment),
        "{name}: missing fragment entry point fs_main"
    );
    Ok(())
}

#[test]
fn line_shader_is_valid_wgsl() {
    validate_wgsl("line.wgsl", include_str!("../shaders/line.wgsl"))
        .expect("line shader must validate");
}

#[test]
fn line_instanced_shader_is_valid_wgsl() {
    validate_wgsl(
        "line_instanced.wgsl",
        include_str!("../shaders/line_instanced.wgsl"),
    )
    .expect("instanced line shader must validate");
}

#[test]
fn mesh_shader_is_valid_wgsl() {
    validate_wgsl("mesh.wgsl", include_str!("../shaders/mesh.wgsl"))
        .expect("mesh shader must validate");
}

#[test]
fn mesh_merged_shader_is_valid_wgsl() {
    validate_wgsl(
        "mesh_merged.wgsl",
        include_str!("../shaders/mesh_merged.wgsl"),
    )
    .expect("merged mesh shader must validate");
}

/// The two accelerated-tier shaders use vertex-stage storage buffers, so they
/// are **not** WebGL2-safe and are deliberately excluded from the GLSL ES 300
/// back-translation test below. They must still parse and validate as WGSL.
#[test]
fn pulled_shaders_are_valid_wgsl() {
    validate_wgsl(
        "line_pulled.wgsl",
        include_str!("../shaders/line_pulled.wgsl"),
    )
    .expect("pulled line shader must validate");
    validate_wgsl(
        "mesh_pulled.wgsl",
        include_str!("../shaders/mesh_pulled.wgsl"),
    )
    .expect("pulled mesh shader must validate");
}

#[test]
fn image_shader_is_valid_wgsl() {
    validate_wgsl("image.wgsl", include_str!("../shaders/image.wgsl"))
        .expect("image shader must validate");
}

/// The id-pass shaders are *assembled*: `shaders/pick_*.wgsl` provide the
/// uniform, the vertex stage and `fs_key`, and a generated fragment tail declares
/// the entry point for the active id-target format. Validating the file alone
/// would therefore only prove the body parses; this validates the exact module
/// text the pipelines compile, for both id-target formats.
#[test]
fn id_pass_shaders_validate_for_both_target_formats() {
    for format in [IdTargetFormat::R32Uint, IdTargetFormat::PackedRgba8] {
        let sources = pick_id_shader_sources(format);
        assert_eq!(
            sources.len(),
            5,
            "every visible draw class needs an id-pass shader"
        );
        for (name, source) in sources {
            validate_wgsl(&format!("{name} ({})", format.as_str()), &source)
                .unwrap_or_else(|e| panic!("{e}"));
        }
    }
}

/// The five color shaders carry a selection-flags block — an integer
/// `texture_2d<u32>` `textureLoad`, a `discard`, and (for the instanced/merged
/// paths) a `flat u32` varying — that is claimed to be WebGL2-safe. WGSL
/// validation alone does not test that claim: the browser tier compiles GLSL ES
/// 3.00. Back-translate each color shader to GLSL ES 3.00 with the same naga
/// wgpu embeds, for both entry points, so the tier claim is tested, not assumed.
#[test]
fn color_shaders_back_translate_to_glsl_es_300() {
    let shaders = [
        ("line.wgsl", include_str!("../shaders/line.wgsl")),
        (
            "line_instanced.wgsl",
            include_str!("../shaders/line_instanced.wgsl"),
        ),
        ("mesh.wgsl", include_str!("../shaders/mesh.wgsl")),
        (
            "mesh_merged.wgsl",
            include_str!("../shaders/mesh_merged.wgsl"),
        ),
        ("image.wgsl", include_str!("../shaders/image.wgsl")),
    ];
    for (name, source) in shaders {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{name}: WGSL parse error: {}", e.emit_to_string(source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: WGSL validation error: {e:?}"));
        for (entry, stage) in [
            ("vs_main", naga::ShaderStage::Vertex),
            ("fs_main", naga::ShaderStage::Fragment),
        ] {
            let options = naga::back::glsl::Options {
                version: naga::back::glsl::Version::new_gles(300),
                ..Default::default()
            };
            let mut glsl = String::new();
            naga::back::glsl::Writer::new(
                &mut glsl,
                &module,
                &info,
                &options,
                &naga::back::glsl::PipelineOptions {
                    shader_stage: stage,
                    entry_point: entry.to_string(),
                    multiview: None,
                },
                naga::proc::BoundsCheckPolicies::default(),
            )
            .unwrap_or_else(|e| panic!("{name} {entry}: GLSL ES 300 writer error: {e:?}"))
            .write()
            .unwrap_or_else(|e| panic!("{name} {entry}: GLSL ES 300 write error: {e:?}"));
            assert!(
                glsl.contains("#version 300 es"),
                "{name} {entry}: expected a GLSL ES 300 header, got:\n{glsl}"
            );
        }
    }
}

#[test]
fn deliberately_broken_shader_fails_validation() {
    // Guards against the validator test being a no-op.
    let broken = r#"
@vertex
fn vs_main() -> @builtin(position) vec4<f32> {
    let x: i32 = 1.0;
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}
@fragment
fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }
"#;
    assert!(validate_wgsl("broken", broken).is_err());
}
