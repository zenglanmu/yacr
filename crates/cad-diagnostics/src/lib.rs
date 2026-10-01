//! Diagnostics packages and redaction (spec v2.0 §11, §19).
//!
//! A diagnostics package may contain build identity, backend capability,
//! object-type counts, resource budgets and timings — never original drawing
//! bytes, text content, full paths or user identity. `encode_redacted` is the
//! only default export; sample bytes require an explicit opt-in by the caller.

use cad_domain::*;

pub mod budget;
pub mod model;

pub use budget::{BudgetCategory, ResourceBudget};
pub use model::{
    codes, DiagnosticParameter, DiagnosticReason, DiagnosticsModel, DiagnosticsSummary,
    ObjectDiagnostics, Severity,
};

pub struct MemoryBudget {
    pub file_bytes: u64,
    pub domain_bytes: u64,
    pub cpu_geometry_bytes: u64,
    pub gpu_estimated_bytes: u64,
    pub atlas_bytes: u64,
    pub attachment_bytes: u64,
}

pub struct LoadTimings {
    pub parse_ms: f64,
    pub build_ms: f64,
    pub upload_ms: f64,
    pub first_usable_ms: f64,
    pub complete_ms: f64,
}

pub struct BenchmarkContext {
    pub sample_hash: [u8; 32],
    pub device: String,
    pub browser: Option<String>,
    pub release_build: bool,
    pub viewport: ViewportId,
    pub quality_configuration: String,
}

pub struct DiagnosticPackage {
    pub build_version: String,
    pub upstream_commit: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub memory: MemoryBudget,
    pub timings: LoadTimings,
    pub backend: String,
}

/// Replace absolute/relative path-like tokens with `«redacted»`.
///
/// Conservative by design: anything containing a path separator or ending in a
/// known drawing/document extension is dropped, and Windows drive prefixes too.
pub fn redact_text(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    for token in message.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        let is_path = token.contains('/')
            || token.contains('\\')
            || token.len() > 2
                && token.as_bytes().get(1) == Some(&b':')
                && token.as_bytes()[0].is_ascii_alphabetic();
        let lower = token.to_ascii_lowercase();
        let is_document = [".dwg", ".dxf", ".json", ".sat", ".sab", ".shx", ".ttf"]
            .iter()
            .any(|extension| lower.ends_with(extension));
        if is_path || is_document {
            out.push_str("«redacted»");
        } else if token.starts_with("http://") || token.starts_with("https://") {
            out.push_str("«redacted-url»");
        } else {
            out.push_str(token);
        }
    }
    out
}

impl DiagnosticPackage {
    /// Encode a redacted diagnostics package as JSON.
    ///
    /// Spec §19: default redaction removes source bytes, text, full paths and
    /// user identity. The caller must explicitly choose to attach a sample.
    pub fn encode_redacted(&self) -> CadResult<Vec<u8>> {
        let diagnostics: Vec<serde_json::Value> = self
            .diagnostics
            .iter()
            .map(|diagnostic| {
                serde_json::json!({
                    "object": diagnostic.object.map(|id| id.0.to_string()),
                    "code": diagnostic.code,
                    "message": redact_text(&diagnostic.message),
                })
            })
            .collect();
        let document = serde_json::json!({
            "build_version": self.build_version,
            "upstream_commit": self.upstream_commit,
            "backend": self.backend,
            "diagnostics": diagnostics,
            "memory": {
                "file_bytes": self.memory.file_bytes,
                "domain_bytes": self.memory.domain_bytes,
                "cpu_geometry_bytes": self.memory.cpu_geometry_bytes,
                "gpu_estimated_bytes": self.memory.gpu_estimated_bytes,
                "atlas_bytes": self.memory.atlas_bytes,
                "attachment_bytes": self.memory.attachment_bytes,
            },
            "timings": {
                "parse_ms": self.timings.parse_ms,
                "build_ms": self.timings.build_ms,
                "upload_ms": self.timings.upload_ms,
                "first_usable_ms": self.timings.first_usable_ms,
                "complete_ms": self.timings.complete_ms,
            },
            "redacted": true,
        });
        serde_json::to_vec_pretty(&document)
            .map_err(|e| CadError::Invariant(format!("diagnostics encode failed: {e}")))
    }

    /// Encode a structured model's entries with completeness, redacted.
    ///
    /// Every object reason is preserved (audit F11: previously a consumer that
    /// took only the first diagnostic could report a document as complete). The
    /// summary distinguishes `complete` from `partial`/`missing`.
    pub fn encode_model_redacted(
        model: &DiagnosticsModel,
        build_version: &str,
    ) -> CadResult<Vec<u8>> {
        let document = model.to_json(build_version);
        serde_json::to_vec_pretty(&document)
            .map_err(|e| CadError::Invariant(format!("diagnostics encode failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_urls_and_document_names_are_redacted() {
        let text = "打开 /home/user/secret.dwg 失败 或 C:\\Users\\me\\plan.DWG 见 https://example.com/a.dwg";
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
}
