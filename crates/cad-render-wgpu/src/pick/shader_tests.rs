use super::*;
use naga::valid::{Capabilities, ValidationFlags, Validator};

/// Assemble every id-pass shader body with both fragment tails and validate
/// it with the same `naga` wgpu embeds. The bodies alone are not complete
/// modules (the fragment entry point comes from the tail), so this is the
/// only place the real module text is checked.
fn validate(name: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{name}: WGSL parse error: {}", e.emit_to_string(source)));
    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::empty());
    validator
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: WGSL validation error: {e:?}"));
    let has = |entry: &str, stage: naga::ShaderStage| {
        module
            .entry_points
            .iter()
            .any(|e| e.name == entry && e.stage == stage)
    };
    assert!(
        has("vs_main", naga::ShaderStage::Vertex),
        "{name}: no vs_main"
    );
    assert!(
        has("fs_main", naga::ShaderStage::Fragment),
        "{name}: no fs_main"
    );
}

#[test]
fn every_id_shader_assembles_and_validates_for_both_targets() {
    for (name, body) in PICK_SHADER_BODIES {
        for format in [IdTargetFormat::R32Uint, IdTargetFormat::PackedRgba8] {
            let source = pick_shader_source(body, format);
            validate(&format!("{name} ({})", format.as_str()), &source);
        }
    }
}

#[test]
fn the_two_tails_declare_the_matching_fragment_output() {
    assert!(PICK_FS_U32.contains("-> @location(0) u32"));
    assert!(PICK_FS_PACKED.contains("-> @location(0) vec4<f32>"));
    // Both read the shared `fs_key`, which is what the bodies declare.
    assert!(PICK_FS_U32.contains("fs_key(in)"));
    assert!(PICK_FS_PACKED.contains("fs_key(in)"));
}

#[test]
fn packed_encoding_round_trips_every_byte() {
    // The shader writes `byte / 255.0` into a unorm8 channel; the readback
    // multiplies back and rounds. Every 8-bit value must survive.
    for byte in 0u32..=255 {
        let encoded = byte as f32 / 255.0;
        let decoded = (encoded * 255.0).round() as u32 & 0xFF;
        assert_eq!(decoded, byte, "unorm8 round trip for {byte}");
    }
}
