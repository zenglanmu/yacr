//! Layer **selection** commands (AutoCAD 2026).
//!
//! These are the layer operations that act on the current selection rather than
//! on a layer name typed into the palette: `LAYCUR` in this commit, with the
//! rest of the run (`LAYM C U R`, `LAYFRZ`, `LAYTHW`, `LAYLCK`, `LAYULK`,
//! `SETBYLAYER`) added beside it.
//!
//! ## Declared divergence: pre-selection instead of "Select objects"
//!
//! AutoCAD prompts *Select objects:* after these commands start. This build's
//! fixed-step capture machine cannot ask that prompt, so every command here uses
//! the **current session selection** as its whole input
//! ([`crate::CommandPayload::None`]), exactly like `MATCHPROP`, `LAYISO` and the
//! Object mode of `REVCLOUD`. A bare invocation with nothing selected is an
//! explicit refusal (`<command>.selection`), never an empty success.
//!
//! ## What "selected object" means here
//!
//! Only directly addressable **model-space entity rows** are accepted. The
//! preflight refuses, by name ([`ensure_model_space_addressable`]):
//!
//! * a ref from another document (`<command>.not_addressable`);
//! * a block-instance member (`InstancePath`) or a sub-element
//!   (`<command>.not_addressable`, `Unsupported`: those have no entity identity
//!   of their own, exactly like the property-write path);
//! * an entity that does not resolve (`<command>.not_addressable`);
//! * an entity in paper space or a block definition
//!   (`<command>.not_addressable`, `Unsupported`: there is no model-space layer
//!   row to act on).
//!
//! ## Writes
//!
//! Entity-layer writes (`LAYCUR`) go through one
//! `begin_drawing_transaction` + `update_entity` per changed entity + one
//! `commit`, recorded as **one** undo step with `DrawingUndoRecord` entity
//! patches, exactly like `app_drawing::revision_cloud` / `properties`.
//!
//! Layer-**table** writes use
//! [`cad_db::DrawingDatabase::set_layer_attributes`]. Each changed row is one
//! database call (so the revision still moves once per changed layer), but the
//! whole command records **one** undo step through
//! [`Application::publish_layer_record`]: undo/redo restores every changed
//! row's exact before/after attribute set in one transaction.
//!
//! Every refusal happens before anything is published: the original drawing
//! keeps its revision, its layer rows and its history. A locked destination
//! layer is refused by the database's own entity-write rule (`layer.locked`);
//! there is no second lock check here.
//!
//! ## `LAYCUR`
//!
//! "Changes the layer property of selected objects to the current layer." Sets
//! every selected entity's `layer` to `SessionState::active_layer`. The active
//! layer must exist in the drawing (`laycur.active_layer_missing`). Entities
//! already on it are not staged; when the whole batch is already there the
//! command opens no transaction and reports `laycur.unchanged`. Success is
//! `laycur.applied` with the layer name and the entity count. Work-only.
//!
//! ## `LAYM C U R`
//!
//! "Sets the current layer to that of a selected object." Uses the **first**
//! selected entity's stored layer (`laymcur.first_pick` names the extra picks
//! when more than one object is selected) and resolves the row
//! (`laymcur.unknown_layer` when it is absent). It then reuses the one
//! current-layer rule, [`Application::ensure_layer_can_be_current`], so a
//! locked/frozen/off layer is refused with the existing
//! `layer.current_locked` / `layer.current_frozen` / `layer.current_off` codes
//! before the session is touched. Session-only: no database write, no revision
//! bump, no history. Success is `laymcur.applied` with the layer name. Work-only.
//!
//! ## `LAYFRZ`
//!
//! "Freezes the layer of selected objects." Applies to **every distinct layer**
//! in the selection. The current layer can never be frozen, so when the
//! selection contains it the whole command is refused (`layfrz.current_layer`)
//! rather than partially applied. v1 handles **Entity** selection only: there
//! are no Settings / Viewports / Block options, and it freezes the layer in
//! **all viewports** (this build has no per-viewport freeze model); the
//! `frozen_in_new_viewport` flag and every other attribute are left untouched.
//! One `set_layer_attributes` call per changed layer means one revision bump per
//! layer; the command still records **one** undo step. Success is `layfrz.applied`
//! with the changed-layer count; an all-frozen selection is `layfrz.unchanged`.
//! Work-only.
//!
//! ## `LAYTHW`
//!
//! "Thaws all layers in the drawing." Takes **no selection** (a payload is
//! refused with `laythw.payload`) and thaws in two places at once, like AutoCAD:
//! every drawing-level `frozen` flag, and every per-viewport frozen override
//! ([`cad_db::PaperViewport::layer_overrides`] — the state `LAYVPI`/`VPLAYER`
//! write) in **every** layout. Only frozen state is cleared: viewport geometry,
//! views and `frozen_in_new_viewport` are untouched, and an all-thawed drawing is
//! `laythw.unchanged` with no write. Layer and layout rows are staged into one
//! `DrawingUndoRecord`, so one undo restores every flag and override exactly
//! (one revision per changed row). Success is `laythw.applied` with the
//! changed-layer, cleared-override and changed-layout counts. Work-only.
//!
//! ## `LAYLCK` / `LAYULK`
//!
//! "Locks/unlocks the layer of a selected object." Applies to every distinct
//! layer in the selection, **including the current layer** (locking is a write
//! permission, not visibility; creating on a locked current layer is then
//! refused by `cad-db`'s own `layer.locked` rule). Only `locked` changes; every
//! other attribute is preserved. Already-locked (or already-unlocked) rows are
//! not staged, and an all-unchanged selection reports `laylck.unchanged` /
//! `layulk.unchanged` without a write. Unknown layer rows are refused by name.
//! One revision bump per changed layer, but exactly **one** undo step for the
//! command. Success is `laylck.applied` / `layulk.applied` with the changed-layer
//! count. Work-only.
//!
//! ## `SETBYLAYER`
//!
//! Resets the four modelled display overrides — **colour, linetype, lineweight
//! and transparency** — to symbolic `ByLayer` through `update_entity_display`
//! in one transaction / one undo step. Materials, text/dimension/hatch/polyline
//! properties are not modelled and are named by
//! `setbylayer.properties_not_reset`.
//!
//! The typed [`SetByLayerSettings::include_blocks`] option (AutoCAD's
//! **Include blocks**) recurses into selected `INSERT`s and rewrites block
//! members' display attributes through the same transaction; only display state
//! changes, so no block row/member list or block write path is touched.
//! `linetype_scale`, layer, space and geometry are preserved. Any target on a
//! locked layer — selected or block member — refuses the whole command
//! (`setbylayer.locked`), because the display path bypasses the lock. Work-only.

mod current_layer;
mod setbylayer;

use super::*;

use super::layer_record::layer_row;
use cad_db::ChangeScope;

/// A named, non-capability refusal (`<command>.<code>: <detail>`).
pub(super) fn invalid(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{prefix}.{code}: {detail}"))
}

/// A named capability-gap refusal (`<command>.<code>: <detail>`).
pub(super) fn unsupported(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{prefix}.{code}: {detail}"))
}

/// These commands read the selection, so a payload is refused rather than
/// silently ignored.
pub(super) fn ensure_no_payload(command: &Command, prefix: &str, label: &str) -> CadResult<()> {
    if matches!(command.payload, CommandPayload::None) {
        Ok(())
    } else {
        Err(invalid(
            prefix,
            "payload",
            format!("{label} takes no payload; the current selection is the input"),
        ))
    }
}

/// Refuse a selected ref that is not a directly addressable model-space entity.
pub(super) fn ensure_model_space_addressable(
    drawing: &DrawingDatabase,
    command: &Command,
    reference: &SelectionRef,
    prefix: &str,
) -> CadResult<()> {
    if reference.document != command.document {
        return Err(invalid(
            prefix,
            "not_addressable",
            format!(
                "the selected ref for entity {} belongs to another document",
                reference.entity.0
            ),
        ));
    }
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(unsupported(
            prefix,
            "not_addressable",
            format!(
                "entity {} is referenced through a block instance or a sub-element and has no \
                 directly addressable identity",
                reference.entity.0
            ),
        ));
    }
    let Some(entity) = drawing.entity(reference.entity) else {
        return Err(invalid(
            prefix,
            "not_addressable",
            format!(
                "entity {} does not exist in this drawing",
                reference.entity.0
            ),
        ));
    };
    if entity.space != SpaceId::Model {
        return Err(unsupported(
            prefix,
            "not_addressable",
            format!(
                "entity {} is not in model space; only model-space entities can be edited by this \
                 command",
                reference.entity.0
            ),
        ));
    }
    Ok(())
}

/// Take the selection snapshot and refuse an empty one.
pub(super) fn selection_snapshot(
    session: &SessionState,
    prefix: &str,
    label: &str,
) -> CadResult<Vec<SelectionRef>> {
    let references = session.selection.refs().to_vec();
    if references.is_empty() {
        return Err(invalid(
            prefix,
            "selection",
            format!("{label} needs at least one selected model-space object"),
        ));
    }
    Ok(references)
}

/// Preflight the selection and collect each distinct entity layer (first-seen
/// order, so the diagnostic and the write order follow the user's pick order).
pub(super) fn selected_distinct_layers(
    drawing: &DrawingDatabase,
    command: &Command,
    references: &[SelectionRef],
    prefix: &str,
) -> CadResult<Vec<LayerId>> {
    let mut layers = Vec::new();
    for reference in references {
        ensure_model_space_addressable(drawing, command, reference, prefix)?;
        let layer = drawing
            .entity(reference.entity)
            .expect("preflight verified the entity resolves")
            .layer;
        if !layers.contains(&layer) {
            layers.push(layer);
        }
    }
    Ok(layers)
}

impl Application {
    /// Dispatch a layer **selection** command.
    pub(crate) fn layer_selection_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.id {
            CommandId::LayCur => self.laycur(session, command),
            CommandId::LayMCur => self.laymcur(session, command),
            CommandId::LayFrz => self.layfrz(session, command),
            CommandId::LayThw => self.laythw(session, command),
            CommandId::LayLck => self.laylck(session, command),
            CommandId::LayUlk => self.layulk(session, command),
            CommandId::SetByLayer => self.setbylayer(session, command),
            _ => Err(CadError::InvalidInput(
                "not a layer selection command".to_string(),
            )),
        }
    }

    /// `LAYFRZ`: freeze every distinct layer in the selection except the current
    /// one. Freezing the current layer is never a partial operation: the whole
    /// command is refused (`layfrz.current_layer`).
    fn layfrz(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, "layfrz", "LAYFRZ")?;
        let references = selection_snapshot(session, "layfrz", "LAYFRZ")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        let layers = selected_distinct_layers(drawing.as_ref(), command, &references, "layfrz")?;

        // AutoCAD cannot freeze the current layer. This build refuses the whole
        // command rather than freezing the other layers and skipping it.
        if layers.contains(&session.active_layer) {
            let name = drawing
                .layer(session.active_layer)
                .map(|row| row.name.as_str())
                .unwrap_or("<unknown>");
            return Err(invalid(
                "layfrz",
                "current_layer",
                format!(
                    "layer '{name}' is the current layer and cannot be frozen; switch the \
                     current layer first"
                ),
            ));
        }
        for layer in &layers {
            if drawing.layer(*layer).is_none() {
                return Err(invalid(
                    "layfrz",
                    "unknown_layer",
                    format!("layer {} does not exist in this drawing", layer.0),
                ));
            }
        }

        let (changes, count) =
            self.write_layer_flags(document_id, &layers, "layfrz", |attributes| {
                if attributes.frozen {
                    false
                } else {
                    // `frozen_in_new_viewport` is a separate flag; it stays.
                    attributes.frozen = true;
                    true
                }
            })?;
        if count == 0 {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "layfrz.unchanged".into(),
                    message: "LAYFRZ: every selected layer is already frozen".into(),
                }],
                measurement: None,
                point_report: None,
            });
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "layfrz.applied".into(),
                message: format!(
                    "LAYFRZ: froze {count} layer(s) in all viewports (one transaction / one \
                     undo step; Settings/Viewports/Block options are not implemented)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `LAYTHW`: thaw every frozen layer, in the layer table **and** in every
    /// viewport of every layout.
    fn laythw(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            let message =
                "LAYTHW takes no payload; it thaws every frozen layer and reads no selection";
            return Err(invalid("laythw", "payload", message));
        }
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let layers: Vec<LayerId> = drawing.layers().map(|layer| layer.id).collect();
        // AutoCAD thaws every layer in *all* viewports, so each layout's frozen
        // viewport overrides are cleared with the layer table. Only `true`
        // (frozen) entries go: an explicit `false` override is not frozen.
        let mut overrides = 0;
        let thawed: Vec<(LayoutId, Vec<cad_db::PaperViewport>)> = drawing
            .layouts()
            .filter_map(|layout| {
                let mut viewports = layout.viewports.clone();
                let mut changed = false;
                for viewport in &mut viewports {
                    let known = viewport.layer_overrides.len();
                    viewport.layer_overrides.retain(|_, frozen| !*frozen);
                    overrides += known - viewport.layer_overrides.len();
                    changed |= viewport.layer_overrides.len() != known;
                }
                changed.then_some((layout.id, viewports))
            })
            .collect();
        let mut candidate = drawing.clone();
        let transaction = next_transaction_id();
        let mut layer_patches = Vec::new();
        // The published change set spans the layer **and** layout writes; its
        // dirty scope must union both (neither is an entity change).
        let mut scope = ChangeScope::default();
        for id in &layers {
            let before_row = layer_row(&drawing, *id);
            let mut attributes = candidate.layer_attributes(*id);
            if !attributes.frozen {
                continue;
            }
            attributes.frozen = false;
            // `frozen_in_new_viewport` is not a viewport model; it stays.
            let write = candidate.set_layer_attributes(*id, attributes, transaction, "laythw")?;
            scope.merge(&write.scope);
            if let Some(before_row) = before_row {
                let after = layer_row(&candidate, *id);
                layer_patches.push(cad_history::layer_patch(*id, Some(before_row), after));
            }
        }
        let mut layout_patches = Vec::with_capacity(thawed.len());
        for (layout, viewports) in thawed {
            let before_row = layout_row(&drawing, layout);
            let write = candidate.set_layout_viewports(layout, viewports, transaction, "laythw")?;
            scope.merge(&write.scope);
            let after = layout_row(&candidate, layout);
            layout_patches.push(cad_history::layout_patch(layout, before_row, after));
        }
        if layer_patches.is_empty() && layout_patches.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "laythw.unchanged".into(),
                    message: "LAYTHW: nothing is frozen in the layer table or a viewport".into(),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let (changed_layers, changed_layouts) = (layer_patches.len(), layout_patches.len());
        let changes = ChangeSet {
            database: candidate.id(),
            before: drawing.revision(),
            after: candidate.revision(),
            transaction,
            reason: "laythw".to_string(),
            changes: Vec::new(),
            scope,
        };
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction,
                label: "laythw".to_string(),
                patches: Vec::new(),
                block_patches: Vec::new(),
                layer_patches,
                layout_patches,
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(changes)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "laythw.applied".into(),
                message: format!(
                    "LAYTHW: thawed {changed_layers} layer(s) at drawing level and cleared \
                     {overrides} frozen viewport override(s) in {changed_layouts} layout(s): no \
                     layer stays frozen in any viewport (one transaction / one undo step; \
                     `frozen_in_new_viewport` is left untouched)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `LAYLCK` / `LAYULK`: set the `locked` flag on every distinct selected
    /// layer (the current layer included).
    fn set_selected_layers_locked(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        prefix: &str,
        label: &str,
        locked: bool,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, prefix, label)?;
        let references = selection_snapshot(session, prefix, label)?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        let layers = selected_distinct_layers(drawing.as_ref(), command, &references, prefix)?;
        for layer in &layers {
            if drawing.layer(*layer).is_none() {
                return Err(invalid(
                    prefix,
                    "unknown_layer",
                    format!("layer {} does not exist in this drawing", layer.0),
                ));
            }
        }

        let (changes, count) =
            self.write_layer_flags(document_id, &layers, prefix, |attributes| {
                if attributes.locked == locked {
                    false
                } else {
                    attributes.locked = locked;
                    true
                }
            })?;
        if count == 0 {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: format!("{prefix}.unchanged"),
                    message: format!(
                        "{label}: every selected layer is already {}",
                        if locked { "locked" } else { "unlocked" }
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let action = if locked { "locked" } else { "unlocked" };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes,
            diagnostics: vec![Diagnostic {
                object: None,
                code: format!("{prefix}.applied"),
                message: format!(
                    "{label}: {action} {count} layer(s) (one transaction / one undo step)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `LAYLCK`: lock every distinct selected layer.
    fn laylck(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.set_selected_layers_locked(session, command, "laylck", "LAYLCK", true)
    }

    /// `LAYULK`: unlock every distinct selected layer.
    fn layulk(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.set_selected_layers_locked(session, command, "layulk", "LAYULK", false)
    }
}
