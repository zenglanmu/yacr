//! Wave 9 **reports, real 2D edits and named refusals** for the extended batch.
//!
//! `UNITS` reports the live document units and Units-dialog settings, and its
//! typed forms write the representable subset: the declared unit + linear
//! precision (`cad_domain::UnitContext`) and the document-level length/angle
//! formats, base angle and clockwise direction (`cad_db::UnitsDisplay`). This
//! build has no Units *dialog* (a host/UI decision) and reads only settings it
//! really stores — the remaining display-consumption and `$INSUNITS` gaps are
//! named in the report instead of being invented.
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
            return self.units_command(session, command, "UNITS");
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

    /// `UNITS` / `-UNITS`: report or write the document's units and display
    /// settings.
    ///
    /// The bare form reports the live `Document.units` value and the live
    /// `cad_db::UnitsDisplay` settings (never a hard-coded "4 decimal places"),
    /// names the exact remaining display gap (`$INSUNITS` insertion scaling and
    /// the fixed-decimal readouts), and changes nothing. A typed form writes only
    /// the named settings through the existing document-header path: one
    /// revision, a header-only `ChangeSet`, and no undo step because
    /// `cad-history` has no header patch kind. An empty patch is refused, never a
    /// silent success.
    pub(crate) fn units_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        name: &str,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Units(payload) = &command.payload else {
            require_none_payload(command, name)?;
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            return Ok(report_outcome(units_report(
                name,
                &document.units,
                document.drawing.units_display(),
            )));
        };
        if payload.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "units.empty: {name} names no setting; expected `<unit> <precision>`, \
                 `LENGTH <format>`, `ANGLE <format> <precision>` or `ANGLE DIRECTION ...`; \
                 nothing was written"
            )));
        }
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        if let Some(declared) = &payload.declared {
            if declared.decimal_places > 8 {
                return Err(CadError::InvalidInput(format!(
                    "units.invalid_precision: {name} accepts 0 to 8 decimal places, got {}; \
                     nothing was written",
                    declared.decimal_places
                )));
            }
        }
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let current_units = document.units.clone();
        let mut next_display = *document.drawing.units_display();
        if let Some(format) = payload.length_format {
            next_display.length_format = format;
        }
        if let Some(format) = payload.angle_format {
            next_display.angle_format = format;
        }
        if let Some(precision) = payload.angle_precision {
            next_display.angle_precision = precision;
        }
        if let Some(base) = payload.angle_base_degrees {
            next_display.angle_base_degrees = base;
        }
        if let Some(clockwise) = payload.angle_clockwise {
            next_display.angle_clockwise = clockwise;
        }
        // Validate the whole resulting block before the first mutation; an
        // out-of-range precision or non-finite base angle leaves the document
        // exactly as it was.
        next_display.validate()?;
        let next_units = payload.declared_context();
        let units_changed = next_units
            .as_ref()
            .is_some_and(|units| *units != current_units);
        let display_changed = next_display != *document.drawing.units_display();
        if !units_changed && !display_changed {
            return Ok(report_outcome(vec![
                diagnostic(
                    "units.unchanged",
                    format!(
                        "{name}: the document already stores the requested settings; no revision \
                         was raised"
                    ),
                ),
                units_gap(name),
            ]));
        }
        let transaction = crate::app_drawing::next_transaction_id();
        let document = self
            .workspace
            .documents
            .get_mut(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let changeset = if display_changed {
            Arc::make_mut(&mut document.drawing).set_units_display(
                next_display,
                transaction,
                "units settings changed",
            )?
        } else {
            Arc::make_mut(&mut document.drawing)
                .note_document_header_change(transaction, "units changed")
        };
        if let Some(units) = next_units {
            document.units = units;
        }
        let after = changeset.after.0;
        let mut rows = Vec::with_capacity(5);
        if units_changed {
            rows.push(diagnostic(
                "units.set",
                format!(
                    "{name}: the document now declares {} units and displays coordinates with {} \
                     decimal place(s) (revision {after}); stored coordinates were not rescaled",
                    unit_label(&document.units.display),
                    document.units.decimal_places
                ),
            ));
        }
        if display_changed {
            rows.push(diagnostic(
                "units.set_display",
                units_display_summary(name, &next_display, after),
            ));
        }
        rows.push(units_gap(name));
        rows.push(diagnostic(
            "units.no_history",
            format!(
                "{name}: this write published one revision and one header-only ChangeSet and \
                 recorded NO undo step; cad-history has no header patch kind"
            ),
        ));
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(changeset)),
            diagnostics: rows,
            measurement: None,
            point_report: None,
        })
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

/// The read-only `UNITS`/`-UNITS` report over the live document header.
fn units_report(
    name: &str,
    units: &cad_domain::UnitContext,
    display: &cad_db::UnitsDisplay,
) -> Vec<Diagnostic> {
    let conversion = match units.display_per_source {
        Some(ratio) => format!(
            "source {} → display {} (×{ratio})",
            unit_label(&units.source),
            unit_label(&units.display)
        ),
        None => format!(
            "source {} (untyped; no display conversion)",
            unit_label(&units.source)
        ),
    };
    vec![
        diagnostic(
            "units.report",
            format!(
                "{name}: coordinates and distances are declared in {} with {} decimal place(s); \
                 {conversion}",
                unit_label(&units.display),
                units.decimal_places
            ),
        ),
        diagnostic(
            "units.length_format",
            format!(
                "{name}: length format {} (the linear precision above is the stored decimal-place \
                 count)",
                display.length_format.as_token()
            ),
        ),
        diagnostic(
            "units.angle_format",
            units_display_angle_line(name, display),
        ),
        units_gap(name),
    ]
}

/// One line naming the live angular settings and the direction preset they
/// imply.
fn units_display_angle_line(name: &str, display: &cad_db::UnitsDisplay) -> String {
    let direction = match display.base_direction() {
        Some(preset) => format!("{preset} (base angle {}°)", display.angle_base_degrees),
        None => format!("Other (base angle {}°)", display.angle_base_degrees),
    };
    format!(
        "{name}: angles are formatted as {} with {} decimal place(s); zero angle points {}; \
         positive angles run {}",
        display.angle_format.as_token(),
        display.angle_precision,
        direction,
        display.rotation_token()
    )
}

/// The one-line write report for a changed `UnitsDisplay`.
fn units_display_summary(name: &str, display: &cad_db::UnitsDisplay, revision: u64) -> String {
    format!(
        "{name}: stored length format {}, angle format {} with {} decimal place(s), base angle \
         {}° and {} positive angles (revision {revision}); the Units dialog itself is a host UI \
         and this command wrote the settings directly",
        display.length_format.as_token(),
        display.angle_format.as_token(),
        display.angle_precision,
        display.angle_base_degrees,
        display.rotation_token()
    )
}

/// The exact boundary the representable write does not close.
fn units_gap(name: &str) -> Diagnostic {
    diagnostic(
        "units.not_implemented",
        format!(
            "{name}: the Units dialog is a host UI (no dialog is implemented); the stored \
             length/angle formats do not yet reformat every on-screen number — the coordinate \
             readout and measurement log still use fixed decimal counts — and `$INSUNITS` \
             insertion scaling (rescaling an inserted drawing by its source/target units) is NOT \
             MODELED by this build"
        ),
    )
}

/// A stable display label for one declared unit.
fn unit_label(unit: &cad_domain::Unit) -> &'static str {
    match unit {
        cad_domain::Unit::DrawingUnits => "drawing units",
        cad_domain::Unit::Millimeter => "mm",
        cad_domain::Unit::Centimeter => "cm",
        cad_domain::Unit::Meter => "m",
        cad_domain::Unit::Inch => "in",
        cad_domain::Unit::Foot => "ft",
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
