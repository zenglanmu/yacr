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

/// Catalog keys that are **not** failures but still have to reach the user.
///
/// `cad_status.error` is the wire's only free-text channel: the React shell
/// renders it as a one-shot dismissible banner. A recovery snapshot that is newer
/// than the saved file is worthless if nobody is told about it, and there is no
/// second notice channel, so these three keys are surfaced through the same
/// string. They are deliberately a closed list, not a prefix: an informational
/// key must be added here on purpose.
const NOTICE_KEYS: [&str; 3] = [
    "recovery.available",
    "recovery.available_no_hint",
    "recovery.unavailable",
];

/// A locale-neutral failure or notice string from a host status, or `None` for a
/// normal status.
///
/// Failure catalog keys (`command.failed`, `linux.failed`, `fonts.failed`, …),
/// the informational [`NOTICE_KEYS`], and explicit external text are surfaced as
/// technical strings built from the status data; no localized prose is hardcoded
/// here.
pub(super) fn status_error(status: &StatusMessage) -> Option<String> {
    match status {
        StatusMessage::ExternalText(text) => Some(text.clone()),
        StatusMessage::Catalog { key, args } => {
            if !(key == "command.failed"
                || key.ends_with(".failed")
                || NOTICE_KEYS.contains(&key.as_str()))
            {
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
