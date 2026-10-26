//! `LAYISO` (isolate the selection's layers), `LAYOFF` (turn the selection's
//! layers off) and the `LAYUNISO`/`LAYERP`/`LAYON` restore path.
//!
//! Both `LAYISO` and `LAYOFF` are **session view operations**: they never touch
//! the drawing's layer table, the database revision or history. Each computes a
//! layer set from the selection and then publishes its hides through the one
//! existing atomic bulk path ([`Application::set_layer_visibilities`], the
//! `CommandPayload::LayerVisibilities` implementation):
//! validate-the-whole-batch-first, clone-set-replace, no partial state, session
//! overrides only. The selection resolution and the hide batch are shared by the
//! two commands ([`Application::selection_layers`] /
//! [`Application::hide_layers_atomically`]) so the pair cannot drift apart.
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
//! `LAYUNISO`, `LAYERP` and `LAYON` are **aliases of the existing restore
//! command** ([`CommandId::RestoreLayers`], `RESTORELAYERS`): all three clear the
//! session's temporary overrides, which is exactly what un-isolating (or turning
//! the layers back on) means here. A second restore path is deliberately not
//! invented; `LAYERP` (AutoCAD's "previous layer state") has no layer-state stack
//! in this build and therefore names the one state it can really restore.

use super::*;

impl Application {
    /// The selection's distinct layers, in first-selection order, plus the
    /// sub-element/instance diagnostics every selection-based edit reports.
    ///
    /// The one preparation step `LAYISO` and `LAYOFF` share: both decide what to
    /// hide from the layers their selected entities live on, so a different
    /// meaning of "the selection" between them would be a silent behavioural
    /// split. The empty-selection refusal stays in each command because its code
    /// is command-specific (`layers.isolate.needs_selection` /
    /// `layers.off.needs_selection`).
    pub(super) fn selection_layers(
        drawing: &DrawingDatabase,
        document_id: DocumentId,
        refs: &[SelectionRef],
        code: &str,
        operation: &str,
        action: &str,
    ) -> CadResult<(Vec<LayerId>, Vec<Diagnostic>)> {
        let (selected, diagnostics) =
            Self::resolve_selection(drawing, document_id, refs, code, operation, action, true)?;
        let mut layers: Vec<LayerId> = Vec::new();
        for entity in &selected {
            if !layers.contains(&entity.layer) {
                layers.push(entity.layer);
            }
        }
        Ok((layers, diagnostics))
    }

    /// Hide every layer in `ids` as **one atomic session override batch**.
    ///
    /// A thin, typed wrapper over [`Application::set_layer_visibilities`] so both
    /// layer-view commands publish their hides the same way (one validated batch,
    /// no partial state, no database write, no revision bump, no history entry).
    pub(super) fn hide_layers_atomically(
        &self,
        session: &mut SessionState,
        document_id: DocumentId,
        ids: &[LayerId],
    ) -> CadResult<CommandOutcome> {
        let changes: Vec<(LayerId, bool)> = ids.iter().map(|id| (*id, false)).collect();
        self.set_layer_visibilities(session, document_id, &changes)
    }
}

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
        let (selected_layers, mut diagnostics) = Self::selection_layers(
            drawing,
            document_id,
            &refs,
            "isolate",
            "IsolateLayers",
            "isolate",
        )?;

        let current = session.active_layer;
        let mut keep: std::collections::BTreeSet<LayerId> = std::collections::BTreeSet::new();
        keep.insert(current);
        let mut current_layer_selected = false;
        for layer in &selected_layers {
            keep.insert(*layer);
            if *layer == current {
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
        let hide: Vec<LayerId> = drawing
            .layers()
            .filter(|layer| !keep.contains(&layer.id))
            .map(|layer| layer.id)
            .collect();
        if hide.is_empty() {
            diagnostics.push(Diagnostic {
                object: None,
                code: "layers.isolate.noop".into(),
                message: "LAYISO: every layer is already in the keep-set; no override written"
                    .into(),
            });
            let mut outcome = CommandOutcome::none();
            outcome.diagnostics = diagnostics;
            return Ok(outcome);
        }

        let mut outcome = self.hide_layers_atomically(session, document_id, &hide)?;
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

    /// `LAYOFF`: turn off every layer the selected entities live on, except the
    /// current layer.
    ///
    /// Allowed in Viewer and Work (a session/display operation), requires at least
    /// one selected entity, and records **no** revision bump, history entry or
    /// database write. The current layer is **never** turned off: this build
    /// refuses to turn the current layer off on the layer-table path
    /// (`layer.current_off`) and a hidden current layer would silently receive new
    /// geometry. A selection that lives *only* on the current layer therefore
    /// changes nothing and reports `layers.off.current_layer`.
    pub(crate) fn layoff_layers(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput("LAYOFF takes no payload".into()));
        }
        let refs = session.selection.refs().to_vec();
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "layers.off.needs_selection: select at least one object first".into(),
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let drawing = document.drawing.as_ref();
        let (selected_layers, mut diagnostics) =
            Self::selection_layers(drawing, document_id, &refs, "off", "LayOff", "turn off")?;

        let current = session.active_layer;
        let off: Vec<LayerId> = selected_layers
            .into_iter()
            .filter(|layer| *layer != current)
            .collect();
        if off.is_empty() {
            diagnostics.push(Diagnostic {
                object: None,
                code: "layers.off.current_layer".into(),
                message: format!(
                    "LAYOFF: the selection is only on layer {}, the current layer; a hidden \
                     current layer would receive new geometry the user cannot see, so nothing \
                     was turned off",
                    current.0
                ),
            });
            let mut outcome = CommandOutcome::none();
            outcome.diagnostics = diagnostics;
            return Ok(outcome);
        }

        let mut outcome = self.hide_layers_atomically(session, document_id, &off)?;
        diagnostics.push(Diagnostic {
            object: None,
            code: "layers.off".into(),
            message: format!(
                "LAYOFF: {} layer(s) turned off (session overrides only); layer {} is the \
                 current layer and stays on",
                off.len(),
                current.0
            ),
        });
        outcome.diagnostics = diagnostics;
        Ok(outcome)
    }
}
