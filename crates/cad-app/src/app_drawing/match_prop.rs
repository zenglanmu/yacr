//! `MATCHPROP`: paint the **first selected** entity's display properties onto
//! every other selected entity (AutoCAD's property painter, selection form).
//!
//! ## Selection order is the input
//!
//! The payload is [`CommandPayload::None`]: the selection **is** the input, read
//! from the session at apply time (a stale UI cannot retarget a different
//! object). The entity order is the one [`SelectionSet`] already preserves — the
//! order the user picked, first-seen first. The **first** selected entity is the
//! source; every other selected entity is a destination. The order is never
//! re-sorted by id/handle, so the same pick sequence always yields the same
//! source. Fewer than two selected entities is an explicit refusal
//! (`matchprop.needs_two`): there is no source-without-destination mode.
//!
//! ## Exactly the five writable display properties
//!
//! The transferred set is [`crate::property_value::SUPPORTED_KEYS`] — `layer`,
//! `color`, `linetype`, `lineweight`, `transparency` — i.e. exactly what the
//! property-write path (`SETENTITYPROPERTY`) can change. There is **no second
//! grammar**: each source value is projected with
//! [`crate::property_value::property_value`] (the canonical, round-trippable
//! string the panel shows) and re-parsed with [`crate::property_value::parse`]
//! against the same drawing tables, so the panel, `SETENTITYPROPERTY` and
//! MATCHPROP accept and refuse the same values.
//!
//! **Not transferred** (and never silently approximated):
//!
//! * the per-property Settings subset (MATCHPROP's *Settings* dialog, which
//!   chooses *which* properties to copy) — the whole five-property set is always
//!   painted;
//! * non-display properties: text style and text height, dimension style, hatch
//!   pattern (and every other geometry/annotation value);
//! * the entity's own linetype **scale** — it is not one of the five keys, so a
//!   named destination linetype keeps the destination's scale (the same rule
//!   `SETENTITYPROPERTY` uses).
//!
//! Different source and destination entity kinds are allowed — that is the point
//! of a property painter — as long as every value is valid for the destination.
//!
//! ## All-or-nothing, one transaction, one undo step
//!
//! Every refusal happens before the transaction is opened, so nothing is written
//! and no history entry appears:
//!
//! * `matchprop.needs_two` — fewer than two selected entities;
//! * `property.not_addressable` / `property.target_missing` — a source **or**
//!   destination ref carries an instance path (a block-instance member) or a
//!   sub-element, belongs to another document, or does not resolve: the exact
//!   rule [`super::properties`] enforces (reused, not re-implemented);
//! * `matchprop.source_value` — the source's stored value has no writable
//!   projection (`property_value` returns `None`) or the shared grammar refuses
//!   it (for example a `ByBlock` transparency or an unnamed linetype). The
//!   refusal names the source entity **and** the property and carries the
//!   parser's own stable `property.*` code.
//!
//! A destination that already matches the source contributes **no** change and
//! is never staged (so undo restores exactly the entities that changed). When
//! every destination already matches, the command reports `matchprop.unchanged`
//! and opens **no** transaction — never an empty undo entry. Otherwise the
//! changed entities are staged in one transaction / one undo step, with patches
//! over both state layers (entity row + display attributes), exactly like
//! `SETENTITYPROPERTY`.

use super::properties::{ensure_addressable, plan_entity, StagedChange};
use super::*;
use crate::property_value::{self, PropertyValue};
use cad_db::{DbEntity, DrawingDatabase};

impl Application {
    /// Paint the first selected entity's five writable display properties onto
    /// the rest of the selection in one transaction.
    pub(crate) fn match_prop(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "matchprop.invalid_payload: MATCHPROP takes no payload; the selection is the \
                 input"
                    .into(),
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

        // The selection's own order is the contract: first picked = source.
        let references = session.selection.refs();
        if references.len() < 2 {
            return Err(CadError::InvalidInput(format!(
                "matchprop.needs_two: MATCHPROP needs a source (the first selected entity) and \
                 at least one destination, got {} selected",
                references.len()
            )));
        }

        // Preflight **every** ref (source and destinations) before resolving a
        // value or staging anything: one non-addressable ref refuses the whole
        // command, exactly like the single-property write.
        for reference in references {
            ensure_addressable(drawing.as_ref(), command, reference)?;
        }

        let source = drawing
            .entity(references[0].entity)
            .expect("the preflight verified the source resolves")
            .clone();
        let targets = source_targets(&drawing, &source)?;
        let planned: Vec<&PropertyValue> = targets.iter().map(|(_, target)| target).collect();

        let mut staged_changes: Vec<StagedChange> = Vec::new();
        for reference in &references[1..] {
            let before = drawing
                .entity(reference.entity)
                .expect("the preflight verified every destination resolves")
                .clone();
            if let Some(change) = plan_entity(&drawing, before, &planned) {
                staged_changes.push(change);
            }
        }

        let destinations = references.len() - 1;
        if staged_changes.is_empty() {
            // Nothing to write: no transaction, no ChangeSet, no history entry.
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "matchprop.unchanged".into(),
                    message: format!(
                        "MATCHPROP: all {destinations} destination entities already match source \
                         entity {}",
                        source.id.0
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
                candidate.begin_drawing_transaction("match entity properties", transaction)?;
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
                label: format!("match properties from {}", source.id.0),
                patches,
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.matchprop".into(),
                message: format!(
                    "MATCHPROP: {count} of {destinations} destination entities now match source \
                     entity {}",
                    source.id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve the source entity's five writable property values into write targets.
///
/// One resolution per property, against the drawing's own tables, before any
/// entity is staged — so a refusal cannot depend on which destination is
/// processed first (the same ordering guarantee `SETENTITYPROPERTY` documents).
fn source_targets(
    drawing: &DrawingDatabase,
    source: &DbEntity,
) -> CadResult<Vec<(&'static str, PropertyValue)>> {
    let mut targets = Vec::with_capacity(property_value::SUPPORTED_KEYS.len());
    for key in property_value::SUPPORTED_KEYS {
        let Some(value) = property_value::property_value(drawing, source, key) else {
            return Err(CadError::Unsupported(format!(
                "matchprop.source_value: source entity {} has no writable {key} value to paint",
                source.id.0
            )));
        };
        let target = property_value::parse(drawing, key, &value)
            .map_err(|error| name_refusal(error, source.id, key, &value))?;
        targets.push((key, target));
    }
    Ok(targets)
}

/// Name the source entity and the property in the shared parser's refusal.
///
/// The [`CadError`] variant is preserved so the parser's stable `property.*` code
/// and the command-layer classification (a refused input vs. an unsupported
/// value) survive; only the message gains the MATCHPROP context.
fn name_refusal(error: CadError, entity: EntityId, key: &str, value: &str) -> CadError {
    let context = format!(
        "matchprop.source_value: source entity {} cannot paint {key} = '{value}': ",
        entity.0
    );
    match error {
        CadError::InvalidInput(message) => CadError::InvalidInput(format!("{context}{message}")),
        CadError::Unsupported(message) => CadError::Unsupported(format!("{context}{message}")),
        other => other,
    }
}
