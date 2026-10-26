//! `LAYISO` (isolate the selection's layers) and the `LAYUNISO`/`LAYERP` restore
//! path.
//!
//! `LAYISO` is a **session view operation**: it never touches the drawing's layer
//! table, the database revision or history. It computes a keep-set and then hides
//! every other layer through the one existing atomic bulk path
//! ([`Application::set_layer_visibilities`], the `CommandPayload::LayerVisibilities`
//! implementation): validate-the-whole-batch-first, clone-set-replace, no partial
//! state, session overrides only.
//!
//! ## Our declared rule: the current layer is always in the keep-set
//!
//! The keep-set is
//!
//! ```text
//! keep = { layer of every selected entity } ∪ { session current layer }
//! ```
//!
//! The current layer is included **unconditionally**. The repo already refuses to
//! make an off/frozen layer current and refuses to turn the current layer off or
//! freeze it (`layer.current_off` / `layer.current_frozen`, see
//! `app_drawing::layer_edit`), so hiding it here would be the same contradiction
//! reached through another door — and the next create would silently store
//! geometry on an invisible layer. When the current layer carried **no** selected
//! entity, a `layers.isolate.current_layer` diagnostic names it, so the extra
//! visible layer is explained rather than mysterious.
//!
//! ## No-op
//!
//! When every layer already belongs to the keep-set there is nothing to hide:
//! the command writes **no** override (no churn, no scene rebuild) and reports
//! `layers.isolate.noop` instead of pretending something happened.
//!
//! ## Restore
//!
//! `LAYUNISO` and `LAYERP` are **aliases of the existing restore command**
//! ([`CommandId::RestoreLayers`], `RESTORELAYERS`): both clear the session's
//! temporary overrides, which is exactly what un-isolating means here. A second
//! restore path is deliberately not invented; `LAYERP` (AutoCAD's "previous layer
//! state") has no layer-state stack in this build and therefore names the one state
//! it can really restore.

use super::*;

impl Application {
    /// `LAYISO`: hide every layer that is not in the keep-set.
    ///
    /// Allowed in Viewer and Work (a session/display operation), requires at least
    /// one selected entity, and records **no** revision bump, history entry or
    /// database write.
    pub(crate) fn isolate_layers(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput("LAYISO takes no payload".into()));
        }
        let refs = session.selection.refs().to_vec();
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "layers.isolate.needs_selection: select at least one object first".into(),
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let drawing = document.drawing.as_ref();

        // The one pick resolver every edit uses: duplicates collapse, and a
        // sub-element/instance ref is reported rather than silently over-applied.
        let (selected, mut diagnostics) = Self::resolve_selection(
            drawing,
            document_id,
            &refs,
            "isolate",
            "IsolateLayers",
            "isolate",
            true,
        )?;

        let current = session.active_layer;
        let mut keep: std::collections::BTreeSet<LayerId> = std::collections::BTreeSet::new();
        keep.insert(current);
        let mut current_layer_selected = false;
        for entity in &selected {
            keep.insert(entity.layer);
            if entity.layer == current {
                current_layer_selected = true;
            }
        }
        if !current_layer_selected {
            diagnostics.push(Diagnostic {
                object: None,
                code: "layers.isolate.current_layer".into(),
                message: format!(
                    "layer {} is the current layer: LAYISO keeps it visible because a hidden \
                     current layer would receive new geometry the user cannot see",
                    current.0
                ),
            });
        }

        // Everything else in the layer table is hidden. Validated as one batch by
        // the shared bulk path, so a refusal changes nothing.
        let hide: Vec<(LayerId, bool)> = drawing
            .layers()
            .filter(|layer| !keep.contains(&layer.id))
            .map(|layer| (layer.id, false))
            .collect();
        if hide.is_empty() {
            diagnostics.push(Diagnostic {
                object: None,
                code: "layers.isolate.noop".into(),
                message: "LAYISO: every layer is already in the keep-set; no override written"
                    .into(),
            });
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics,
                measurement: None,
                point_report: None,
            });
        }

        let mut outcome = self.set_layer_visibilities(session, document_id, &hide)?;
        diagnostics.push(Diagnostic {
            object: None,
            code: "layers.isolate".into(),
            message: format!(
                "LAYISO: {} layer(s) kept visible, {} hidden (session overrides only)",
                keep.len(),
                hide.len()
            ),
        });
        outcome.diagnostics = diagnostics;
        Ok(outcome)
    }
}
