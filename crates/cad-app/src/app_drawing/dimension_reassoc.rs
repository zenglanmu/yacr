//! `DIMREASSOC`: restore the measured value to overridden/modified dimension
//! text.
//!
//! AutoCAD's Express tool `DIMREASSOC` restores the dimension text that a text
//! override has replaced with the value the dimension actually measures. This
//! build stores exactly one such override: `DimensionGeometry::text_override`
//! (the same entity field `DIMEDIT TEXT` writes and `DIMEDIT RESET` clears), so
//! the command's whole support domain is "clear that field on every selected
//! dimension and keep the unaffected ones untouched".
//!
//! The selection is the input (the pre-selection, no interactive prompt): every
//! selected dimension with an override is rebuilt through the one shared
//! synthesis ([`super::dimension_edit::rebuild_dimension`]) with no override, so
//! the display text returns to the measured value; a dimension that has no
//! override is named as a no-op and is not rewritten; a non-dimension selection
//! is a named refusal before anything is staged. Every real change goes through
//! **one transaction / one undo step** ([`Application::commit_dimension_changes`]);
//! when nothing is writable no transaction is opened at all.
//!
//! Association to source objects (`DIMASSOC`) is an ADR global-scope gap shared
//! by every dimension command in this build, not a gap of this command.

use super::dimension_edit::{drawing_of, invalid, rebuild_dimension, resolve_selected_dimensions};
use super::*;
use crate::app_reports_ext::report_outcome;

impl Application {
    /// `DIMREASSOC`: clear the text override on every selected dimension.
    pub(crate) fn dim_reassoc(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "dimreassoc.payload",
                "DIMREASSOC takes no payload; the pre-selection names the dimensions",
            ));
        }
        let references = session.selection.refs();
        if references.is_empty() {
            return Err(invalid(
                "dimreassoc.selection",
                "DIMREASSOC needs at least one selected dimension",
            ));
        }
        let drawing = drawing_of(self, command)?;
        let selected = resolve_selected_dimensions(&drawing, command, references, "dimreassoc")?;
        let mut changes = Vec::new();
        let mut restored = Vec::new();
        let mut without_override = Vec::new();
        for target in selected {
            if target.dimension.text_override().is_none() {
                // Nothing to restore: name it and leave the row byte-identical.
                without_override.push(target.entity.id.0);
                continue;
            }
            let after = rebuild_dimension(
                &target.dimension,
                target.dimension.definition().clone(),
                None,
                None,
                target.dimension.text_h_align(),
            )
            .map_err(|error| {
                invalid(
                    "dimreassoc.degenerate",
                    format!("the dimension has no synthesizable measured display: {error}"),
                )
            })?;
            let mut entity = target.entity.clone();
            entity.geometry = SemanticGeometry::Dimension(Box::new(after));
            if entity.geometry == target.entity.geometry {
                // Defensive: an override that already equals the measured text
                // still differs in the stored field, so this cannot be a silent
                // no-op; treat it as no work rather than open a transaction.
                without_override.push(target.entity.id.0);
                continue;
            }
            restored.push(target.entity.id.0);
            changes.push((target.entity, entity));
        }
        if changes.is_empty() {
            let mut diagnostics: Vec<Diagnostic> = without_override
                .iter()
                .map(|id| Diagnostic {
                    object: Some(ObjectId(*id)),
                    code: "dimreassoc.no_override".into(),
                    message: format!(
                        "DIMREASSOC: dimension {id} has no text override, so its measured value \
                         is already shown; nothing was written"
                    ),
                })
                .collect();
            diagnostics.push(Diagnostic {
                object: None,
                code: "dimreassoc.unchanged".into(),
                message: "DIMREASSOC: no selected dimension carries a text override \
                          (no transaction)"
                    .into(),
            });
            return Ok(report_outcome(diagnostics));
        }
        let count = changes.len();
        let mut outcome = self.commit_dimension_changes(
            command,
            changes,
            "dimreassoc",
            "dimreassoc.applied",
            &format!("DIMREASSOC: restored the measured value on {count} dimension(s)"),
        )?;
        for id in restored {
            outcome.diagnostics.push(Diagnostic {
                object: Some(ObjectId(id)),
                code: "dimreassoc.restored".into(),
                message: format!(
                    "DIMREASSOC: dimension {id} shows its measured value again (text override \
                     cleared)"
                ),
            });
        }
        for id in without_override {
            outcome.diagnostics.push(Diagnostic {
                object: Some(ObjectId(id)),
                code: "dimreassoc.no_override".into(),
                message: format!(
                    "DIMREASSOC: dimension {id} has no text override, so its measured value is \
                     already shown; nothing was written"
                ),
            });
        }
        Ok(outcome)
    }
}
