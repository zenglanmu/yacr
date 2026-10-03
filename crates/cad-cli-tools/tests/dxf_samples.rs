//! Committed QCAD example DXF corpus: parse/representation contract (no GPU).
//!
//! Every file in `fixtures/dxf/qcad-examples/` must import and build a non-empty
//! representation without build failures. This is a regression guard for the
//! entity conversions the corpus exercises (LINE, LWPOLYLINE, MTEXT, INSERT,
//! SPLINE, DIMENSION, ARC, CIRCLE, POINT, ELLIPSE, HATCH, LEADER, SOLID).
//! It does not render (that needs a GPU adapter) and does not claim vendor
//! compatibility. Provenance: `fixtures/dxf/qcad-examples/SOURCE.md`.

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_cad-cli-tools");

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dxf/qcad-examples")
}

fn run_json(operation: &str, sample: &std::path::Path) -> serde_json::Value {
    let output = Command::new(BIN)
        .arg(operation)
        .arg(sample)
        .output()
        .expect("spawn cad-cli-tools");
    assert!(
        output.status.success(),
        "{operation} {} failed: {}",
        sample.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is pure JSON")
}

fn corpus_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus_dir())
        .expect("committed corpus directory exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "dxf"))
        .collect();
    files.sort();
    files
}

#[test]
fn committed_qcad_examples_import_and_build_without_failures() {
    let files = corpus_files();
    assert!(!files.is_empty(), "corpus is empty");
    for file in files {
        let scan = run_json("scan", &file);
        assert!(
            scan["entities"].as_u64().unwrap_or(0) > 0,
            "{}: no entities",
            file.display()
        );
        assert!(
            scan["model_entities"].as_u64().unwrap_or(0) > 0,
            "{}: no model-space entities",
            file.display()
        );
        let build = run_json("build-representation", &file);
        let failures = build["failures"].as_array().expect("failures is an array");
        assert!(failures.is_empty(), "{}: {failures:?}", file.display());
        assert!(
            build["primitives"].as_u64().unwrap_or(0) > 0,
            "{}: empty representation",
            file.display()
        );
    }
}

/// Every entity type exercised by the committed corpus (plus the flange sample)
/// must have a display representation: no `render: unsupported`. This is the
/// coverage gate the user asked for; `docs/dxf-entity-coverage.md` records the
/// kinds that still have no representation and are therefore absent from the
/// corpus.
#[test]
fn committed_qcad_examples_have_no_unsupported_entity_types() {
    let mut files = corpus_files();
    files.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dxf/qcad-flange/flange.dxf"),
    );
    for file in files {
        let report = run_json("proxy-report", &file);
        let unsupported: Vec<String> = report["entity_types"]
            .as_array()
            .expect("entity_types is an array")
            .iter()
            .filter(|entry| entry["render"] == "unsupported")
            .map(|entry| entry["type"].as_str().unwrap_or("?").to_string())
            .collect();
        assert!(
            unsupported.is_empty(),
            "{}: unsupported entity types {unsupported:?}",
            file.display()
        );
    }
}

#[test]
fn committed_qcad_leader_entities_have_a_display_representation() {
    // `entities.dxf` and `example01.dxf` contain LEADER entities. They must no
    // longer be reported as missing a display representation.
    for name in ["entities.dxf", "example01.dxf"] {
        let file = corpus_dir().join(name);
        let scan = run_json("scan", &file);
        let items = scan["completeness"]["items"].to_string();
        assert!(
            !items.contains("Leader"),
            "{name}: LEADER still unsupported: {items}"
        );
        let build = run_json("build-representation", &file);
        assert!(
            build["kind_counts"]["meshes"].as_u64().unwrap_or(0) > 0,
            "{name}: expected leader/dimension arrowheads: {build}"
        );
    }
}
