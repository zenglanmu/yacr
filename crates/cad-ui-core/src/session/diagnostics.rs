//! Locale-neutral, technical diagnostics helpers.
//!
//! These build stable machine strings from real host/engine data; no localized
//! UI prose is produced here.

use cad_domain::CadError;
use cad_engine::protocol::{RawDiagnostic, RawDiagnosticParameter, RequestError, StatusMessage};

use super::status::CadCamera;
/// A camera block from real 2D camera parameters, or `None` when unrenderable.
pub(super) fn camera_status(params: cad_app::Camera2dParams) -> Option<CadCamera> {
    let [x, y] = [params.center.x, params.center.y];
    if !x.is_finite()
        || !y.is_finite()
        || !params.world_per_px.is_finite()
        || params.world_per_px <= 0.0
    {
        return None;
    }
    Some(CadCamera {
        center: [x, y],
        scale: params.world_per_px,
    })
}

/// A locale-neutral failure string from a host status, or `None` for a normal
/// status.
///
/// Failure catalog keys (`command.failed`, `linux.failed`, `fonts.failed`, …)
/// and explicit external text are surfaced as technical strings built from the
/// status data; no localized prose is hardcoded here.
pub(super) fn status_error(status: &StatusMessage) -> Option<String> {
    match status {
        StatusMessage::ExternalText(text) => Some(text.clone()),
        StatusMessage::Catalog { key, args } => {
            if !(key == "command.failed" || key.ends_with(".failed")) {
                return None;
            }
            Some(
                args.iter()
                    .find(|argument| argument.key == "error")
                    .map(|argument| argument.value.clone())
                    .unwrap_or_else(|| key.clone()),
            )
        }
    }
}

/// A technical `resource.missing` report built from the raw resource keys.
pub(super) fn resource_keys_error<'a>(
    code: &str,
    keys: impl Iterator<Item = &'a String>,
) -> String {
    let mut text = code.to_string();
    for key in keys {
        text.push(' ');
        text.push_str(key);
    }
    text
}

/// A technical report from a `resource.missing` diagnostic (code + raw keys).
pub(super) fn resource_diagnostic_text(diagnostic: &RawDiagnostic) -> String {
    let mut text = diagnostic.code.clone();
    for parameter in &diagnostic.parameters {
        if let RawDiagnosticParameter::Key(key) = parameter {
            text.push(' ');
            text.push_str(key);
        }
    }
    text
}

/// A user-facing `CadError` string, unwrapping the explicit refusal variants.
pub(super) fn cad_error_text(error: &CadError) -> String {
    match error {
        CadError::Unsupported(reason) => reason.clone(),
        CadError::NotImplemented(feature) => (*feature).to_string(),
        other => other.to_string(),
    }
}

/// A technical `RequestError` string (the `CadError` inside `Execution` is
/// unwrapped the same way the host does).
pub(super) fn request_error_text(error: &RequestError) -> String {
    match error {
        RequestError::Execution(cad) => cad_error_text(cad),
        other => format!("{other:?}"),
    }
}
