//! Unit tests.

use super::*;

#[test]
fn cli_operation_names_parse() {
    for name in [
        "scan",
        "proxy-report",
        "measure",
        "import-notes",
        "export-notes",
        "build-representation",
        "render",
        "benchmark",
    ] {
        assert!(CliOperation::parse(name).is_some(), "{name}");
    }
    assert!(CliOperation::parse("nope").is_none());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn render_of_missing_input_fails_before_any_gpu_work() {
    // Import happens before device creation, so a missing file is an
    // `invalid_input` failure on every machine — adapter or not.
    let invocation = CliInvocation::new(CliOperation::FixedViewportRender, "missing.dwg");
    let error = run(&invocation).unwrap_err();
    assert_eq!(error.code, error_code::INVALID_INPUT);
    assert_eq!(error.exit_code(), 1);
}

#[cfg(target_arch = "wasm32")]
#[test]
fn render_on_wasm_is_explicitly_unsupported() {
    // wasm has no headless device: the operation stays explicitly not run.
    let invocation = CliInvocation::new(CliOperation::FixedViewportRender, "missing.dwg");
    let error = run(&invocation).unwrap_err();
    assert_eq!(error.code, error_code::UNSUPPORTED);
    assert_eq!(error.exit_code(), 1);
}

#[test]
fn scan_of_missing_file_is_an_input_error() {
    let invocation = CliInvocation::new(CliOperation::Scan, "definitely-missing.dwg");
    let error = run(&invocation).unwrap_err();
    assert_eq!(error.code, error_code::INVALID_INPUT);
}

#[test]
fn locale_normalizes_and_falls_back() {
    assert_eq!(Locale::parse("en"), Locale::En);
    assert_eq!(Locale::parse("en-US"), Locale::En);
    assert_eq!(Locale::parse("zh-CN"), Locale::ZhCn);
    assert_eq!(Locale::parse("fr"), Locale::ZhCn);
    assert_eq!(Locale::default(), Locale::ZhCn);
}

#[test]
fn error_document_has_stable_schema() {
    let error = CliError::usage("bad option").with_context(serde_json::json!({"flag": "--x"}));
    let value = error.to_json(CliOperation::Measure);
    assert_eq!(value["schema_version"], CLI_SCHEMA_VERSION);
    assert_eq!(value["operation"], "measure");
    assert_eq!(value["error"]["code"], error_code::USAGE);
    assert_eq!(value["error"]["message"], "bad option");
    assert_eq!(value["error"]["context"]["flag"], "--x");
    assert_eq!(error.exit_code(), 2);
}

#[test]
fn atomic_write_leaves_no_temp_on_failure() {
    let dir = std::env::temp_dir().join(format!("yacr-atomic-{}", unique_counter()));
    std::fs::create_dir_all(&dir).unwrap();
    // A directory cannot be replaced by rename, so this must fail cleanly.
    let blocked = dir.join("target-dir");
    std::fs::create_dir_all(&blocked).unwrap();
    assert!(write_atomic(&blocked, b"nope").is_err());
    // No stray temp files remain.
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
    std::fs::remove_dir_all(&dir).ok();
}
