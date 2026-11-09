//! Wave 13 **page-setup writes plus read-only conversion reports**.
//!
//! `PAGESETUP` / `-PAGESETUP` with no payload report each layout's stored
//! `PLOTSETTINGS` record through `DrawingDatabase::plot_settings` (paper
//! name/size, rotation, custom scale, plot type, paper units and provenance).
//! With a `CommandPayload::Layout(PageSetup | PageSetupIn)` they are real
//! writes: one revision and one undo step through
//! `Application::page_setup_write` (`cad-db`'s `set_plot_settings`).
//! `PSETUPIN` / `-PSETUPIN` always take the write path and require that payload.
//! `CONVERTPOLY` reports that this build stores one `LWPOLYLINE`
//! representation, so there is nothing to convert.
//!
//! The report form opens no transaction, bumps no revision and records no
//! history; the write form is Work-only.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::require_none_payload;

impl Application {
    /// Dispatch one Wave 13 report/write.
    pub(crate) fn extended_w13_reports(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::PageSetup | ExtendedCommandId::DashPageSetup => {
                match &command.payload {
                    CommandPayload::Layout(payload) => {
                        self.page_setup_write(command, payload.as_ref())
                    }
                    _ => self.page_setup_report(command, ext.selector()),
                }
            }
            ExtendedCommandId::PageSetupIn | ExtendedCommandId::DashPageSetupIn => match &command
                .payload
            {
                CommandPayload::Layout(payload) => self.page_setup_write(command, payload.as_ref()),
                _ => Err(CadError::InvalidInput(format!(
                    "{} needs a Layout(PageSetupIn) payload",
                    ext.selector()
                ))),
            },
            ExtendedCommandId::ConvertPoly => self.convert_poly_report(command),
            other => Err(CadError::Invariant(format!(
                "not a Wave 13 report: {}",
                other.selector()
            ))),
        }
    }

    fn page_setup_report(&self, command: &Command, name: &str) -> CadResult<CommandOutcome> {
        require_none_payload(command, name)?;
        let drawing = document_drawing(self, command)?;
        let mut rows = Vec::new();
        for layout in drawing.layouts() {
            let Some(settings) = drawing.plot_settings(layout.id) else {
                rows.push(diagnostic(
                    "pagesetup.no_record",
                    format!(
                        "{name}: layout '{}' stores no PLOTSETTINGS record; nothing was invented",
                        layout.name
                    ),
                ));
                continue;
            };
            let (width, height) = settings.rotated_size();
            rows.push(diagnostic(
                "pagesetup.entry",
                format!(
                    "{name}: layout '{}' paper='{}' rotated_size={}x{} rotation={:?} \
                     scale={}/{} plot_type={:?} paper_units={:?} provenance={:?}",
                    layout.name,
                    settings.paper_size_name,
                    width,
                    height,
                    settings.rotation,
                    settings.scale_numerator,
                    settings.scale_denominator,
                    settings.plot_type,
                    settings.paper_units,
                    settings.provenance
                ),
            ));
        }
        rows.push(diagnostic(
            "pagesetup.report_only",
            format!(
                "{name}: read-only report; use the typed PAGESETUP <layout> <paper> <width> \
                 <height> [<num>:<den>] [0|90|180|270] form to write a page setup"
            ),
        ));
        Ok(report_outcome(rows))
    }

    fn convert_poly_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "CONVERTPOLY")?;
        Ok(report_outcome(vec![diagnostic(
            "convertpoly.single_representation",
            "CONVERTPOLY: this build stores a single LWPOLYLINE representation (with bulge \
             arcs); there is no legacy lightweight/heavy polyline pair to convert, so nothing \
             was changed"
                .into(),
        )]))
    }
}

fn document_drawing<'a>(
    app: &'a Application,
    command: &Command,
) -> CadResult<&'a cad_db::DrawingDatabase> {
    app.workspace
        .documents
        .get(&command.document)
        .map(|document| document.drawing.as_ref())
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))
}
