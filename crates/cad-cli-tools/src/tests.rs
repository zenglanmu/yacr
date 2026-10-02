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
        "plot",
        "benchmark",
    ] {
        assert!(CliOperation::parse(name).is_some(), "{name}");
    }
    assert!(CliOperation::parse("nope").is_none());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn plot_of_missing_input_fails_before_any_gpu_work() {
    // Import (and therefore the file check) happens before device creation, so
    // a missing file is an `invalid_input` failure even without a GPU adapter.
    let invocation = CliInvocation::new(CliOperation::Plot, "missing.dwg");
    let error = run(&invocation).unwrap_err();
    assert_eq!(error.code, error_code::INVALID_INPUT);
    assert_eq!(error.exit_code(), 1);
}

/// Path to the committed synthetic plot fixture.
#[cfg(not(target_arch = "wasm32"))]
fn plot_fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/plot/a4-layout.dwg")
}

/// A scratch directory on the target filesystem (`/tmp` may be a small tmpfs).
#[cfg(not(target_arch = "wasm32"))]
fn plot_scratch_dir(label: &str) -> std::path::PathBuf {
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("yacr-plot-{label}-{}", unique_counter()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// PNG `IHDR` dimensions, or `None` when the bytes are not a valid PNG header.
#[cfg(not(target_arch = "wasm32"))]
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn plot_renders_a_layout_to_a_non_empty_png() {
    use cad_render_wgpu::headless::enumerate_adapters;
    use cad_render_wgpu::BackendPreference;
    // No adapter: the operation would report `gpu_failure`, which is the honest
    // answer but not what this test asserts. Skip instead of failing CI.
    if enumerate_adapters(BackendPreference::Auto).is_empty() {
        eprintln!("no wgpu adapter; skipping plot render test");
        return;
    }
    let fixture = plot_fixture();
    assert!(fixture.exists(), "missing fixture {}", fixture.display());
    let dir = plot_scratch_dir("render");
    let png = dir.join("out.png");
    let mut invocation = CliInvocation::new(CliOperation::Plot, fixture);
    invocation.render_width = 320;
    invocation.render_height = 452;
    invocation.png = Some(png.clone());

    let json = run(&invocation).expect("plot must succeed on the synthetic fixture");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["operation"], "plot");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["width"], 320);
    assert_eq!(value["height"], 452);
    // The fixture's standalone PLOTSETTINGS object is A3 at 90°, so provenance
    // is imported and the paper is A3 (proving the import path ran).
    assert_eq!(value["paper"]["provenance"], "imported");
    assert_eq!(value["paper"]["rotation_degrees"], 90.0);
    assert_eq!(value["paper"]["width"], 297.0);

    let bytes = std::fs::read(&png).expect("PNG written");
    assert!(!bytes.is_empty());
    assert!(bytes.len() as u64 == value["bytes"].as_u64().unwrap());
    assert_eq!(png_dimensions(&bytes), Some((320, 452)));
    assert!(
        value["pixels"]["non_background"].as_u64().unwrap() > 0,
        "the layout's line must rasterize to at least one pixel"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn plot_unknown_layout_is_an_input_error() {
    // Layout selection happens before any GPU device is created, so this is an
    // input error regardless of adapter availability.
    let mut invocation = CliInvocation::new(CliOperation::Plot, plot_fixture());
    invocation.layout = Some("NoSuchLayout".into());
    let error = run(&invocation).unwrap_err();
    assert_eq!(error.code, error_code::INVALID_INPUT);
    assert!(error.message.contains("NoSuchLayout"), "{}", error.message);
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
