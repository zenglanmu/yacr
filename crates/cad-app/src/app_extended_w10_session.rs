//! Wave 10 **session-token reports** for the extended batch.
//!
//! `-UNITS`, `LWEIGHT`/`-LWEIGHT` and `COLOR`/`-COLOR` report session tokens.
//! The drawing header stores no unit-format table, and `LWEIGHT` is the one
//! token here with **no possible consumer**: `LWDISPLAY` would switch whether
//! stored lineweights are drawn on screen, but `cad-render-wgpu` rasterizes every
//! line at one hairline width and reports `render.lineweight_not_drawn`, so there
//! is no wide-line path for `ON`/`OFF` to switch. That report states exactly
//! that instead of claiming an effect. `COLOR` is the one token a write path
//! reads: the shared create funnel stamps the current ACI on every entity it
//! inserts (`256` = ByLayer writes no per-entity override, exactly the pre-`COLOR`
//! behaviour), so the command's [`CommandPayload::CurrentColor`] payload really
//! moves it. The Select Color dialog itself is still NOT IMPLEMENTED. None of
//! these opens a transaction, bumps the revision or records history; the reports
//! stay Viewer-legal and only the `COLOR` write is refused in Viewer mode.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::require_none_payload;

impl Application {
    /// Dispatch one Wave 10 session-token report.
    pub(crate) fn extended_w10_session(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::DashUnits => {
                require_none_payload(command, "-UNITS")?;
                Ok(report_outcome(vec![
                    diagnostic(
                        "units.report",
                        "-UNITS: coordinates and distances are displayed in drawing units with 4 \
                         decimal places; angles are displayed in degrees; no per-drawing unit \
                         table exists"
                            .into(),
                    ),
                    diagnostic(
                        "units.dialog_not_implemented",
                        "-UNITS: the Units dialog and its precision/insertion-scale/direction \
                         controls are NOT IMPLEMENTED"
                            .into(),
                    ),
                ]))
            }
            ExtendedCommandId::LineWeight | ExtendedCommandId::DashLineWeight => {
                let name = ext.selector();
                require_none_payload(command, name)?;
                let state = if session.drafting_extras.lweight_display() {
                    "ON"
                } else {
                    "OFF"
                };
                Ok(report_outcome(vec![diagnostic(
                    "lweight.report",
                    format!(
                        "{name}: session lineweight display is {state}; this token is stored but \
                         has no renderer consumer in this build — every line is rasterized at one \
                         hairline width (the resolved weight reaches the batch and the GPU uniform \
                         and is reported as `render.lineweight_not_drawn`), so there is no wide-line \
                         path for ON or OFF to switch and no lineweight width or display change is \
                         claimed; the LWEIGHT dialog is NOT IMPLEMENTED"
                    ),
                )]))
            }
            ExtendedCommandId::Color | ExtendedCommandId::DashColor => {
                let name = ext.selector();
                // `COLOR <aci>` / `COLOR BYLAYER`: move the session token the
                // create funnel reads. Session state only — no drawing row, no
                // revision bump, no undo entry — but a Viewer session may not
                // steer what later creations inherit.
                if let CommandPayload::CurrentColor(aci) = &command.payload {
                    if session.app_mode() == AppMode::Viewer {
                        return Err(CadError::PermissionDenied);
                    }
                    session.drafting_extras.set_current_color(*aci)?;
                    let label = color_label(*aci);
                    return Ok(report_outcome(vec![diagnostic(
                        "color.set",
                        format!(
                            "{name}: session current colour is now {label} (token {aci}); every \
                             entity created from now on inherits it, and the drawing is unchanged \
                             (the Select Color dialog is NOT IMPLEMENTED)"
                        ),
                    )]));
                }
                require_none_payload(command, name)?;
                let aci = session.drafting_extras.current_color();
                let label = color_label(aci);
                Ok(report_outcome(vec![diagnostic(
                    "color.report",
                    format!(
                        "{name}: session current colour is {label} (token {aci}); the shared \
                         create funnel stamps this value on every new entity, and the Select \
                         Color dialog is NOT IMPLEMENTED, so nothing is changed"
                    ),
                )]))
            }
            other => Err(CadError::Invariant(format!(
                "not a Wave 10 session report: {}",
                other.selector()
            ))),
        }
    }
}

/// The user-facing label of a session current-colour ACI.
///
/// `256` is AutoCAD's ByLayer token and `0` its ByBlock token; every other value
/// in the closed domain is a concrete palette index. The label is never the
/// evidence on its own — the numeric token is always printed beside it.
fn color_label(aci: i16) -> String {
    match aci {
        256 => "ByLayer".to_string(),
        0 => "ByBlock".to_string(),
        index => format!("ACI {index}"),
    }
}
