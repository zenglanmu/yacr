//! `SETENTITYPROPERTY`: write one property value to every selected entity.
//!
//! This is the core/domain side of the properties panel's edit path. It is
//! deliberately **not** a second creation path: it re-reads each selected entity
//! inside the command, changes exactly one stored value, and stages **one**
//! transaction / **one** undo step for the whole selection.
//!
//! ## All-or-nothing
//!
//! Every refusal happens before the transaction is opened, so a bad value or a
//! selection containing one non-addressable ref writes nothing:
//!
//! * `property.unsupported_key` — `text` (it has its own `TEXTEDIT` path) or any
//!   key outside [`crate::property_value::SUPPORTED_KEYS`];
//! * `property.invalid_value` / `property.unknown_layer` /
//!   `property.unknown_linetype` / `property.ambiguous_name` — the grammar in
//!   [`crate::property_value`] rejected the value;
//! * `property.selection_empty` — nothing is selected;
//! * `property.not_addressable` — a selected ref carries an instance path (a
//!   block-instance member) or a sub-element; those have no entity identity of
//!   their own, exactly like `TEXTEDIT` and [`crate::property_row_editable`];
//! * `property.target_missing` — a ref does not resolve in this document.
//!
//! ## Both state layers, one transaction
//!
//! `layer` lives on the entity row; colour, linetype, lineweight and
//! transparency live beside it in the entity's display state (the values the
//! representation/renderer reads). The entity row is staged through
//! [`cad_db::DrawingTransaction::update_entity`]; the display attributes through
//! [`cad_db::DrawingTransaction::update_entity_display`], which replays them
//! after the validated entity write and marks the update with
//! [`cad_db::ChangeMask::STYLE`] so dependency invalidation rebuilds the
//! representation (the same mechanism every other style edit uses).
//!
//! ## Undo granularity and no-ops
//!
//! * The record carries one patch per **changed** entity, all under one
//!   transaction. Entities that already hold the requested value are not staged
//!   and get no patch, so undoing restores exactly what changed.
//! * Each patch captures the full before/after entity **and** display state, so
//!   undo restores both layers exactly (the same pattern the delete/erase paths
//!   use for their display state).
//! * If **no** entity changes, the command opens no transaction, publishes no
//!   ChangeSet, records no undo entry and reports `property.unchanged`.

use super::*;
use crate::property_value::{self, LinetypeTarget, PropertyValue};
use cad_db::{DbEntity, DrawingDatabase, EntityLineType, EntityRenderAttributes};

impl Application {
    /// Write one property value to every currently selected entity.
    ///
    /// Payload is `EntityProperty { key, value }`. The selection is read from the
    /// session at apply time (no ref travels in the payload), so a stale UI
    /// cannot retarget a different object.
    pub(crate) fn set_entity_property(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::EntityProperty { key, value } = &command.payload else {
            return Err(CadError::InvalidInput(
                "property.invalid_payload: SETENTITYPROPERTY needs an \
                 EntityProperty { key, value } payload"
                    .into(),
            ));
        };

        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        // Resolve the value once, against the drawing's own tables. Unsupported
        // keys (`text` included) are refused here.
        let target = property_value::parse(&drawing, key, value)?;

        let references = session.selection.refs();
        if references.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "property.selection_empty: select at least one entity to change {key}"
            )));
        }

        // Preflight every ref before staging anything: one non-addressable ref
        // refuses the whole command (no partial write).
        for reference in references {
            ensure_addressable(drawing.as_ref(), command, reference)?;
        }

        let mut staged_changes: Vec<StagedChange> = Vec::new();
        for reference in references {
            let before = drawing
                .entity(reference.entity)
                .expect("preflight verified the entity resolves")
                .clone();
            if let Some(change) = plan_entity(&drawing, before, &[&target]) {
                staged_changes.push(change);
            }
        }

        if staged_changes.is_empty() {
            // Nothing to write: no transaction, no ChangeSet, no history entry.
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "property.unchanged".into(),
                    message: format!(
                        "SETENTITYPROPERTY: {key} already has the requested value on every \
                         selected entity"
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged =
                candidate.begin_drawing_transaction("set entity property", transaction)?;
            for change in &staged_changes {
                if change.after != change.before {
                    staged.update_entity(change.after.clone())?;
                }
                if let Some(attributes) = &change.after_attributes {
                    staged.update_entity_display(change.id, attributes.clone())?;
                }
            }
            staged.commit()?
        };

        // Read the committed state back so patches carry the exact stored value,
        // never a re-derivation.
        for change in &mut staged_changes {
            change.after = candidate
                .entity(change.id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("edited entity vanished after commit".into()))?;
            if change.after_attributes.is_some() {
                change.after_attributes = Some(candidate.entity_render_attributes(change.id));
            }
        }

        let patches: Vec<DrawingPatch> = staged_changes
            .iter()
            .map(|change| DrawingPatch {
                id: change.id,
                before: Some(change.before.clone()),
                after: Some(change.after.clone()),
                before_state: Some(drawing.entity_display_state(change.id)),
                after_state: Some(candidate.entity_display_state(change.id)),
            })
            .collect();
        let objects: Vec<ObjectId> = staged_changes
            .iter()
            .map(|change| ObjectId(change.id.0))
            .collect();
        let count = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: format!("set {key}"),
                patches,
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.property.set".into(),
                message: format!("SETENTITYPROPERTY: set {key} on {count} entities"),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// One entity that will change, with both state layers captured.
///
/// Shared with [`super::match_prop`], which applies five resolved values per
/// entity through the same [`plan_entity`] comparison rules (the no-op rule for
/// transparency and the linetype-scale preservation live here only).
pub(super) struct StagedChange {
    pub(super) id: EntityId,
    pub(super) before: DbEntity,
    pub(super) after: DbEntity,
    /// `Some` **iff** a display attribute changes (and therefore the display
    /// state must be replayed).
    pub(super) after_attributes: Option<EntityRenderAttributes>,
}

/// Refuse a ref that has no directly addressable entity identity.
pub(super) fn ensure_addressable(
    drawing: &DrawingDatabase,
    command: &Command,
    reference: &SelectionRef,
) -> CadResult<()> {
    if reference.document != command.document {
        return Err(CadError::InvalidInput(format!(
            "property.target_missing: selection ref for entity {} belongs to another document",
            reference.entity.0
        )));
    }
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(CadError::Unsupported(
            "property.not_addressable: block-instance members and sub-elements have no \
             directly addressable entity identity"
                .into(),
        ));
    }
    if drawing.entity(reference.entity).is_none() {
        return Err(CadError::InvalidInput(format!(
            "property.target_missing: entity {} does not exist in this drawing",
            reference.entity.0
        )));
    }
    Ok(())
}

/// The planned change for one entity, or `None` when it already holds every
/// value in `targets`.
///
/// This is the single applicator of a resolved [`PropertyValue`] list: the
/// `SETENTITYPROPERTY` path passes one target and `MATCHPROP` passes the five a
/// source entity carries, so the value-independent rules — the transparency
/// percent no-op comparison, the destination's linetype scale preservation and
/// the "both state layers or nothing" rule — exist once.
pub(super) fn plan_entity(
    drawing: &DrawingDatabase,
    before: DbEntity,
    targets: &[&PropertyValue],
) -> Option<StagedChange> {
    let mut after = before.clone();
    let mut attributes = drawing.entity_render_attributes(before.id);
    let before_attributes = attributes.clone();
    let mut entity_changed = false;
    let mut attributes_changed = false;

    for target in targets {
        match target {
            PropertyValue::Layer(layer) => {
                if after.layer != *layer {
                    after.layer = *layer;
                    entity_changed = true;
                }
            }
            PropertyValue::Color(color) => {
                if attributes.color != *color {
                    attributes.color = *color;
                    attributes_changed = true;
                }
            }
            PropertyValue::Lineweight(lineweight) => {
                if attributes.lineweight != *lineweight {
                    attributes.lineweight = *lineweight;
                    attributes_changed = true;
                }
            }
            PropertyValue::Transparency(transparency) => {
                // Compare on the wire grammar (integer percent), not on the stored
                // float: the importer quantised the source byte into an opacity, so
                // the stored value may not equal the exact percent-derived float even
                // though it displays the same percentage. A panel that reads "55" and
                // writes "55" must not create an undo entry.
                if property_value::transparency_value(attributes.transparency)
                    != property_value::transparency_value(*transparency)
                {
                    attributes.transparency = *transparency;
                    attributes_changed = true;
                }
            }
            PropertyValue::Linetype(target) => {
                let next = resolve_linetype(&attributes.linetype, target);
                if attributes.linetype != next {
                    attributes.linetype = next;
                    attributes_changed = true;
                }
            }
        }
    }

    if !entity_changed && !attributes_changed {
        return None;
    }
    debug_assert_eq!(
        attributes_changed,
        attributes != before_attributes,
        "the changed flag must describe the display state exactly"
    );
    Some(StagedChange {
        id: before.id,
        before,
        after,
        after_attributes: attributes_changed.then_some(attributes),
    })
}

/// Build the linetype value for one entity: a named table entry keeps the
/// entity's own linetype scale (an entity value the table does not carry);
/// a symbolic target replaces the whole value.
fn resolve_linetype(current: &EntityLineType, target: &LinetypeTarget) -> EntityLineType {
    match target {
        LinetypeTarget::ByLayer => EntityLineType::ByLayer,
        LinetypeTarget::ByBlock => EntityLineType::ByBlock,
        LinetypeTarget::Named { name, pattern } => {
            let scale = match current {
                EntityLineType::Explicit { scale, .. } => *scale,
                EntityLineType::ByLayer | EntityLineType::ByBlock => 1.0,
            };
            EntityLineType::Explicit {
                name: name.clone(),
                pattern: pattern.clone(),
                scale,
            }
        }
    }
}
