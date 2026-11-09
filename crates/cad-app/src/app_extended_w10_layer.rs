//! Wave 10 **layer / view / scale-list reports** for the extended batch.
//!
//! `-LAYER` reports the drawing's real layer table (name, visibility, off,
//! frozen, locked and the session current layer) and refuses every mutation: this
//! build has no command-line layer editor.
//!
//! `LAYERSTATESAVE` and `NEWVIEW` are thin routes: with the owning typed payload
//! (`LayerState` / `View`) they call the existing session store, so there is one
//! implementation and no second write path. Without that payload they report the
//! missing dialog/prompt instead of inventing a name.
//!
//! `-SCALELISTEDIT` reports the drawing's real `ACAD_SCALELIST` entries;
//! `COPYHIST` reports that no command-history text is stored in the session.
//! All five are read-only and Viewer-legal (the two routes only store session
//! state, never drawing rows).

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::require_none_payload;
use cad_db::DrawingDatabase;

impl Application {
    /// Dispatch one Wave 10 layer/view/report command.
    pub(crate) fn extended_w10_layer(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::DashLayer => self.dash_layer_report(session, command),
            ExtendedCommandId::LayerStateSave => self.layer_state_save(session, command),
            ExtendedCommandId::NewView => self.new_view(session, command),
            ExtendedCommandId::DashScaleListEdit => self.dash_scale_list_report(command),
            ExtendedCommandId::CopyHist => self.copy_hist_report(command),
            other => Err(CadError::Invariant(format!(
                "not a Wave 10 layer command: {}",
                other.selector()
            ))),
        }
    }

    /// `-LAYER`: read-only table report; no mutation form is admitted.
    fn dash_layer_report(
        &self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "-LAYER")?;
        let drawing = document_drawing(self, command)?;
        let mut rows = Vec::new();
        for layer in drawing.layers() {
            let attributes = drawing.layer_attributes(layer.id);
            let current = if layer.id == session.active_layer {
                " [current]"
            } else {
                ""
            };
            rows.push(diagnostic(
                "layer.entry",
                format!(
                    "-LAYER: '{}' id {} visible={} off={} frozen={} locked={}{}",
                    layer.name,
                    layer.id.0,
                    layer.visible,
                    attributes.off,
                    attributes.frozen,
                    attributes.locked,
                    current
                ),
            ));
        }
        rows.push(diagnostic(
            "layer.report_only",
            "-LAYER: read-only table report; creating/deleting/renaming layers, on/off/freeze/\
             thaw/lock/unlock and per-viewport settings are NOT IMPLEMENTED here (use the \
             dedicated LAYER* commands and the layer panel)"
                .into(),
        ));
        Ok(report_outcome(rows))
    }

    /// `LAYERSTATESAVE`: route a `LayerState` payload to the one session store.
    fn layer_state_save(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if matches!(command.payload, CommandPayload::LayerState(_)) {
            self.layer_state(session, command)
        } else {
            Ok(report_outcome(vec![diagnostic(
                "layerstatesave.needs_name",
                "LAYERSTATESAVE: the New Layer State dialog is NOT IMPLEMENTED; supply a \
                 `LAYERSTATE SAVE <name>` line instead, which stores the snapshot in the \
                 session layer-state table"
                    .into(),
            )]))
        }
    }

    /// `NEWVIEW`: route a `View::Save` payload to the one named-view store.
    fn new_view(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if matches!(command.payload, CommandPayload::View(_)) {
            self.view_command(session, command)
        } else {
            Ok(report_outcome(vec![diagnostic(
                "newview.needs_name",
                "NEWVIEW: this build has no window/name prompt; supply `VIEW SAVE <name>` to \
                 store the current camera as a named view"
                    .into(),
            )]))
        }
    }

    /// `-SCALELISTEDIT`: read-only report of the real scale table.
    fn dash_scale_list_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "-SCALELISTEDIT")?;
        let drawing = document_drawing(self, command)?;
        let mut rows = Vec::new();
        for scale in drawing.scales() {
            rows.push(diagnostic(
                "scalelistedit.entry",
                format!(
                    "-SCALELISTEDIT: scale '{}' paper/drawing = {}/{} (factor {})",
                    scale.name,
                    scale.paper_units,
                    scale.drawing_units,
                    scale.factor()
                ),
            ));
        }
        rows.push(diagnostic(
            "scalelistedit.write_unsupported",
            "-SCALELISTEDIT: creating, editing, deleting or reordering scales is NOT \
             IMPLEMENTED (no post-import scale-table write API)"
                .into(),
        ));
        Ok(report_outcome(rows))
    }

    /// `COPYHIST`: no command-history text is stored in the session.
    fn copy_hist_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "COPYHIST")?;
        Ok(report_outcome(vec![diagnostic(
            "copyhist.not_tracked",
            "COPYHIST: this build stores no command-history text in the session; the chrome's \
             command log owns its own lines, so nothing was copied to a clipboard"
                .into(),
        )]))
    }
}

fn document_drawing<'a>(app: &'a Application, command: &Command) -> CadResult<&'a DrawingDatabase> {
    app.workspace
        .documents
        .get(&command.document)
        .map(|document| document.drawing.as_ref())
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))
}
