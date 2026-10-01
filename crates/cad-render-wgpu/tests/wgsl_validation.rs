//! Offline WGSL validation (spec §5.2, audit F14).
//!
//! Both shader files are parsed and validated with the same `naga` version that
//! `wgpu 30.0.1` embeds, so a syntax or type error in a shader fails this test
//! without needing a GPU. This is static validation only: it does not compile
//! the pipelines or execute a draw.

use naga::valid::{Capabilities, ValidationFlags, Validator};

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
fn mesh_shader_is_valid_wgsl() {
    validate_wgsl("mesh.wgsl", include_str!("../shaders/mesh.wgsl"))
        .expect("mesh shader must validate");
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
