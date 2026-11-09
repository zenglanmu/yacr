//! `LAYVPI`: freeze layers in every viewport of the active paper layout.
//!
//! The overrides live in [`cad_db::PaperViewport::layer_overrides`] (per
//! viewport, never the global layer table), so the write is one validated
//! `set_layout_viewports` call recorded as one [`cad_db::LayoutPatch`] — one
//! revision and one undo step. This build has no viewport selection: the
//! declared scope is every viewport of the active layout, stated in the
//! `layvpi.frozen` diagnostic and in [`crate::command_id::layout`].

use super::*;

use cad_history::layout_patch;

use crate::app_drawing::{layout_row, next_transaction_id};
use crate::app_layout::{layout_viewports, require_paper_space};
use crate::app_layout_names::resolve_layer_name;

impl Application {
    /// Handle `LayoutPayload::FreezeViewportLayers`.
    pub(super) fn freeze_viewport_layers(
        &mut self,
        session: &SessionState,
        command: &Command,
        layers: &[String],
    ) -> CadResult<CommandOutcome> {
        let layout_id = require_paper_space(session, "layvpi")?;
        if layers.is_empty() {
            return Err(CadError::InvalidInput(
                "layvpi.invalid_argument: LAYVPI needs at least one layer name".into(),
            ));
        }
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        // Resolve every name first: an unknown or ambiguous name refuses before
        // anything mutates, so a bad argument never freezes a subset. Repeated
        // names collapse to one override.
        let mut frozen: Vec<(LayerId, String)> = Vec::with_capacity(layers.len());
        for name in layers {
            let id = resolve_layer_name(&drawing, name)?;
            if frozen.iter().any(|(known, _)| *known == id) {
                continue;
            }
            let canonical = drawing
                .layers()
                .find(|layer| layer.id == id)
                .map(|layer| layer.name.clone())
                .unwrap_or_else(|| name.clone());
            frozen.push((id, canonical));
        }
        let mut viewports = layout_viewports(&drawing, layout_id)?;
        if viewports.is_empty() {
            return Err(CadError::Unsupported(format!(
                "layvpi.no_viewport: layout {} has no viewport; LAYVPI freezes layers per \
                 viewport and there is no viewport to receive the override",
                layout_id.0
            )));
        }
        let mut changed = false;
        for viewport in &mut viewports {
            for (id, _) in &frozen {
                if viewport.layer_overrides.get(id) != Some(&true) {
                    viewport.layer_overrides.insert(*id, true);
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "layvpi.unchanged".into(),
                    message: format!(
                        "LAYVPI: every viewport of layout {} already freezes the named layer(s); \
                         no transaction was opened",
                        layout_id.0
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let before = layout_row(&drawing, layout_id);
        let transaction = next_transaction_id();
        let mut candidate = drawing;
        let viewport_count = viewports.len();
        let change_set =
            candidate.set_layout_viewports(layout_id, viewports, transaction, "layvpi")?;
        let after = layout_row(&candidate, layout_id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            "layvpi",
            vec![layout_patch(layout_id, before, after)],
        )?;
        let names = frozen
            .iter()
            .map(|(_, name)| format!("{name:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "layvpi.frozen".into(),
                message: format!(
                    "LAYVPI: froze layer(s) {names} in all {viewport_count} viewport(s) of \
                     layout {}; this build has no viewport selection, so every viewport of the \
                     active layout received the override, and the global layer table is unchanged",
                    layout_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
