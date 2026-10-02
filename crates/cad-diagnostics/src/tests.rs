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
