//! Committed-DXF fixture contract (no GPU required).
//!
//! The QCAD `flange` sample in `fixtures/dxf/qcad-flange/` is the repository's
//! one openly-distributed render regression input (provenance in its
//! `SOURCE.md`). These tests pin that the CLI can still import it and build a
//! non-empty representation. They do **not** render (that needs a GPU adapter
//! and is covered by `scripts/check-dxf-reference.py`) and they do not claim
//! vendor compatibility.
//!
//! Path resolution uses `CARGO_MANIFEST_DIR`, so the tests run from the crate
//! directory in CI the same way they do locally.

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_cad-cli-tools");

fn fixture() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/dxf/qcad-flange/flange.dxf")
        .canonicalize()
        .expect("committed QCAD flange DXF fixture exists");
    assert!(path.is_file(), "fixture is not a file: {}", path.display());
    path
}

fn run_json(operation: &str) -> serde_json::Value {
    let output = Command::new(BIN)
        .arg(operation)
        .arg(fixture())
        .output()
        .expect("spawn cad-cli-tools");
    assert!(
        output.status.success(),
        "{operation} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{operation} success must not write to stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is pure JSON")
}

#[test]
fn committed_qcad_flange_scan_reports_millimetres_and_partial() {
    let value = run_json("scan");
    assert_eq!(value["operation"], "scan");
    assert_eq!(value["schema_version"], 1);
    assert!(
        value["entities"].as_u64().unwrap_or(0) > 0,
        "no entities imported: {value}"
    );
    assert!(
        value["model_entities"].as_u64().unwrap_or(0) > 0,
        "no model-space entities: {value}"
    );
    assert_eq!(value["units"], "Millimeter");
    // The sample carries DIMENSION/MTEXT the importer cannot draw yet; the
    // completeness report must say so rather than silently claiming success.
    assert_eq!(value["completeness"]["status"], "partial");
    let items = value["completeness"]["items"]
        .as_array()
        .expect("completeness.items is an array");
    assert!(!items.is_empty(), "Partial without a reason: {value}");
    assert!(
        items.iter().any(|item| item
            .as_str()
            .map(|text| text.contains("AcDbDimension"))
            .unwrap_or(false)),
        "expected the known dimension limitation: {value}"
    );
}

/// The flange fixture has an **empty** first paper layout (`*Paper_Space`) and a
/// **populated** second one (`*Paper_Space1`). `plot` must default to the
/// populated sheet, render it, and report the paper in millimetres (the locked
/// acadrust reader leaves `group 72` unapplied; the name carries the unit).
#[test]
fn flange_plot_defaults_to_the_populated_paper_layout() {
    let icd = std::env::var_os("VK_ICD_FILENAMES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/share/vulkan/icd.d/lvp_icd.json"));
    if !icd.exists() {
        eprintln!(
            "SKIP flange_plot_defaults_to_the_populated_paper_layout: no Vulkan ICD at {}",
            icd.display()
        );
        return;
    }
    let output = Command::new(BIN)
        .env("VK_ICD_FILENAMES", &icd)
        .args(["plot"])
        .arg(fixture())
        .args(["--width", "400", "--height", "300"])
        .output()
        .expect("spawn cad-cli-tools");
    assert!(
        output.status.success(),
        "plot failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("plot JSON");
    assert_eq!(value["layout"]["name"], "*Paper_Space1", "{value}");
    assert_eq!(value["paper"]["units"], "millimeters", "{value}");
    // The locked reader drops LAYOUT `group 73`; the sheet is inferred as
    // landscape from the viewport windows (A4 portrait declared), so the paper
    // axes are swapped and the content stays upright.
    assert_eq!(value["paper"]["width"], 297.0, "{value}");
    assert_eq!(value["paper"]["height"], 210.0, "{value}");
    assert_eq!(value["paper"]["rotation_degrees"], 0.0, "{value}");
    assert!(
        value["pixels"]["non_background"].as_u64().unwrap_or(0) > 0,
        "blank plot: {value}"
    );
}

#[test]
fn committed_qcad_flange_builds_a_non_empty_representation() {
    let value = run_json("build-representation");
    assert_eq!(value["operation"], "build-representation");
    assert!(
        value["failures"]
            .as_array()
            .expect("failures is an array")
            .is_empty(),
        "representation failures: {value}"
    );
    assert!(
        value["primitives"].as_u64().unwrap_or(0) > 0,
        "empty representation: {value}"
    );
    assert!(
        value["kind_counts"]["lines"].as_u64().unwrap_or(0) > 0,
        "expected line geometry: {value}"
    );
    assert!(
        value["vertices"].as_u64().unwrap_or(0) > 0,
        "expected vertices: {value}"
    );
    // The six dimensions have no persisted anonymous block, so they must be
    // synthesized: solid arrowheads (meshes) and measurement text. If this
    // regresses, dimensions silently disappear from the render again.
    assert!(
        value["kind_counts"]["meshes"].as_u64().unwrap_or(0) >= 8,
        "expected synthesized dimension arrowheads: {value}"
    );
    assert!(
        value["kind_counts"]["texts"].as_u64().unwrap_or(0) >= 6,
        "expected synthesized dimension measurement text: {value}"
    );
}
