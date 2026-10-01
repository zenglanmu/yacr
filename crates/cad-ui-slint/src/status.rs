//! Pure mapping logic between the structured diagnostics model and the Slint
//! status/diagnostics drawer (audit U08, N01).
//!
//! The core deliberately keeps diagnostics **locale-independent**: reasons carry
//! a stable machine code plus typed parameters, never prose
//! (`cad_diagnostics::model`). This module is the one place that turns that
//! model into user-facing rows by looking templates up in the active catalog.
//!
//! Everything here is a pure function of `(DiagnosticsModel, MessageSource)` so
//! it can be unit-tested without a Slint host and reused by any host that wants
//! to push the drawer state.
//!
//! **Reachability gap (honest):** `cad-app` does not currently hand a
//! `DiagnosticsModel` to `cad-ui-slint`; the importer/representation paths that
//! build one live behind `apps/**` and the CLI. The drawer is therefore only
//! populated when a host explicitly calls [`DiagnosticsPanelState`] /
//! `UiHandle::set_diagnostics_state`. This file makes that possible and testable;
//! it does not claim the live wire-up exists yet (see `docs/diagnostics-ui.md`).

use cad_diagnostics::model::{DiagnosticParameter, DiagnosticsModel, Severity};
use cad_diagnostics::DiagnosticsSummary;
use cad_domain::Completeness;

use crate::i18n::MessageSource;

/// One localized row in the diagnostics drawer.
///
/// `code` is the stable, locale-independent machine code (never translated);
/// `severity`, `description` and `details` are display strings produced from the
/// active catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRowUi {
    /// Stable diagnostic code from `cad_diagnostics::model::codes`.
    pub code: String,
    /// Localized severity word ("info"/"warning"/"error" rendered).
    pub severity: String,
    /// Localized description for the code.
    pub description: String,
    /// Localized object label, or empty for document-level reasons.
    pub object: String,
    /// Localized parameter summary (may be empty).
    pub details: String,
}

/// Diagnostics drawer snapshot pushed into the shell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticsPanelState {
    /// One row per reason, document reasons first then per-object reasons.
    pub rows: Vec<DiagnosticRowUi>,
    /// Localized overall summary, e.g. "缺失 2" / "Missing 2".
    pub summary: String,
    /// Localized backend label the rows came from, if the host supplied one.
    pub backend: String,
    /// Explicit empty-state text shown before a host pushes any model.
    pub empty_label: String,
}

/// A string that is either a resolved reason template or an explicit unknown.
///
/// Kept as its own type so the "unknown code" fallback is visible to callers and
/// tests rather than being confused with a real description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasonText {
    /// The code has a catalog template.
    Found(String),
    /// No catalog template exists for this code (shown with `{code}`).
    Unknown(String),
}

/// Look up the localized description template for a diagnostic `code`.
///
/// Codes map to `diagnostic.<code>`; dots in the code are already valid key
/// separators. Unknown codes are reported explicitly, never silently blank.
pub fn reason_text(messages: &MessageSource, code: &str) -> ReasonText {
    let key = format!("diagnostic.{code}");
    let message = messages.message(&key, &[("code", code)]);
    if message.is_found() {
        ReasonText::Found(message.text())
    } else {
        // The fallback key is a catalog key like any other; if even that is
        // missing, `Message::text()` still returns the bracketed key.
        ReasonText::Unknown(
            messages
                .message("diagnostics.unknown_code", &[("code", code)])
                .text(),
        )
    }
}

/// Localized severity word for a reason severity.
pub fn severity_text(messages: &MessageSource, severity: Severity) -> String {
    let key = match severity {
        Severity::Info => "diagnostics.severity.info",
        Severity::Warning => "diagnostics.severity.warning",
        Severity::Error => "diagnostics.severity.error",
    };
    messages.text(key, &[])
}

/// Localized summary key + count for a completeness verdict.
pub fn completeness_text(messages: &MessageSource, completeness: &Completeness) -> String {
    match completeness {
        Completeness::Complete => messages.text("diagnostics.summary.complete", &[]),
        Completeness::Partial(_) => messages.text("diagnostics.summary.partial", &[("count", "1")]),
        Completeness::Missing(_) => messages.text("diagnostics.summary.missing", &[("count", "1")]),
        Completeness::Unverified => messages.text("diagnostics.summary.unverified", &[]),
    }
}

/// Localized rendering of one typed parameter.
pub fn parameter_text(messages: &MessageSource, parameter: &DiagnosticParameter) -> String {
    let (key, value) = match parameter {
        DiagnosticParameter::Object(id) => ("diagnostic.param.object", id.0.to_string()),
        DiagnosticParameter::Key(key) => ("diagnostic.param.key", key.clone()),
        DiagnosticParameter::Identifier(identifier) => {
            ("diagnostic.param.identifier", identifier.clone())
        }
        DiagnosticParameter::Count(count) => ("diagnostic.param.count", count.to_string()),
        DiagnosticParameter::Bytes(bytes) => ("diagnostic.param.bytes", bytes.to_string()),
        DiagnosticParameter::Limit(limit) => ("diagnostic.param.limit", limit.to_string()),
        DiagnosticParameter::Depth(depth) => ("diagnostic.param.depth", depth.to_string()),
    };
    messages.text(key, &[("value", &value)])
}

fn join_parameters(messages: &MessageSource, parameters: &[DiagnosticParameter]) -> String {
    let separator = messages.text("diagnostics.detail.separator", &[]);
    parameters
        .iter()
        .map(|parameter| parameter_text(messages, parameter))
        .collect::<Vec<_>>()
        .join(&separator)
}

fn row_for(
    messages: &MessageSource,
    object: Option<&str>,
    code: &str,
    severity: Severity,
    parameters: &[DiagnosticParameter],
) -> DiagnosticRowUi {
    let description = match reason_text(messages, code) {
        ReasonText::Found(text) | ReasonText::Unknown(text) => text,
    };
    DiagnosticRowUi {
        code: code.to_string(),
        severity: severity_text(messages, severity),
        description,
        object: object.unwrap_or("").to_string(),
        details: join_parameters(messages, parameters),
    }
}

impl DiagnosticsPanelState {
    /// Build the drawer from the real model, localizing every reason.
    ///
    /// Document-level reasons come first, then per-object reasons in model
    /// order. The summary itself is formatted from the model's own counts, so a
    /// document with several missing objects is never reported as complete
    /// (audit U08/F11).
    pub fn from_model(
        model: &DiagnosticsModel,
        messages: &MessageSource,
        backend: impl Into<String>,
    ) -> Self {
        let summary = model.summary();
        let mut rows = Vec::new();
        for reason in &model.document {
            rows.push(row_for(
                messages,
                None,
                &reason.code,
                reason.severity,
                &reason.parameters,
            ));
        }
        for object in &model.objects {
            let object_label = messages.text(
                "diagnostic.param.object",
                &[("value", &object.object.0.to_string())],
            );
            for reason in &object.reasons {
                rows.push(row_for(
                    messages,
                    Some(&object_label),
                    &reason.code,
                    reason.severity,
                    &reason.parameters,
                ));
            }
        }
        DiagnosticsPanelState {
            rows,
            summary: Self::summary_text(messages, &summary),
            backend: backend.into(),
            empty_label: messages.text("diagnostics.empty", &[]),
        }
    }

    /// Format the model's real counts into one localized line.
    ///
    /// The `complete` flag is the model's own truth: it is only true when every
    /// object is `Complete` and no reason remains.
    pub fn summary_text(messages: &MessageSource, summary: &DiagnosticsSummary) -> String {
        let separator = messages.text("diagnostics.detail.separator", &[]);
        let count = |value: usize| value.to_string();
        let mut parts = Vec::new();
        if summary.complete {
            parts.push(messages.text("diagnostics.summary.complete", &[]));
        }
        if summary.partial_objects > 0 {
            parts.push(messages.text(
                "diagnostics.summary.partial",
                &[("count", &count(summary.partial_objects))],
            ));
        }
        if summary.missing_objects > 0 {
            parts.push(messages.text(
                "diagnostics.summary.missing",
                &[("count", &count(summary.missing_objects))],
            ));
        }
        if summary.unverified_objects > 0 {
            parts.push(messages.text(
                "diagnostics.summary.unverified",
                &[("count", &count(summary.unverified_objects))],
            ));
        }
        parts.join(&separator)
    }

    /// Whether a host has pushed any real reason yet.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The catalog key for a stable measurement kind key (e.g. `distance`).
pub fn measurement_kind_key(kind_key: &str) -> String {
    format!("measure.kind.{kind_key}")
}

/// The catalog key for a stable annotation kind key (e.g. `freehand`).
pub fn annotation_kind_key(kind_key: &str) -> String {
    format!("annotation.kind.{kind_key}")
}

/// Every measurement kind label, in `MeasurementToolKind::ALL` order.
pub fn measurement_kind_labels(messages: &MessageSource) -> Vec<String> {
    cad_app::MeasurementToolKind::ALL
        .iter()
        .map(|kind| messages.text(&measurement_kind_key(kind.key()), &[]))
        .collect()
}

/// Every annotation kind label, in `AnnotationToolKind::ALL` order.
pub fn annotation_kind_labels(messages: &MessageSource) -> Vec<String> {
    cad_app::AnnotationToolKind::ALL
        .iter()
        .map(|kind| messages.text(&annotation_kind_key(kind.key()), &[]))
        .collect()
}

/// Every backend choice label, in `Auto, WebGPU, WebGL2` order.
///
/// `WebGPU`/`WebGL2` are product names and are deliberately not translated; only
/// `Auto` has a catalog entry.
pub fn backend_labels(messages: &MessageSource) -> Vec<String> {
    vec![
        messages.text("backend.auto", &[]),
        "WebGPU".to_string(),
        "WebGL2".to_string(),
    ]
}

/// Map a localized combobox label back to a measurement kind.
///
/// This is why the shell never needed a second ordering list: the labels are
/// built from `MeasurementToolKind::ALL`, so position is authoritative and the
/// Chinese-only `from_label` is no longer relied on for the translated chrome.
pub fn measurement_kind_from_label(
    messages: &MessageSource,
    label: &str,
) -> Option<cad_app::MeasurementToolKind> {
    measurement_kind_labels(messages)
        .iter()
        .position(|candidate| candidate == label)
        .and_then(|index| cad_app::MeasurementToolKind::from_index(index as i32))
}

/// Map a localized combobox label back to an annotation kind.
pub fn annotation_kind_from_label(
    messages: &MessageSource,
    label: &str,
) -> Option<cad_app::AnnotationToolKind> {
    annotation_kind_labels(messages)
        .iter()
        .position(|candidate| candidate == label)
        .and_then(|index| cad_app::AnnotationToolKind::from_index(index as i32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_diagnostics::model::codes;
    use cad_diagnostics::model::DiagnosticReason;
    use cad_domain::ObjectId;

    fn zh() -> MessageSource {
        MessageSource::for_locale(crate::Locale::ZhCn)
    }

    fn en() -> MessageSource {
        MessageSource::for_locale(crate::Locale::En)
    }

    #[test]
    fn known_codes_resolve_in_both_locales_and_unknown_codes_are_explicit() {
        for messages in [zh(), en()] {
            match reason_text(&messages, codes::REPRESENTATION_UNAVAILABLE) {
                ReasonText::Found(text) => assert!(!text.is_empty()),
                ReasonText::Unknown(_) => panic!("known code must resolve"),
            }
            match reason_text(&messages, "totally.unknown_code") {
                ReasonText::Unknown(text) => assert!(text.contains("totally.unknown_code")),
                ReasonText::Found(_) => panic!("unknown code must not resolve"),
            }
        }
    }

    #[test]
    fn every_stable_code_has_a_translated_description() {
        // The codes table in cad-diagnostics is the schema; every entry must be
        // localizable in both catalogs or the drawer would show a bracketed key.
        let all = [
            codes::REPRESENTATION_UNAVAILABLE,
            codes::REPRESENTATION_APPROXIMATE,
            codes::REPRESENTATION_NOT_IMPLEMENTED,
            codes::RESOURCE_MISSING,
            codes::RESOURCE_OVER_BUDGET,
            codes::RESOURCE_RECURSION_LIMIT,
            codes::FONT_UNRESOLVED,
            codes::FONT_UNSUPPORTED,
            codes::IMPORT_UNKNOWN_ENTITY,
            codes::PROXY_UNDECODED,
            codes::RENDER_FRAME_OVER_BUDGET,
            codes::RENDER_DEVICE_LOST,
        ];
        for messages in [zh(), en()] {
            for code in all {
                assert!(
                    matches!(reason_text(&messages, code), ReasonText::Found(_)),
                    "code {code} has no catalog description"
                );
            }
        }
    }

    #[test]
    fn panel_keeps_every_reason_and_derives_completeness_from_the_model() {
        let mut model = DiagnosticsModel::new();
        model.add(
            ObjectId(7),
            DiagnosticReason::missing(
                codes::REPRESENTATION_UNAVAILABLE,
                vec![DiagnosticParameter::Identifier("HATCH".into())],
            ),
        );
        model.add(
            ObjectId(7),
            DiagnosticReason::partial(
                codes::REPRESENTATION_APPROXIMATE,
                vec![DiagnosticParameter::Count(2)],
            ),
        );
        model.add_document(DiagnosticReason::unverified(
            codes::RESOURCE_FONT_UNRESOLVED,
            vec![DiagnosticParameter::Key("font:arial".into())],
        ));

        let state = DiagnosticsPanelState::from_model(&model, &zh(), "WebGPU");
        // Document reason + both object reasons are all kept.
        assert_eq!(state.rows.len(), 3);
        assert_eq!(state.rows[0].code, codes::RESOURCE_FONT_UNRESOLVED);
        assert_eq!(state.rows[1].code, codes::REPRESENTATION_UNAVAILABLE);
        assert_eq!(state.rows[2].code, codes::REPRESENTATION_APPROXIMATE);
        // The summary must not claim complete for a missing object.
        assert!(
            state.summary.contains('1'),
            "summary must carry real counts"
        );
        assert_eq!(state.backend, "WebGPU");
    }

    #[test]
    fn empty_model_has_empty_rows_and_an_explicit_empty_label() {
        let state = DiagnosticsPanelState::from_model(&DiagnosticsModel::new(), &en(), "WebGPU");
        assert!(state.is_empty());
        assert!(!state.empty_label.is_empty());
    }

    #[test]
    fn localized_labels_map_back_to_the_right_kind() {
        for messages in [zh(), en()] {
            for (index, kind) in cad_app::MeasurementToolKind::ALL.iter().enumerate() {
                let label = &measurement_kind_labels(&messages)[index];
                assert_eq!(measurement_kind_from_label(&messages, label), Some(*kind));
            }
            for (index, kind) in cad_app::AnnotationToolKind::ALL.iter().enumerate() {
                let label = &annotation_kind_labels(&messages)[index];
                assert_eq!(annotation_kind_from_label(&messages, label), Some(*kind));
            }
            // Technical identifiers never resolve to a translated kind.
            assert_eq!(measurement_kind_from_label(&messages, "distance"), None);
        }
    }

    #[test]
    fn backend_labels_keep_product_names_and_translate_auto() {
        assert_eq!(backend_labels(&zh())[0], "自动");
        assert_eq!(backend_labels(&en())[0], "Auto");
        assert_eq!(backend_labels(&zh())[1], "WebGPU");
        assert_eq!(backend_labels(&en())[2], "WebGL2");
    }

    #[test]
    fn severity_and_summary_are_localized() {
        assert_eq!(severity_text(&zh(), Severity::Error), "错误");
        assert_eq!(severity_text(&en(), Severity::Error), "Error");
        let complete = Completeness::Complete;
        assert_eq!(completeness_text(&zh(), &complete), "完整");
        assert_eq!(completeness_text(&en(), &complete), "Complete");
    }
}
