//! `LAYVPI`: freeze layers in every viewport of the active paper layout.
//!
//! The overrides live in [`cad_db::PaperViewport::layer_overrides`] (per
//! viewport, never the global layer table), so the write is one validated
//! `set_layout_viewports` call recorded as one [`cad_db::LayoutPatch`] — one
//! revision and one undo step. This build has no viewport selection: the
//! unscoped form (`FreezeViewportLayers`) declares the scope "every viewport of
//! the active layout", stated in the `layvpi.frozen` diagnostic and in
//! [`crate::command_id::layout`].
//!
//! AutoCAD's `LAYVPI` freezes in *all viewports except the current one*. This
//! build stores no current-viewport marker for the freeze scope, so that exact
//! default is **not** what the unscoped form does. The targeted form
//! (`FreezeViewportLayersIn`) takes an explicit viewport index, which is how a
//! caller expresses "all except the current one" today: it issues the
//! per-index form for every index it wants and skips the current viewport. The
//! command-line parser (`cad-ui-core`) does not produce that form yet — a
//! documented residual of that out-of-scope lane, not a behaviour of this one.
//!
//! # Residual
//!
//! The unscoped form is `all viewports`, the targeted form is one index. The
//! "except current" default needs a current-viewport marker the session does
//! not own for layer freezes, so it stays unimplemented rather than guessed.

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
        self.freeze_viewport_layers_impl(session, command, layers, None)
    }

    /// Handle `LayoutPayload::FreezeViewportLayersIn`: one viewport index only.
    pub(super) fn freeze_viewport_layers_at(
        &mut self,
        session: &SessionState,
        command: &Command,
        layers: &[String],
        viewport: usize,
    ) -> CadResult<CommandOutcome> {
        self.freeze_viewport_layers_impl(session, command, layers, Some(viewport))
    }

    /// The one `LAYVPI` write, scoped to every viewport or to one index.
    fn freeze_viewport_layers_impl(
        &mut self,
        session: &SessionState,
        command: &Command,
        layers: &[String],
        only: Option<usize>,
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
        if let Some(index) = only {
            if index >= viewports.len() {
                return Err(CadError::InvalidInput(format!(
                    "layvpi.unknown_viewport: layout {} has {} viewport(s); there is no viewport \
                     {index} to freeze",
                    layout_id.0,
                    viewports.len()
                )));
            }
        }
        let mut changed = false;
        for (index, viewport) in viewports.iter_mut().enumerate() {
            if only.is_some_and(|wanted| wanted != index) {
                continue;
            }
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
                        "LAYVPI: {} already freezes the named layer(s); no transaction was opened",
                        match only {
                            Some(index) => format!("viewport {index} of layout {}", layout_id.0),
                            None => format!("every viewport of layout {}", layout_id.0),
                        }
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
                message: match only {
                    Some(index) => format!(
                        "LAYVPI: froze layer(s) {names} in viewport {index} of layout {} ({} of \
                         {viewport_count} viewport(s); the global layer table is unchanged)",
                        layout_id.0,
                        index + 1
                    ),
                    None => format!(
                        "LAYVPI: froze layer(s) {names} in all {viewport_count} viewport(s) of \
                         layout {}; this form has no viewport selection, so every viewport of the \
                         active layout received the override, and the global layer table is \
                         unchanged",
                        layout_id.0
                    ),
                },
            }],
            measurement: None,
            point_report: None,
        })
    }
}
