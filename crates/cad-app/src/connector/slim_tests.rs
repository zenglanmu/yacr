//! Contract tests for the compiled-out lane answers.
//!
//! A slim build (`--no-default-features`, the `dom-canvas` wasm bundle) compiles
//! a probe out but still has to answer honestly for the bytes that format owns.
//! These tests pin the reason strings, the export-key table and the one build
//! where the historical unsupported-input message is unchanged.

use super::*;

#[test]
fn absent_import_reasons_are_pinned_per_format() {
    // No compiled lane: every real format is a named refusal.
    let none: &[&str] = &[];
    let cases: &[(DrawingFormat, Option<&str>)] = &[
        (DrawingFormat::Dwg, Some("dwg")),
        (DrawingFormat::Dxf, Some("dxf")),
        (DrawingFormat::Step, Some("step")),
        (DrawingFormat::Ifc, Some("ifc")),
        (DrawingFormat::Obj, Some("obj")),
        (DrawingFormat::Stl, Some("stl")),
        (DrawingFormat::Ply, Some("ply")),
        (DrawingFormat::Off, Some("off")),
        (DrawingFormat::Gltf, Some("gltf")),
        (DrawingFormat::Dgn, Some("dgn")),
        (DrawingFormat::Ole, Some("ole")),
        (DrawingFormat::Skp, Some("skp")),
        (DrawingFormat::Unknown, None),
    ];
    for (format, id) in cases {
        let reason = absent_import_reason(*format, none);
        match id {
            Some(id) => assert_eq!(
                reason,
                Some(format!("unsupported-runtime: import.{id}")),
                "{format:?} must be a named refusal"
            ),
            None => assert_eq!(reason, None, "{format:?} is not a format"),
        }
    }

    // A compiled lane resolves to `None`; an absent one stays named.
    let compiled: &[&str] = &["dwg", "dxf"];
    assert_eq!(absent_import_reason(DrawingFormat::Dwg, compiled), None);
    assert_eq!(absent_import_reason(DrawingFormat::Dxf, compiled), None);
    assert_eq!(
        absent_import_reason(DrawingFormat::Step, compiled),
        Some("unsupported-runtime: import.step".into())
    );
}

#[test]
fn absent_export_reasons_cover_the_known_keys_only() {
    let compiled: &[&str] = &["dxf", "dwg"];
    for key in KNOWN_EXPORT_KEYS {
        let reason = absent_export_reason(key, compiled);
        if matches!(*key, "dxf" | "dwg") {
            assert_eq!(reason, None, "{key} is compiled");
        } else {
            assert_eq!(
                reason,
                Some(format!("unsupported-runtime: export.{key}")),
                "{key} must refuse by its own name"
            );
        }
    }
    // A product-unknown key stays the registry's `export.unknown_format`.
    assert_eq!(absent_export_reason("zip", compiled), None);
    assert_eq!(absent_export_reason("zip", &[]), None);
}

#[test]
fn known_export_keys_cover_every_lane_and_refusal() {
    let registry = ConnectorRegistry::builtin();
    for connector in &registry.exports {
        assert!(
            KNOWN_EXPORT_KEYS.contains(&connector.format_id),
            "{} is a lane but is not a known key",
            connector.format_id
        );
    }
    for (key, _) in &registry.refusals {
        assert!(
            KNOWN_EXPORT_KEYS.contains(key),
            "{key} is a refusal but is not a known key"
        );
    }

    let mut expected = vec![
        "dxf",
        "dwg",
        "stl",
        "stl-ascii",
        "obj",
        "ply",
        "off",
        "ifc",
        "skp",
        "glb",
        "gltf",
        "step",
        "dgn",
        "rvt",
    ];
    let mut actual = KNOWN_EXPORT_KEYS.to_vec();
    expected.sort_unstable();
    actual.sort_unstable();
    assert_eq!(actual, expected);
}

#[cfg(feature = "formats-all")]
#[test]
fn the_full_build_keeps_the_historical_unsupported_input() {
    const { assert!(ALL_FORMATS_COMPILED) };
    assert_eq!(unclaimed_input(), crate::import::UNSUPPORTED_INPUT);

    let registry = ConnectorRegistry::builtin();
    let compiled: Vec<&str> = registry
        .imports
        .iter()
        .map(|connector| connector.descriptor.id)
        .collect();
    for descriptor in registry.descriptors() {
        assert!(
            compiled.contains(&descriptor.id),
            "{} is listed but has no lane",
            descriptor.id
        );
    }
    for format in [
        DrawingFormat::Dwg,
        DrawingFormat::Dxf,
        DrawingFormat::Step,
        DrawingFormat::Ifc,
        DrawingFormat::Obj,
        DrawingFormat::Stl,
        DrawingFormat::Ply,
        DrawingFormat::Off,
        DrawingFormat::Gltf,
        DrawingFormat::Dgn,
        DrawingFormat::Skp,
    ] {
        assert_eq!(
            absent_import_reason(format, &compiled),
            None,
            "{format:?} must be compiled in a full build"
        );
    }
}

#[test]
fn dwg_opened_source_keeps_the_reverse_engineered_notice() {
    let source = ConnectorRegistry::builtin()
        .opened_source(Some("dwg"))
        .expect("DWG is a reverse-engineered source");
    assert_eq!(source.connector, "dwg");
    assert_eq!(source.spec, SpecVisibility::ReverseEngineered);
    assert!(source.can_write, "the DWG writer is wired in every build");
    assert!(source.experimental, "the DWG writer is not vendor-verified");
}
