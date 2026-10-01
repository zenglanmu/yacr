//! Integration contracts for the headless CLI binary.
//!
//! These tests invoke the *built* binary (via `CARGO_BIN_EXE_...`) so they
//! exercise the real argument parsing, stdout/stderr split, exit codes and the
//! atomic `--out` write — not just the library API.
//!
//! `assert_cmd` is deliberately not used: it is not a dependency of this
//! workspace, and the audit requires no new third-party crates for this.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The compiled CLI binary under test.
const BIN: &str = env!("CARGO_BIN_EXE_cad-cli-tools");

/// A unique scratch directory per test.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yacr-cli-{}-{}-{}",
        tag,
        std::process::id(),
        unique()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Write a minimal header-only `AC1015` DWG.
///
/// The importer is failsafe: a valid version signature with no recoverable
/// content yields an empty document, which is exactly what the success-path
/// contracts need without shipping authorized CAD fixtures.
fn write_minimal_dwg(dir: &Path) -> PathBuf {
    let mut bytes = vec![0u8; 2048];
    bytes[0..6].copy_from_slice(b"AC1015");
    let path = dir.join("minimal.dwg");
    std::fs::write(&path, bytes).expect("write dwg");
    path
}

fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(BIN).args(args).output().expect("spawn cli")
}

fn s(value: &str) -> &std::ffi::OsStr {
    std::ffi::OsStr::new(value)
}

fn stdout_json(output: &Output) -> serde_json::Value {
    let text = String::from_utf8(output.stdout.clone()).expect("stdout is utf-8");
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("stdout is not pure JSON ({e}): {text:?}"))
}

fn stderr_json(output: &Output) -> serde_json::Value {
    let text = String::from_utf8(output.stderr.clone()).expect("stderr is utf-8");
    // The human line comes first; the structured document is the first JSON
    // value in the stream. Trailing human text (e.g. usage) is ignored.
    let start = text
        .find('{')
        .unwrap_or_else(|| panic!("no structured error document in stderr: {text:?}"));
    let mut stream =
        serde_json::Deserializer::from_str(&text[start..]).into_iter::<serde_json::Value>();
    stream
        .next()
        .unwrap_or_else(|| panic!("no JSON value in stderr: {text:?}"))
        .unwrap_or_else(|e| panic!("stderr error document is not JSON ({e}): {text:?}"))
}

// ---------------------------------------------------------------------------
// stdout is pure JSON; success exits zero
// ---------------------------------------------------------------------------

#[test]
fn scan_success_is_pure_json_on_stdout() {
    let dir = scratch("scan");
    let dwg = write_minimal_dwg(&dir);
    let output = run(&[s("scan"), dwg.as_os_str()]);
    assert!(output.status.success(), "exit: {:?}", output.status);
    assert!(
        output.stderr.is_empty(),
        "success must not write to stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(&output);
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["operation"], "scan");
    assert!(value.get("entities").is_some());
    clean(&dir);
}

#[test]
fn measure_success_reports_structured_numbers() {
    let dir = scratch("measure");
    let dwg = write_minimal_dwg(&dir);
    let output = run(&[s("measure"), dwg.as_os_str(), s("--points"), s("0,0;3,4")]);
    assert!(output.status.success());
    let value = stdout_json(&output);
    assert_eq!(value["operation"], "measure");
    assert_eq!(value["measurement"]["value"], 5.0);
    assert_eq!(value["measurement"]["units"], "DrawingUnits");
    clean(&dir);
}

// ---------------------------------------------------------------------------
// Error path: structured stderr + non-zero exit
// ---------------------------------------------------------------------------

#[test]
fn missing_input_exits_non_zero_with_structured_error() {
    let output = run(&[s("scan"), s("/definitely/not/here.dwg")]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    // stdout must stay empty on failure — no partial JSON result.
    assert!(
        output.stdout.is_empty(),
        "stdout must be empty on error: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value = stderr_json(&output);
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["operation"], "scan");
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(value["error"].get("message").is_some());
    assert!(value["error"].get("context").is_some());
}

#[test]
fn render_is_explicitly_unsupported_and_exits_non_zero() {
    let dir = scratch("render");
    let dwg = write_minimal_dwg(&dir);
    let output = run(&[s("render"), dwg.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let value = stderr_json(&output);
    assert_eq!(value["operation"], "render");
    assert_eq!(value["error"]["code"], "unsupported");
    clean(&dir);
}

#[test]
fn unknown_operation_exits_two_with_usage_code() {
    let output = run(&[s("bogus-operation"), s("x.dwg")]);
    assert_eq!(output.status.code(), Some(2));
    let value = stderr_json(&output);
    assert_eq!(value["error"]["code"], "usage");
}

#[test]
fn missing_required_points_is_a_non_zero_failure_not_empty_success() {
    let dir = scratch("short-points");
    let dwg = write_minimal_dwg(&dir);
    let output = run(&[s("measure"), dwg.as_os_str(), s("--points"), s("0,0")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let value = stderr_json(&output);
    assert_eq!(value["error"]["code"], "invalid_input");
    clean(&dir);
}

// ---------------------------------------------------------------------------
// Atomic --out
// ---------------------------------------------------------------------------

#[test]
fn out_writes_file_and_keeps_stdout_empty() {
    let dir = scratch("out");
    let dwg = write_minimal_dwg(&dir);
    let out = dir.join("result.json");
    let output = run(&[s("scan"), dwg.as_os_str(), s("--out"), out.as_os_str()]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty(), "stdout must be empty with --out");
    let text = std::fs::read_to_string(&out).expect("out file exists");
    let value: serde_json::Value = serde_json::from_str(&text).expect("out is json");
    assert_eq!(value["operation"], "scan");
    clean(&dir);
}

#[test]
fn failed_run_leaves_no_partial_out_file() {
    // A structurally valid operation whose write target cannot be replaced:
    // `--out` points at an existing directory, so the atomic rename must fail,
    // the temp file must be removed, and no output file may appear.
    let dir = scratch("out-fail");
    let dwg = write_minimal_dwg(&dir);
    let target = dir.join("occupied-dir");
    std::fs::create_dir_all(&target).unwrap();

    let output = run(&[s("scan"), dwg.as_os_str(), s("--out"), target.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let value = stderr_json(&output);
    assert_eq!(value["operation"], "scan");
    assert_eq!(value["error"]["code"], "output_write_failed");

    // No `.tmp` leftovers anywhere in the directory.
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
    clean(&dir);
}

#[test]
fn failed_operation_never_creates_the_out_file() {
    let dir = scratch("out-only-on-success");
    let out = dir.join("never.json");
    let output = run(&[s("scan"), s("/missing.dwg"), s("--out"), out.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!out.exists(), "a failed run must not create --out");
    clean(&dir);
}

// ---------------------------------------------------------------------------
// Locale independence of machine output
// ---------------------------------------------------------------------------

#[test]
fn locale_does_not_change_machine_keys() {
    let dir = scratch("locale");
    let dwg = write_minimal_dwg(&dir);

    let zh = run(&[s("scan"), dwg.as_os_str(), s("--locale"), s("zh-CN")]);
    let en = run(&[s("scan"), dwg.as_os_str(), s("--locale"), s("en")]);
    assert!(zh.status.success() && en.status.success());

    let zh_json = stdout_json(&zh);
    let en_json = stdout_json(&en);
    assert_eq!(zh_json, en_json, "--locale must not change machine output");

    // Both error documents must agree on every machine key; only the leading
    // human line differs.
    let zh_err = run(&[s("render"), dwg.as_os_str(), s("--locale"), s("zh-CN")]);
    let en_err = run(&[s("render"), dwg.as_os_str(), s("--locale"), s("en")]);
    let zh_doc = stderr_json(&zh_err);
    let en_doc = stderr_json(&en_err);
    assert_eq!(
        zh_doc, en_doc,
        "--locale must not change error machine keys"
    );
    clean(&dir);
}

fn clean(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}
