//! Wave 9 **reports, real 2D edits and named refusals** for the extended batch.
//!
//! `UNITS` is a read-only report of the session display-format token: this
//! build has no UNITS dialog and stores no per-drawing precision/format table,
//! so the report states the fixed display rule instead of inventing a value.
//!
//! `TXTEXP` is a real 2D edit: it explodes the selected `TEXT`/`MTEXT` into font
//! outline `LWPOLYLINE`s through the same font engine the display uses, in one
//! transaction / one undo step. The whole contract — produced geometry, the
//! carrier-supplied font engine, and every `txtexp.*` refusal — lives in the
//! `text_explode` submodule.
//!
//! The remaining names describe features with no model or API here and are
//! refused with a stable `CadError::Unsupported` reason: dimension-style
//! export/import files (`DIMEX`, `DIMIM`) and field updates (`UPDATEFIELD`). A
//! refusal writes nothing, bumps no revision and records no `cad-history` entry.
//!
//! `SPACETRANS <distance>` is a real read-only conversion: the typed distance is
//! read as a **model-space** length and converted to paper units for the active
//! layout viewport through the same exact `viewport_transform` the display and
//! measurement paths use. The inverse direction is reported too, and a
//! model-space session or an unresolved viewport is refused by name.

use super::*;
use crate::app_layout::{layout_viewports, paper_writes::target_viewport_index};
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, require_none_payload};
use cad_representation::{viewport_transform, ViewportState};

#[path = "app_extended_w9_text_explode.rs"]
pub(crate) mod text_explode;

/// Install the font engine `TXTEXP` outlines with.
///
/// The carrier (a host that already loads shaping fonts for the display) calls
/// this once per process; `cad-app` never loads font bytes itself. Re-exported
/// from the crate root so a host adapter has one stable entry point.
pub fn install_outline_fonts(fonts: std::sync::Arc<cad_representation::FontEngine>) -> bool {
    text_explode::install_outline_fonts(fonts)
}

impl Application {
    /// Dispatch one Wave 9 report/refusal.
    pub(crate) fn extended_w9_reports(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        if matches!(ext, ExtendedCommandId::Units) {
            require_none_payload(command, "UNITS")?;
            return Ok(report_outcome(vec![
                diagnostic(
                    "units.report",
                    "UNITS: coordinates and distances are displayed in drawing units with 4 \
                     decimal places; angles are displayed in degrees; the drawing header stores \
                     no linear/angular unit-format table, so no per-drawing override exists"
                        .into(),
                ),
                diagnostic(
                    "units.dialog_not_implemented",
                    "UNITS: the AutoCAD Units dialog (precision, insertion scale, direction \
                     control) is NOT IMPLEMENTED; this report changes nothing"
                        .into(),
                ),
            ]));
        }
        if matches!(ext, ExtendedCommandId::TextExplode) {
            return self.text_explode(session, command);
        }
        if matches!(ext, ExtendedCommandId::SpaceTrans) {
            return self.space_trans(session, command);
        }
        require_none_payload(command, ext.selector())?;
        Err(CadError::Unsupported(reason(ext)))
    }

    /// `SPACETRANS <distance>`: convert a model-space distance to paper units for
    /// the active layout viewport (and report the inverse direction).
    ///
    /// The viewport is the same "active viewport" every paper command uses
    /// ([`target_viewport_index`]): an explicit index when one exists, the
    /// `VPMAX`-maximized viewport of this layout, else viewport `0`. The exact
    /// `paper_per_model` factor comes from [`viewport_transform`]; a viewport the
    /// representation layer cannot resolve is refused by name, never guessed.
    fn space_trans(&self, session: &SessionState, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::SpaceTrans(distance) = &command.payload else {
            return Err(CadError::InvalidInput(
                "spacetrans.invalid_payload: SPACETRANS needs `<distance>`".into(),
            ));
        };
        let distance = *distance;
        if !distance.is_finite() || distance <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "spacetrans.invalid_distance: SPACETRANS needs one finite positive distance, got \
                 {distance}"
            )));
        }
        let layout_id = match &session.active_space {
            SpaceId::Paper(layout_id) => *layout_id,
            _ => {
                return Err(CadError::Unsupported(
                    "spacetrans.model_space: SPACETRANS needs an active paper layout with a \
                     viewport; switch to a layout first"
                        .into(),
                ))
            }
        };
        let drawing = document_drawing(self, command)?;
        let viewports = layout_viewports(&drawing, layout_id)?;
        let index = target_viewport_index(session, layout_id, None, &viewports, "spacetrans")?;
        let transform = match viewport_transform(&viewports[index]) {
            ViewportState::Supported(transform) => *transform,
            ViewportState::Unsupported(why) => {
                return Err(CadError::Unsupported(format!(
                    "spacetrans.unsupported_viewport: layout {} viewport {index} cannot be \
                     resolved: {why}",
                    layout_id.0
                )))
            }
        };
        let paper = distance * transform.paper_per_model;
        let model = distance / transform.paper_per_model;
        Ok(report_outcome(vec![
            diagnostic(
                "spacetrans.converted",
                format!(
                    "SPACETRANS: {distance} model unit(s) = {paper} paper unit(s) in layout {} \
                     viewport {index} (direction model→paper)",
                    layout_id.0
                ),
            ),
            diagnostic(
                "spacetrans.factor",
                format!(
                    "SPACETRANS: factor {} paper unit(s) per model unit ({} model unit(s) per \
                     paper unit); model→paper multiplies by that factor, paper→model divides by \
                     it, so the same {distance} read as paper unit(s) = {model} model unit(s)",
                    transform.paper_per_model,
                    1.0 / transform.paper_per_model
                ),
            ),
        ]))
    }
}

fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::DimExport => {
            "dimex.unsupported: dimension-style export files are not modeled"
        }
        ExtendedCommandId::DimImport => {
            "dimim.unsupported: dimension-style import files are not modeled"
        }
        ExtendedCommandId::UpdateField => "updatefield.unsupported: fields are not modeled",
        other => {
            return format!(
                "{}: this command is not a Wave 9 report/refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
