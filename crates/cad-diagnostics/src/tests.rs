//! Unit tests.

use super::*;

#[test]
fn paths_urls_and_document_names_are_redacted() {
    let text =
        "打开 /home/user/secret.dwg 失败 或 C:\\Users\\me\\plan.DWG 见 https://example.com/a.dwg";
    let redacted = redact_text(text);
    assert!(!redacted.contains("secret"));
    assert!(!redacted.contains("plan.DWG"));
    assert!(!redacted.contains("example.com"));
    assert!(redacted.contains("打开"));
}

#[test]
fn package_encoding_contains_no_paths() {
    let package = DiagnosticPackage {
        build_version: "0.1.0".into(),
        upstream_commit: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: "io.error".into(),
            message: "cannot read /tmp/private/project.dwg".into(),
        }],
        memory: MemoryBudget {
            file_bytes: 1,
            domain_bytes: 2,
            cpu_geometry_bytes: 3,
            gpu_estimated_bytes: 4,
            atlas_bytes: 5,
            attachment_bytes: 6,
        },
        timings: LoadTimings {
            parse_ms: 1.0,
            build_ms: 2.0,
            upload_ms: 3.0,
            first_usable_ms: 4.0,
            complete_ms: 5.0,
        },
        backend: "WebGL2 (swiftshader)".into(),
    };
    let encoded = package.encode_redacted().unwrap();
    let text = String::from_utf8(encoded).unwrap();
    assert!(!text.contains("/tmp/private"));
    assert!(text.contains("io.error"));
}

#[test]
fn benchmark_report_schema_is_explicit_about_absence() {
    let report = BenchmarkReport {
        sample_hash: Some([0xab; 32]),
        release_build: true,
        timings: MeasuredTimings {
            parse_ms: Some(1.25),
            build_ms: Some(2.5),
            upload_ms: None,
            first_usable_ms: None,
            complete_ms: Some(3.75),
        },
        memory: MeasuredMemory {
            file_bytes: Some(100),
            domain_bytes: None,
            cpu_geometry_bytes: Some(200),
            gpu_estimated_bytes: Some(300),
            atlas_bytes: None,
            attachment_bytes: None,
        },
        budgets: BenchmarkBudgets {
            cpu_bytes: Some(1000),
            upload_bytes_per_frame: Some(500),
            queued_tasks: Some(8),
            max_vertices_per_frame: Some(100),
            max_triangles_per_frame: Some(50),
        },
        context: MeasuredContext {
            sample_hash: Some([0xab; 32]),
            device: Some("lavapipe".into()),
            browser: None,
            release_build: true,
            viewport: None,
            quality_configuration: Some("default".into()),
        },
        over_budget: vec!["bytes".into()],
    };
    let value = report.to_json();
    assert_eq!(value["claim"], "measurement, not compatibility");
    assert_eq!(value["environment"]["release_build"], true);
    assert_eq!(value["environment"]["profile"], "release");
    // A measured phase is a rounded number...
    assert_eq!(value["timings_ms"]["parse"], 1.25);
    assert_eq!(value["timings_ms"]["build"], 2.5);
    // ...an unmeasured one is explicit null, never 0.
    assert!(value["timings_ms"]["upload"].is_null());
    assert!(value["timings_ms"]["first_usable"].is_null());
    // Memory: present vs absent is distinguishable.
    assert_eq!(value["memory_bytes"]["file"], 100);
    assert!(value["memory_bytes"]["domain"].is_null());
    // The hash is lower-case hex, never a path.
    assert_eq!(value["sample_hash"], "ab".repeat(32));
    assert_eq!(value["context"]["sample_hash"], "ab".repeat(32));
    assert_eq!(value["context"]["device"], "lavapipe");
    assert!(value["context"]["browser"].is_null());
    assert_eq!(value["budgets"]["queued_tasks"], 8);
    assert_eq!(value["over_budget"][0], "bytes");
}

#[test]
fn absent_sample_hash_encodes_as_null_not_zeroes() {
    let report = BenchmarkReport {
        sample_hash: None,
        release_build: false,
        timings: MeasuredTimings::default(),
        memory: MeasuredMemory::default(),
        budgets: BenchmarkBudgets::default(),
        context: MeasuredContext::default(),
        over_budget: Vec::new(),
    };
    let value = report.to_json();
    assert!(value["sample_hash"].is_null());
    assert!(value["context"]["sample_hash"].is_null());
    assert!(value["context"]["viewport"].is_null());
    assert_eq!(value["environment"]["profile"], "debug");
    assert!(value["timings_ms"]["complete"].is_null());
}

#[test]
fn measured_timings_view_substitutes_zero_only_for_the_redacted_package() {
    let timings = MeasuredTimings {
        parse_ms: Some(4.0),
        ..MeasuredTimings::default()
    };
    let load = timings.to_load_timings();
    assert_eq!(load.parse_ms, 4.0);
    assert_eq!(load.upload_ms, 0.0);
}

#[test]
fn partial_context_never_upgrades_with_invented_fields() {
    let mut context = MeasuredContext {
        sample_hash: Some([1; 32]),
        device: Some("lavapipe".into()),
        browser: None,
        release_build: true,
        viewport: Some(ViewportId(7)),
        quality_configuration: Some("default".into()),
    };
    // All required fields present: the full host context is recoverable.
    let full = context.to_benchmark_context().expect("complete context");
    assert_eq!(full.viewport, ViewportId(7));
    assert_eq!(full.device, "lavapipe");
    // Dropping a required field makes it explicitly unavailable, not invented.
    context.viewport = None;
    assert!(context.to_benchmark_context().is_none());
}

#[test]
fn model_keeps_every_reason_not_just_the_first() {
    let mut model = DiagnosticsModel::new();
    model.add(
        ObjectId(1),
        DiagnosticReason::partial(
            codes::REPRESENTATION_APPROXIMATE,
            vec![DiagnosticParameter::Identifier("HATCH".into())],
        ),
    );
    model.add(
        ObjectId(1),
        DiagnosticReason::missing(
            codes::REPRESENTATION_UNAVAILABLE,
            vec![DiagnosticParameter::Identifier("TEXT".into())],
        ),
    );
    // Both reasons survive; completeness takes the weakest verdict.
    let entry = model.object(ObjectId(1)).unwrap();
    assert_eq!(entry.reasons.len(), 2);
    assert!(matches!(entry.completeness(), Completeness::Missing(_)));
    assert_eq!(entry.severity(), Severity::Error);
}

#[test]
fn summary_distinguishes_complete_from_partial_and_missing() {
    let mut model = DiagnosticsModel::new();
    model.add(
        ObjectId(1),
        DiagnosticReason::unverified(codes::REPRESENTATION_NOT_IMPLEMENTED, vec![]),
    );
    let summary = model.summary();
    assert!(!summary.complete);
    assert_eq!(summary.objects, 1);
    assert_eq!(summary.unverified_objects, 1);
    assert!(matches!(summary.completeness, Completeness::Unverified));

    let empty = DiagnosticsModel::new().summary();
    assert!(empty.complete);
    assert_eq!(empty.complete_objects, 0);
}

#[test]
fn model_encoding_preserves_codes_parameters_and_summary() {
    let mut model = DiagnosticsModel::new();
    model.add(
        ObjectId(42),
        DiagnosticReason::missing(
            codes::RESOURCE_MISSING,
            vec![DiagnosticParameter::Key("romans.shx".into())],
        ),
    );
    model.add_document(DiagnosticReason::missing(
        codes::RESOURCE_OVER_BUDGET,
        vec![DiagnosticParameter::Limit(1024)],
    ));
    let encoded = DiagnosticPackage::encode_model_redacted(&model, "0.1.0").unwrap();
    let text = String::from_utf8(encoded).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["summary"]["complete"], serde_json::json!(false));
    assert_eq!(value["completeness"], serde_json::json!("missing"));
    assert_eq!(value["objects"][0]["object"], serde_json::json!("42"));
    assert_eq!(
        value["objects"][0]["reasons"][0]["code"],
        serde_json::json!("resource.missing")
    );
    assert_eq!(
        value["objects"][0]["reasons"][0]["parameters"][0]["value"],
        serde_json::json!("romans.shx")
    );
    assert_eq!(
        value["document"][0]["code"],
        serde_json::json!("resource.over_budget")
    );
}
