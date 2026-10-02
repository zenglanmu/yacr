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
fn benchmark_json_has_reproducible_schema_and_real_measurements() {
    // The demo controller gives a real document (no DWG fixture exists), so the
    // build/cpu/gpu figures are measured on it while parse/upload stay `null`.
    let controller = HostController::with_demo_document([1920.0, 1080.0]).unwrap();
    let dir = std::env::temp_dir().join(format!("yacr-bench-{}", unique_counter()));
    std::fs::create_dir_all(&dir).unwrap();
    let sample = dir.join("sample.dwg");
    std::fs::write(&sample, vec![0u8; 4096]).unwrap();
    let mut invocation = CliInvocation::new(CliOperation::Benchmark, &sample);
    invocation.locale = Locale::En;

    let value = run_benchmark(&controller, &invocation, None).unwrap();

    // Envelope keys.
    assert_eq!(value["schema_version"], CLI_SCHEMA_VERSION);
    assert_eq!(value["operation"], "benchmark");
    // The claim is fixed: a measurement, never a compatibility statement.
    assert_eq!(value["claim"], cad_diagnostics::BenchmarkReport::CLAIM);
    assert_eq!(value["claim"], "measurement, not compatibility");
    // Release/debug flag is present and boolean.
    assert!(value["environment"]["release_build"].is_boolean());
    // The measured context carries the real session viewport and the release
    // flag; a CLI has no browser/device on this path, so they are null.
    assert_eq!(
        value["context"]["viewport"],
        controller.viewport_id.0.to_string()
    );
    assert!(value["context"]["browser"].is_null());
    assert!(value["context"]["device"].is_null());
    assert_eq!(
        value["context"]["release_build"],
        value["environment"]["release_build"]
    );
    // Real on-disk size was measured (4096 bytes written above).
    assert_eq!(value["memory_bytes"]["file"], 4096);
    // Build, CPU geometry and GPU estimate were measured on the demo document.
    assert!(value["timings_ms"]["build"].is_number());
    assert!(value["memory_bytes"]["cpu_geometry"].is_number());
    assert!(value["memory_bytes"]["gpu_estimated"].is_number());
    assert!(value["scene"]["batches"].as_u64().unwrap() > 0);
    // Unmeasured phases are explicit `null`, never a fabricated zero.
    assert!(value["timings_ms"]["parse"].is_null());
    assert!(value["timings_ms"]["upload"].is_null());
    assert!(value["timings_ms"]["first_usable"].is_null());
    assert!(value["memory_bytes"]["domain"].is_null());
    assert!(value["sample_hash"].is_null());
    // Budget ceilings are reported so a reader can judge the charge.
    assert!(value["budgets"]["cpu_bytes"].is_number());
    assert!(value["budgets"]["upload_bytes_per_frame"].is_number());
    assert_eq!(value["budgets"]["queued_tasks"], 8);
    assert!(value["over_budget"].is_array());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn benchmark_json_is_deterministic_for_an_unchanged_document() {
    let controller = HostController::with_demo_document([1920.0, 1080.0]).unwrap();
    let invocation = CliInvocation::new(CliOperation::Benchmark, "missing.dwg");
    let a = run_benchmark(&controller, &invocation, None).unwrap();
    let b = run_benchmark(&controller, &invocation, None).unwrap();
    // Everything except the measured times is byte-identical; counts are exact.
    for key in ["scene", "memory_bytes", "budgets", "sample_hash", "claim"] {
        assert_eq!(a[key], b[key], "key {key} drifted");
    }
    assert_eq!(a["memory_bytes"]["file"], b["memory_bytes"]["file"]);
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
