//! Characterization of the connector table itself.
//!
//! The dispatch behaviour is pinned here so a table reshuffle cannot change what
//! a carrier sees: the flat import probe order, the export key -> descriptor map,
//! the `export()` resolution matrix, and the descriptor order. Each entry carries
//! the same `formats-*` gate as its lane, so the full build and a slim build
//! each pin their own answers and both compile.

use super::tests::triangle_db;
use super::*;
use cad_domain::DocumentId;

/// The flat import probe order: one descriptor id per **probe evaluation**, in
/// the order the registry tries them.
fn flat_import_probe_ids(registry: &ConnectorRegistry) -> Vec<&'static str> {
    let mut ids = Vec::new();
    for connector in &registry.connectors {
        if let Some(lane) = &connector.import {
            ids.extend(lane.probes.iter().map(|_| connector.descriptor.id));
        }
    }
    ids
}

/// Every registered export lane as `(registry key, descriptor id)`.
pub(super) fn export_lanes(registry: &ConnectorRegistry) -> Vec<(&'static str, &'static str)> {
    let mut lanes = Vec::new();
    for connector in &registry.connectors {
        lanes.extend(
            connector
                .exports
                .iter()
                .map(|lane| (lane.format_id, connector.descriptor.id)),
        );
    }
    lanes
}

/// What [`ConnectorRegistry::export`] must answer for a known key in this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportExpectation {
    /// A real writer runs and produces bytes.
    Writes,
    /// The format is compiled in but has no writer: a named `not_implemented`.
    ReadOnly,
    /// The format is compiled out of this build: `unsupported-runtime`.
    CompiledOut,
}

fn export_expectation(key: &str) -> ExportExpectation {
    match key {
        "dxf" | "dwg" => ExportExpectation::Writes,
        "stl" | "stl-ascii" | "obj" | "ply" | "off" => {
            if cfg!(feature = "formats-mesh") {
                ExportExpectation::Writes
            } else {
                ExportExpectation::CompiledOut
            }
        }
        "ifc" => {
            if cfg!(feature = "formats-ifc") {
                ExportExpectation::Writes
            } else {
                ExportExpectation::CompiledOut
            }
        }
        "skp" => {
            if cfg!(feature = "formats-skp") {
                ExportExpectation::Writes
            } else {
                ExportExpectation::CompiledOut
            }
        }
        "glb" | "gltf" => {
            if cfg!(feature = "formats-gltf") {
                ExportExpectation::Writes
            } else {
                ExportExpectation::CompiledOut
            }
        }
        "step" => {
            if cfg!(feature = "formats-step") {
                ExportExpectation::ReadOnly
            } else {
                ExportExpectation::CompiledOut
            }
        }
        "dgn" => {
            if cfg!(feature = "formats-dgn") {
                ExportExpectation::ReadOnly
            } else {
                ExportExpectation::CompiledOut
            }
        }
        "rvt" => {
            if cfg!(feature = "formats-rvt") {
                ExportExpectation::ReadOnly
            } else {
                ExportExpectation::CompiledOut
            }
        }
        other => panic!("{other} is a known export key with no pinned expectation"),
    }
}

/// The flat import probe order, pinned as the exact evaluation sequence.
///
/// This is the **evaluation** order, not the descriptor order: DGN appears
/// twice because its V7 ISFF-header probe and its V8 OLE-storage probe are two
/// separate evaluations of one descriptor, and the RVT lane sits between the
/// DGN pair and DWG. Each entry carries the same `formats-*` gate as its lane,
/// so the full build and the slim build each pin their own order and both
/// compile.
// The list head is `cfg`-gated, so `vec![..]` cannot express it.
#[allow(clippy::vec_init_then_push)]
#[test]
fn import_probe_order_is_pinned() {
    let registry = ConnectorRegistry::builtin();
    let mut expected: Vec<&'static str> = Vec::new();
    #[cfg(feature = "formats-skp")]
    expected.push("skp");
    #[cfg(feature = "formats-dgn")]
    {
        // V7 ISFF header, then V8 OLE root storage (`Dgn~H`).
        expected.push("dgn");
        expected.push("dgn");
    }
    #[cfg(feature = "formats-rvt")]
    expected.push("rvt");
    expected.push("dwg");
    expected.push("dxf");
    #[cfg(feature = "formats-ifc")]
    expected.push("ifc");
    #[cfg(feature = "formats-step")]
    expected.push("step");
    #[cfg(feature = "formats-gltf")]
    expected.push("gltf");
    #[cfg(feature = "formats-mesh")]
    {
        expected.push("stl");
        expected.push("ply");
        expected.push("off");
        expected.push("obj");
    }
    assert_eq!(flat_import_probe_ids(&registry), expected);
}

/// The export key -> descriptor map, pinned for every key this build compiles.
///
/// Two descriptors own two keys each (`stl`/`stl-ascii`, `glb`/`gltf`), so the
/// map is deliberately not 1:1; a carrier passes the key through to the writer,
/// which stamps its own container into the report.
#[test]
fn export_keys_map_to_their_descriptors() {
    let registry = ConnectorRegistry::builtin();
    let mut expected: Vec<(&'static str, &'static str)> = Vec::new();
    expected.push(("dxf", "dxf"));
    expected.push(("dwg", "dwg"));
    #[cfg(feature = "formats-mesh")]
    {
        expected.push(("stl", "stl"));
        expected.push(("stl-ascii", "stl"));
        expected.push(("obj", "obj"));
        expected.push(("ply", "ply"));
        expected.push(("off", "off"));
    }
    #[cfg(feature = "formats-ifc")]
    expected.push(("ifc", "ifc"));
    #[cfg(feature = "formats-skp")]
    expected.push(("skp", "skp"));
    #[cfg(feature = "formats-gltf")]
    {
        expected.push(("glb", "gltf"));
        expected.push(("gltf", "gltf"));
    }
    // The **map** is the behaviour; the lane order is a property of where the
    // key is declared (per connector), so it is compared as a set of pairs. A
    // duplicate key would make the order observable, and there is none.
    let mut actual = export_lanes(&registry);
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
    assert!(
        expected.windows(2).all(|pair| pair[0].0 != pair[1].0),
        "each export key belongs to exactly one descriptor"
    );

    // Every lane key resolves back to the descriptor it writes through.
    for (key, descriptor_id) in &expected {
        assert_eq!(
            registry.descriptor(key).map(|descriptor| descriptor.id),
            Some(*descriptor_id),
            "{key} must resolve to its own descriptor"
        );
    }

    // STEP/DGN/RVT are known keys with no writer: compiled in, they resolve by
    // id and are refusals (never lanes); compiled out, they are runtime ones.
    for id in ["step", "dgn", "rvt"] {
        assert!(
            !expected.iter().any(|(key, _)| *key == id),
            "{id} must not be an export lane"
        );
        if export_expectation(id) == ExportExpectation::CompiledOut {
            assert_eq!(
                registry.descriptor(id),
                None,
                "{id} is compiled out, so it cannot name a descriptor"
            );
        } else {
            let descriptor = registry
                .descriptor(id)
                .unwrap_or_else(|| panic!("{id} is compiled in"));
            assert!(
                descriptor.export.is_none(),
                "{id} must not declare an export capability this build cannot honour"
            );
        }
    }
}

/// The `export()` resolution matrix, pinned entry by entry.
///
/// Real lane / named `not_implemented` refusal for a compiled read-only format /
/// `unsupported-runtime: export.<key>` for a compiled-out format / the
/// `export.unknown_format` answer for junk. The middle two are the slim-build
/// asymmetry: a compiled-out format must never be reported as if the product
/// had no idea what it is.
#[test]
fn export_resolution_matrix_is_pinned() {
    let registry = ConnectorRegistry::builtin();
    let database = triangle_db();
    let units = cad_domain::UnitContext::from_meter();
    let job = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: Some(&units),
        space: cad_connector_core::ExportSpace::Model,
        limits: cad_connector_core::ExportLimits::default(),
    };
    let settings = ExportSettings::default();
    for key in KNOWN_EXPORT_KEYS {
        let result = registry.export(key, &job, &settings);
        match export_expectation(key) {
            ExportExpectation::Writes => {
                let output =
                    result.unwrap_or_else(|error| panic!("{key} lane must run: {error:?}"));
                assert!(!output.bytes.is_empty(), "{key} wrote no bytes");
            }
            ExportExpectation::ReadOnly => match result {
                Err(CadError::Unsupported(message)) => {
                    assert_eq!(
                        message,
                        format!("export.{key}.not_implemented: writer is not wired in this build"),
                        "{key} refusal"
                    );
                }
                other => panic!("{key}: expected a named refusal, got {other:?}"),
            },
            ExportExpectation::CompiledOut => match result {
                Err(CadError::Unsupported(message)) => {
                    assert_eq!(message, format!("unsupported-runtime: export.{key}"));
                }
                other => panic!("{key}: expected a runtime refusal, got {other:?}"),
            },
        }
    }
    match registry.export("not-a-format", &job, &settings) {
        Err(CadError::Unsupported(message)) => {
            assert_eq!(message, "export.unknown_format: not-a-format")
        }
        other => panic!("expected an unknown-format refusal, got {other:?}"),
    }
}

/// `descriptors()` lists every connector once, in connector order.
// The list head is `cfg`-gated, so `vec![..]` cannot express it.
#[allow(clippy::vec_init_then_push)]
#[test]
fn descriptor_order_is_pinned() {
    let registry = ConnectorRegistry::builtin();
    let mut expected: Vec<&'static str> = Vec::new();
    #[cfg(feature = "formats-skp")]
    expected.push("skp");
    #[cfg(feature = "formats-dgn")]
    expected.push("dgn");
    #[cfg(feature = "formats-rvt")]
    expected.push("rvt");
    expected.push("dwg");
    expected.push("dxf");
    #[cfg(feature = "formats-ifc")]
    expected.push("ifc");
    #[cfg(feature = "formats-step")]
    expected.push("step");
    #[cfg(feature = "formats-gltf")]
    expected.push("gltf");
    #[cfg(feature = "formats-mesh")]
    {
        expected.push("stl");
        expected.push("ply");
        expected.push("off");
        expected.push("obj");
    }
    let actual: Vec<&'static str> = registry
        .descriptors()
        .map(|descriptor| descriptor.id)
        .collect();
    assert_eq!(actual, expected);
}

/// The table and its descriptors must agree in both directions.
///
/// A lane without a declared direction would advertise a capability the
/// descriptor denies; a declared direction without a lane would promise one the
/// registry cannot dispatch to — both are silent capability lies.
#[test]
fn the_table_and_its_descriptors_agree_on_both_directions() {
    let registry = ConnectorRegistry::builtin();
    for connector in &registry.connectors {
        let id = connector.descriptor.id;
        assert_eq!(
            connector.import.is_some(),
            connector.descriptor.import.is_some(),
            "{id}: an import lane must exist exactly when the descriptor declares the direction"
        );
        if let Some(lane) = &connector.import {
            assert!(
                !lane.probes.is_empty(),
                "{id}: an import lane without a probe can never claim bytes"
            );
        }
        assert_eq!(
            !connector.exports.is_empty(),
            connector.descriptor.export.is_some(),
            "{id}: an export lane must exist exactly when the descriptor declares the direction"
        );
        for lane in connector.exports {
            assert!(
                KNOWN_EXPORT_KEYS.contains(&lane.format_id),
                "{id}: {} is an export key the product does not know",
                lane.format_id
            );
        }
    }
}

/// A full build must register every format the product knows.
///
/// A connector silently dropped from `formats-all` is exactly what this
/// catches: the registry would answer `unsupported-runtime` on a build that is
/// supposed to carry the format.
#[cfg(feature = "formats-all")]
#[test]
fn a_full_build_registers_every_known_format() {
    let registry = ConnectorRegistry::builtin();
    let ids: Vec<&'static str> = registry
        .descriptors()
        .map(|descriptor| descriptor.id)
        .collect();
    for id in [
        "skp", "dgn", "rvt", "dwg", "dxf", "ifc", "step", "gltf", "stl", "ply", "off", "obj",
    ] {
        assert!(
            ids.contains(&id),
            "{id} is missing from a formats-all build: {ids:?}"
        );
    }
    assert_eq!(ids.len(), 12, "one connector per format: {ids:?}");
}
