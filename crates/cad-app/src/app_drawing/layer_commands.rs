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
//! [`cad_db::DrawingDatabase::set_layer_attributes`]. They raise the revision
//! but are **not undoable** (cad-history has no layer patch kind); the success
//! diagnostic says so, and no undo entry is recorded.
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
//! layer; the write is **not undoable**. Success is `layfrz.applied` with the
//! changed-layer count; an all-frozen selection is `layfrz.unchanged`. Work-only.
//!
//! ## `LAYTHW`
//!
//! "Thaws all layers." Takes **no selection** (a payload is refused with
//! `laythw.payload`); every layer whose `frozen` flag is set is thawed in one
//! layer-table pass, and an all-thawed drawing is `laythw.unchanged` with no
//! write. This build has no viewport model, so **viewport-level frozen layers
//! are not thawed** and `frozen_in_new_viewport` is left untouched; the same
//! one-revision-bump-per-changed-layer and not-undoable rules apply. Success is
//! `laythw.applied` with the changed-layer count. Work-only.
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
//! One revision bump per changed layer; **not undoable**. Success is
//! `laylck.applied` / `layulk.applied` with the changed-layer count. Work-only.
//!
//! ## `SETBYLAYER`
//!
//! "Changes the property overrides of selected objects to ByLayer." Resets
//! exactly **colour, linetype and lineweight** to their symbolic `ByLayer`
//! forms through `update_entity_display` in one transaction / one undo step
//! (display writes are undoable through display-state patches). Transparency is
//! deliberately **not** reset in this build, and materials are not modelled;
//! text style/height, dimension style, hatch colour/pattern and polyline
//! thickness are not modelled either, so `setbylayer.properties_not_reset`
//! names them (including transparency and materials) in every run. AutoCAD's
//! **Include blocks** option is **NOT IMPLEMENTED**. `linetype_scale`, layer,
//! space, geometry and every other field are preserved. If any selected entity
//! is on a locked layer the whole command is refused (`setbylayer.locked`):
//! AutoCAD does not modify objects on locked layers, and the display-write path
//! would otherwise bypass the lock. An all-ByLayer selection opens no
//! transaction and reports `setbylayer.unchanged`; success is
//! `setbylayer.applied` with the entity count. Work-only.

use super::*;
use cad_db::{
    EntityColor, EntityLineType, EntityLineWeight, EntityRenderAttributes, LayerAttributes,
};

/// A named, non-capability refusal (`<command>.<code>: <detail>`).
fn invalid(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{prefix}.{code}: {detail}"))
}

/// A named capability-gap refusal (`<command>.<code>: <detail>`).
fn unsupported(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{prefix}.{code}: {detail}"))
}

/// These commands read the selection, so a payload is refused rather than
/// silently ignored.
fn ensure_no_payload(command: &Command, prefix: &str, label: &str) -> CadResult<()> {
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
fn ensure_model_space_addressable(
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
fn selection_snapshot(
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
fn selected_distinct_layers(
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

/// The `SETBYLAYER` informational diagnostic: what AutoCAD's command also resets
/// but this build deliberately does not.
fn properties_not_reset() -> Diagnostic {
    Diagnostic {
        object: None,
        code: "setbylayer.properties_not_reset".into(),
        message: "SETBYLAYER: transparency and materials are not reset, and text style/height, \
                  dimension style, hatch colour/pattern and polyline thickness are not modelled; \
                  the Include blocks option is not implemented"
            .into(),
    }
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

    /// Publish an already-validated layer-table candidate. **No history**: layer
    /// edits are not undoable (cad-history has no layer patch kind).
    fn publish_layer_table(
        &mut self,
        document_id: DocumentId,
        candidate: DrawingDatabase,
    ) -> CadResult<()> {
        self.workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::Invariant("document disappeared while publishing".into()))?
            .drawing = Arc::new(candidate);
        Ok(())
    }

    /// Apply one boolean layer-attribute change to every layer in `layers`.
    ///
    /// Each changed row is one [`DrawingDatabase::set_layer_attributes`] call, so
    /// a multi-layer command raises the revision once per changed layer (a
    /// documented limitation). `mutate` returns whether the row actually changed;
    /// unchanged rows are not staged, so the revision does not move for them.
    /// Nothing is published when no row changes. A successful write reports one
    /// combined revision span and the changed-row count.
    fn write_layer_flags(
        &mut self,
        document_id: DocumentId,
        layers: &[LayerId],
        reason: &str,
        mutate: impl Fn(&mut LayerAttributes) -> bool,
    ) -> CadResult<(Option<Arc<ChangeSet>>, usize)> {
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        let mut candidate = drawing.as_ref().clone();
        let before = candidate.revision();
        let transaction = next_transaction_id();
        let mut changed = 0;
        for id in layers {
            let mut attributes = candidate.layer_attributes(*id);
            if mutate(&mut attributes) {
                candidate.set_layer_attributes(*id, attributes, transaction, reason)?;
                changed += 1;
            }
        }
        if changed == 0 {
            return Ok((None, 0));
        }
        let changeset = ChangeSet {
            database: candidate.id(),
            before,
            after: candidate.revision(),
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
        };
        self.publish_layer_table(document_id, candidate)?;
        Ok((Some(Arc::new(changeset)), changed))
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
                    // v1 freezes the layer in every viewport; the
                    // `frozen_in_new_viewport` flag (the per-viewport model this
                    // build does not have) is deliberately left untouched.
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
                    "LAYFRZ: froze {count} layer(s) in all viewports (layer-table edit; not \
                     undoable; Settings/Viewports/Block options are not implemented)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `LAYTHW`: thaw every frozen layer.
    fn laythw(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "laythw",
                "payload",
                "LAYTHW takes no payload; it thaws every frozen layer and reads no selection",
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        let layers: Vec<LayerId> = drawing.layers().map(|layer| layer.id).collect();

        let (changes, count) =
            self.write_layer_flags(document_id, &layers, "laythw", |attributes| {
                if attributes.frozen {
                    attributes.frozen = false;
                    // `frozen_in_new_viewport` is the per-viewport flag this
                    // build has no model for; thawing the layer leaves it alone.
                    true
                } else {
                    false
                }
            })?;
        if count == 0 {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "laythw.unchanged".into(),
                    message: "LAYTHW: no layer is frozen; nothing changed".into(),
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
                code: "laythw.applied".into(),
                message: format!(
                    "LAYTHW: thawed {count} layer(s) (viewport-level frozen layers are not \
                     thawed; layer-table edit, not undoable, one revision bump per changed layer)"
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
                    "{label}: {action} {count} layer(s) (layer-table edit; not undoable; one \
                     revision bump per changed layer)"
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

    /// `SETBYLAYER`: reset the three modelled display overrides to ByLayer.
    fn setbylayer(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, "setbylayer", "SETBYLAYER")?;
        let references = selection_snapshot(session, "setbylayer", "SETBYLAYER")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        for reference in &references {
            ensure_model_space_addressable(drawing.as_ref(), command, reference, "setbylayer")?;
        }

        // AutoCAD does not modify objects on locked layers. The display-write
        // path deliberately bypasses the lock (it is a representation rebuild),
        // so this command owns the refusal for the whole batch.
        for reference in &references {
            let entity = drawing
                .entity(reference.entity)
                .expect("preflight verified the entity resolves");
            if drawing.layer_locked(entity.layer) {
                let name = drawing
                    .layer(entity.layer)
                    .map(|row| row.name.as_str())
                    .unwrap_or("<unknown>");
                return Err(invalid(
                    "setbylayer",
                    "locked",
                    format!(
                        "entity {} is on locked layer '{name}'; AutoCAD does not modify objects \
                         on locked layers, so the whole command is refused",
                        entity.id.0
                    ),
                ));
            }
        }

        // Only the three modelled overrides change. Transparency, materials,
        // text/dimension/hatch/polyline properties are outside this reset (see
        // `properties_not_reset`), and `linetype_scale` is preserved because the
        // symbolic `ByLayer` variant carries no scale of its own.
        let mut changes: Vec<(EntityId, DbEntity, EntityRenderAttributes)> = Vec::new();
        for reference in &references {
            let before = drawing
                .entity(reference.entity)
                .expect("preflight verified the entity resolves")
                .clone();
            let mut attributes = drawing.entity_render_attributes(before.id);
            if attributes.color == EntityColor::ByLayer
                && attributes.linetype == EntityLineType::ByLayer
                && attributes.lineweight == EntityLineWeight::ByLayer
            {
                continue;
            }
            attributes.color = EntityColor::ByLayer;
            attributes.linetype = EntityLineType::ByLayer;
            attributes.lineweight = EntityLineWeight::ByLayer;
            changes.push((before.id, before, attributes));
        }

        if changes.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![
                    Diagnostic {
                        object: None,
                        code: "setbylayer.unchanged".into(),
                        message: "SETBYLAYER: colour, linetype and lineweight are already ByLayer \
                                  on every selected entity"
                            .into(),
                    },
                    properties_not_reset(),
                ],
                measurement: None,
                point_report: None,
            });
        }

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("setbylayer", transaction)?;
            for (id, _, attributes) in &changes {
                staged.update_entity_display(*id, attributes.clone())?;
            }
            staged.commit()?
        };

        let mut patches = Vec::with_capacity(changes.len());
        let mut objects = Vec::with_capacity(changes.len());
        for (id, before, _) in &changes {
            patches.push(DrawingPatch {
                id: *id,
                before: Some(before.clone()),
                after: Some(before.clone()),
                before_state: Some(drawing.entity_display_state(*id)),
                after_state: Some(candidate.entity_display_state(*id)),
            });
            objects.push(ObjectId(id.0));
        }
        let count = changes.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "setbylayer".to_string(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "setbylayer.applied".into(),
                    message: format!(
                        "SETBYLAYER: reset colour, linetype and lineweight to ByLayer on {count} \
                         entities (one transaction / one undo step)"
                    ),
                },
                properties_not_reset(),
            ],
            measurement: None,
            point_report: None,
        })
    }

    /// `LAYM C U R`: make the first selected entity's layer the current layer.
    fn laymcur(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, "laymcur", "LAYMCUR")?;
        let references = selection_snapshot(session, "laymcur", "LAYMCUR")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        for reference in &references {
            ensure_model_space_addressable(drawing.as_ref(), command, reference, "laymcur")?;
        }

        let first = &references[0];
        let layer = drawing
            .entity(first.entity)
            .expect("preflight verified the entity resolves")
            .layer;
        let layer_name = drawing
            .layer(layer)
            .map(|row| row.name.clone())
            .ok_or_else(|| {
                invalid(
                    "laymcur",
                    "unknown_layer",
                    format!(
                        "the first selected entity's layer ({}) does not exist in this drawing",
                        layer.0
                    ),
                )
            })?;

        // The one current-layer rule: locked / frozen / off are refused with the
        // existing `layer.current_*` codes before the session is touched.
        self.ensure_layer_can_be_current(document_id, layer)?;
        session.active_layer = layer;

        let mut diagnostics = vec![Diagnostic {
            object: None,
            code: "laymcur.applied".into(),
            message: format!("LAYMCUR: the current layer is now '{layer_name}'"),
        }];
        if references.len() > 1 {
            diagnostics.push(Diagnostic {
                object: None,
                code: "laymcur.first_pick".into(),
                message: format!(
                    "LAYMCUR: {} objects were selected; the current layer follows the first \
                     selected object",
                    references.len()
                ),
            });
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `LAYCUR`: move every selected entity to the session's current layer.
    fn laycur(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, "laycur", "LAYCUR")?;
        let references = selection_snapshot(session, "laycur", "LAYCUR")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        let active_layer = session.active_layer;
        let layer_name = drawing
            .layer(active_layer)
            .map(|row| row.name.clone())
            .ok_or_else(|| {
                invalid(
                    "laycur",
                    "active_layer_missing",
                    format!(
                        "the session's current layer ({}) does not exist in this drawing",
                        active_layer.0
                    ),
                )
            })?;

        for reference in &references {
            ensure_model_space_addressable(drawing.as_ref(), command, reference, "laycur")?;
        }

        let mut changed_before: Vec<DbEntity> = Vec::new();
        for reference in &references {
            let entity = drawing
                .entity(reference.entity)
                .expect("preflight verified the entity resolves");
            if entity.layer != active_layer {
                changed_before.push(entity.clone());
            }
        }

        if changed_before.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "laycur.unchanged".into(),
                    message: format!(
                        "LAYCUR: every selected entity is already on layer '{layer_name}'"
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("laycur", transaction)?;
            for before in &changed_before {
                let mut after = before.clone();
                after.layer = active_layer;
                staged.update_entity(after)?;
            }
            staged.commit()?
        };

        let mut patches = Vec::with_capacity(changed_before.len());
        let mut objects = Vec::with_capacity(changed_before.len());
        for before in changed_before {
            let id = before.id;
            let after = candidate
                .entity(id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("LAYCUR: edited entity vanished".into()))?;
            patches.push(drawing_patch(id, Some(before), Some(after)));
            objects.push(ObjectId(id.0));
        }
        let count = patches.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "laycur".to_string(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "laycur.applied".into(),
                message: format!(
                    "LAYCUR: moved {count} entities to the current layer '{layer_name}'"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
