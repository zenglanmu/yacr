//! Unit tests.

use super::*;

#[test]
fn cli_operation_names_parse() {
    for name in [
        "scan",
        "proxy-report",
        "measure",
        "build-representation",
        "render",
        "plot",
        "benchmark",
    ] {
        assert!(CliOperation::parse(name).is_some(), "{name}");
    }
    assert!(CliOperation::parse("nope").is_none());
}

#[test]
fn scene_budget_fails_explicitly_instead_of_oom() {
    let base = CliInvocation::new(CliOperation::FixedViewportRender, "x.dwg");
    // Default caps are generous; an ordinary small scene fits.
    assert!(enforce_scene_budget(10, 1000, &base).is_ok());
    // Tiny caps turn a scene that exceeds them into an explicit error.
    let tiny = CliInvocation {
        max_batches: Some(5),
        max_vertices: Some(100),
        ..base.clone()
    };
    match enforce_scene_budget(6, 10, &tiny) {
        Err(CadError::InvalidInput(message)) => assert!(message.contains("--max-batches")),
        other => panic!("expected batch-budget rejection, got {other:?}"),
    }
    match enforce_scene_budget(1, 101, &tiny) {
        Err(CadError::InvalidInput(message)) => assert!(message.contains("--max-vertices")),
        other => panic!("expected vertex-budget rejection, got {other:?}"),
    }
    // `0` disables each cap for trusted large drawings.
    let disabled = CliInvocation {
        max_batches: Some(0),
        max_vertices: Some(0),
        ..tiny.clone()
    };
    assert!(enforce_scene_budget(100_000_000, 1_000_000_000, &disabled).is_ok());
    // `None` also means unlimited.
    let unlimited = CliInvocation {
        max_batches: None,
        max_vertices: None,
        ..tiny
    };
    assert!(enforce_scene_budget(1_000_000, 1_000_000, &unlimited).is_ok());
}

#[test]
fn gpu_selection_stable_names_parse_for_ui_and_cli() {
    for (name, expected) in [
        ("auto", GpuPreference::Auto),
        ("high", GpuPreference::HighPerformance),
        ("low", GpuPreference::LowPower),
    ] {
        assert_eq!(GpuPreference::parse(name), Some(expected));
    }
    assert_eq!(GpuPreference::parse("discrete"), None);
    assert_eq!(GpuPreference::default(), GpuPreference::Auto);
    assert_eq!(
        CliInvocation::new(CliOperation::FixedViewportRender, "x.dwg").gpu,
        GpuPreference::Auto
    );
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
fn synthetic_plot_scene_is_a_drawable_a4_sheet() {
    // The fallback used when a drawing has no paper layout: it must be a real
    // A4 sheet with drawable paper-space geometry, not an empty canvas.
    let db = synthetic_plot_database();
    let layout = db.layouts().next().expect("synthetic layout");
    assert_eq!(layout.name, "Synthetic");
    let record = db.plot_settings_for(layout.id);
    assert_eq!((record.paper_width, record.paper_height), (210.0, 297.0));
    assert!(matches!(
        record.provenance,
        cad_db::PlotProvenance::DefaultPage { .. }
    ));
    let page = cad_representation::plan_plot_for_record(
        &record,
        cad_representation::PlotTarget::Pixels {
            width: 320,
            height: 452,
        },
    )
    .unwrap();
    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let context = cad_representation::RepresentationContext::new(
        DocumentId(1),
        TolerancePolicy::default(),
        TaskStamp::new(DocumentId(1), 0),
    );
    let representation =
        cad_representation::build_paper_space(&registry, &db, layout.id, &context, &|_| true)
            .unwrap();
    assert!(!representation.fragments.is_empty());
    // Every synthetic vertex maps inside the planned canvas.
    for fragment in &representation.fragments {
        if let cad_representation::DisplayPrimitive::Lines(points) = &fragment.primitive {
            for point in points.iter() {
                let (x, y) = page.map_paper_point(*point);
                assert!((-1.0..=page.width as f64 + 1.0).contains(&x), "x={x}");
                assert!((-1.0..=page.height as f64 + 1.0).contains(&y), "y={y}");
            }
        }
    }
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
fn render_and_plot_do_not_silently_ignore_invalid_fonts() {
    let dir = plot_scratch_dir("invalid-font");
    let font = dir.join("broken.ttf");
    std::fs::write(&font, b"not a font").unwrap();
    for operation in [CliOperation::FixedViewportRender, CliOperation::Plot] {
        let mut invocation = CliInvocation::new(operation, plot_fixture());
        invocation.fonts.push(("broken".into(), font.clone()));
        let error = run(&invocation).expect_err("font validation must precede GPU work");
        assert_eq!(error.code, error_code::CORRUPT_DATA);
        assert!(
            error.message.contains("font 'broken' cannot be parsed"),
            "{}",
            error.message
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
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

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_drawing_without_images_loads_an_empty_image_cache() {
    // The demo document references no rasters: resolution must be a no-op, not
    // an empty-but-present texture set.
    let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let requested = images_requested(&controller);
    assert!(
        requested.is_empty(),
        "demo document must not request images: {requested:?}"
    );
    let invocation = CliInvocation::new(CliOperation::FixedViewportRender, "demo.dwg");
    let (cache, report) = load_images(&invocation, &requested).unwrap();
    assert!(cache.is_empty());
    assert!(report.is_empty());
    assert!(report.unresolved.is_empty());
    assert!(report.failed.is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn build_scene_without_images_is_unchanged() {
    use cad_app::render_scene::{build_scene_with_space, build_scene_with_space_and_images};
    use cad_representation::SpaceSelection;
    // Threading `None` images must reproduce the pre-image scene byte-for-byte
    // in shape: same batch count, no image batches.
    let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let drawing = controller.drawing().unwrap();
    let stamp = TaskStamp::new(controller.document_id, 0);
    let overrides = controller.session.layer_overrides.clone();
    let plain = build_scene_with_space(
        &drawing,
        stamp.clone(),
        None,
        &overrides,
        SpaceSelection::Model,
    )
    .unwrap();
    let with_none = build_scene_with_space_and_images(
        &drawing,
        stamp,
        None,
        None,
        &overrides,
        SpaceSelection::Model,
    )
    .unwrap();
    assert_eq!(plain.added.len(), with_none.added.len());
    assert!(plain.images.is_empty() && with_none.images.is_empty());
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

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn plot_format_parses_and_defaults_to_png() {
    assert_eq!(PlotFormat::default(), PlotFormat::Png);
    assert_eq!(PlotFormat::parse("png"), Some(PlotFormat::Png));
    assert_eq!(PlotFormat::parse("SVG"), Some(PlotFormat::Svg));
    assert_eq!(PlotFormat::parse(" pdf "), Some(PlotFormat::Pdf));
    assert_eq!(PlotFormat::parse("hpgl"), None);
    assert_eq!(PlotFormat::parse(""), None);
    assert!(PlotFormat::Svg.is_vector());
    assert!(PlotFormat::Pdf.is_vector());
    assert!(!PlotFormat::Png.is_vector());
    assert_eq!(PlotFormat::Pdf.as_str(), "pdf");
}

/// The vector plot fixtures: the committed synthetic A4 layout, and a scratch
/// output directory on the target filesystem.
#[cfg(not(target_arch = "wasm32"))]
fn vector_fixture_output(extension: &str) -> std::path::PathBuf {
    let dir = plot_scratch_dir(&format!("vector-{extension}"));
    dir.join(format!("out.{extension}"))
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn plot_svg_layout_to_a_structurally_valid_document() {
    let png = vector_fixture_output("svg");
    let mut invocation = CliInvocation::new(CliOperation::Plot, plot_fixture());
    invocation.plot_format = PlotFormat::Svg;
    invocation.render_width = 320;
    invocation.render_height = 452;
    invocation.png = Some(png.clone());

    let json = run(&invocation).expect("SVG plot must succeed without a GPU");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["operation"], "plot");
    assert_eq!(value["format"], "svg");
    assert_eq!(value["status"], "ok");
    // The fixture's standalone PLOTSETTINGS object is A3 at 90°, so the vector
    // report carries the same imported provenance and the rotated page.
    assert_eq!(value["paper"]["provenance"], "imported");
    assert_eq!(value["paper"]["rotation_degrees"], 90.0);
    assert_eq!(value["paper"]["width"], 297.0);
    // The physical vector page is positive and independent of the pixel target.
    assert!(value["width_mm"].as_f64().unwrap() > 0.0);
    assert!(value["height_mm"].as_f64().unwrap() > 0.0);

    let bytes = std::fs::read(&png).expect("SVG written");
    assert_eq!(bytes.len() as u64, value["bytes"].as_u64().unwrap());
    assert!(!bytes.is_empty());
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("<?xml version=\"1.0\""));
    assert!(text.contains("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(text.trim_end().ends_with("</svg>"));
    assert!(text.contains("<polyline") || text.contains("<polygon"));
    std::fs::remove_dir_all(png.parent().unwrap()).ok();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn plot_pdf_layout_to_a_structurally_valid_document() {
    let pdf = vector_fixture_output("pdf");
    let mut invocation = CliInvocation::new(CliOperation::Plot, plot_fixture());
    invocation.plot_format = PlotFormat::Pdf;
    invocation.render_width = 320;
    invocation.render_height = 452;
    invocation.png = Some(pdf.clone());

    let json = run(&invocation).expect("PDF plot must succeed without a GPU");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format"], "pdf");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["paper"]["provenance"], "imported");

    let bytes = std::fs::read(&pdf).expect("PDF written");
    assert_eq!(bytes.len() as u64, value["bytes"].as_u64().unwrap());
    assert!(bytes.starts_with(b"%PDF-1.4"));
    // The PDF header carries a binary high-bit comment line, so scan the ASCII
    // structure without requiring the whole file to be valid UTF-8.
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Type /Catalog"));
    assert!(text.contains("/MediaBox"));
    assert!(text.contains("xref"));
    assert!(text.trim_end().ends_with("%%EOF"));
    std::fs::remove_dir_all(pdf.parent().unwrap()).ok();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn vector_plot_of_a_missing_input_fails_before_any_output() {
    let mut invocation = CliInvocation::new(CliOperation::Plot, "missing.dwg");
    invocation.plot_format = PlotFormat::Svg;
    let error = run(&invocation).unwrap_err();
    assert_eq!(error.code, error_code::INVALID_INPUT);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn vector_plot_output_defaults_to_the_format_extension() {
    // No `--png`: the default path must carry the chosen format's extension so a
    // vector document never lands in a filename that claims to be PNG.
    let dir = plot_scratch_dir("vector-default-ext");
    let input = dir.join("sheet.dwg");
    std::fs::copy(plot_fixture(), &input).unwrap();
    let mut invocation = CliInvocation::new(CliOperation::Plot, &input);
    invocation.plot_format = PlotFormat::Pdf;
    let json = run(&invocation).expect("vector plot must succeed");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let output = value["output"].as_str().unwrap();
    assert!(output.ends_with(".plot.pdf"), "{output}");
    assert!(std::path::Path::new(output).exists());
    std::fs::remove_dir_all(&dir).ok();
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
