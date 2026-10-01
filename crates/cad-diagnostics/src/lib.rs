//! Diagnostics packages and redaction (spec v2.0 §11, §19).
//!
//! A diagnostics package may contain build identity, backend capability,
//! object-type counts, resource budgets and timings — never original drawing
//! bytes, text content, full paths or user identity. `encode_redacted` is the
//! only default export; sample bytes require an explicit opt-in by the caller.

use cad_domain::*;

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

pub struct DiagnosticsModel {
    pub entries: Vec<Diagnostic>,
    pub completeness: Completeness,
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

fn completeness_name(completeness: &Completeness) -> &'static str {
    match completeness {
        Completeness::Complete => "complete",
        Completeness::Partial(_) => "partial",
        Completeness::Missing(_) => "missing",
        Completeness::Unverified => "unverified",
    }
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

    /// Encode a model's entries with completeness, redacted.
    pub fn encode_model_redacted(model: &DiagnosticsModel, build_version: &str) -> CadResult<Vec<u8>> {
        let entries: Vec<serde_json::Value> = model
            .entries
            .iter()
            .map(|diagnostic| {
                serde_json::json!({
                    "code": diagnostic.code,
                    "message": redact_text(&diagnostic.message),
                })
            })
            .collect();
        let document = serde_json::json!({
            "build_version": build_version,
            "completeness": completeness_name(&model.completeness),
            "entries": entries,
            "redacted": true,
        });
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
}