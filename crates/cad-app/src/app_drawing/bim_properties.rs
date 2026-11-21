//! `SETENTITYPROPERTY` for one dynamic BIM PSet slot (`bim.pset.{set}.{prop}`).
//!
//! The properties panel emits one dynamic row per stored, non-null PSet property
//! ([`crate::selection_bim`]), and a row is writable only when its ref is
//! directly addressable and its value is a scalar. This module is the write half
//! of that contract: it is reached from the one property command, not from a new
//! command id, and it never lets a display-property grammar see these keys
//! ([`crate::property_value`] deliberately does not know them).
//!
//! # One slot, every selected element
//!
//! The key names a **position** in the stored PSet vector, not a source name.
//! A multi-selection can therefore write the same position on several elements
//! only when that position still denotes one property: every selected element
//! must carry the same `(pset name, property name)` pair, compared byte for byte.
//! A difference is `property.bim_conflict` and writes nothing — the same rule the
//! read side uses to turn the row into a mixed, non-editable row.
//!
//! # Each element keeps its own variant
//!
//! The draft string is resolved **per element against that element's own stored
//! variant**, so `"5"` can write a `Real` on one element and an `Integer` on
//! another. A `Real` keeps its declared measure: the draft may repeat the unit
//! token (`"3.5 m"`), which must equal the stored token byte for byte, and the
//! original `ifc_type`/`unit` are written back unchanged. A real must be finite
//! (`"inf"`/`"NaN"` are refused, never stored). An integer is a strict
//! [`i64`] parse and a boolean is exactly `true`/`false`.
//!
//! # All or nothing, one revision, one undo step
//!
//! Every refusal happens before the write: addressability, entity existence, a
//! stored BIM block, the slot being in range, a non-null value, the identical
//! property pair, and the draft grammar. The changed elements are then written
//! through [`cad_db::DrawingDatabase::apply_bim_updates`] in one transaction
//! (one revision, one `ChangeSet`), and the same transaction is recorded as one
//! undo step carrying the exact before/after element rows
//! ([`cad_db::BimPatch`]). If no element changes, the command opens no
//! transaction, records no history and reports `property.unchanged`.
//!
//! # `Heuristic` elements are editable
//!
//! A heuristic import is still the document's stored data: the user is editing
//! the value the document holds, not re-importing the source. The edit keeps
//! `evidence` exactly as the importer recorded it — rewriting a value must
//! neither promote it to `declared` nor erase how the data was obtained.

use super::*;
use crate::selection_bim::BimPropertyKey;
use cad_db::{BimElement, BimPatch, BimValue, DrawingDatabase, BIM_MAX_STRING_BYTES};
use cad_history::DrawingUndoRecord;

/// One selected element prepared for the write: its stored block, the value at
/// the addressed slot, and the value the draft resolves to for this variant.
struct BimTarget {
    id: EntityId,
    before: BimElement,
    current: BimValue,
    resolved: BimValue,
}

impl Application {
    /// Write one dynamic BIM property slot on every currently selected element.
    ///
    /// The key is already parsed ([`BimPropertyKey`]) by the property command's
    /// prefix dispatch; the selection is read from the session at apply time, so
    /// a stale UI cannot retarget another element.
    pub(crate) fn set_bim_property(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        slot: BimPropertyKey,
        value: &str,
    ) -> CadResult<CommandOutcome> {
        let key = slot.key();
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        let references = session.selection.refs();
        if references.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "property.selection_empty: select at least one BIM element to change {key}"
            )));
        }

        // Preflight the whole selection before anything is staged. The first
        // element defines the property pair every other element must match.
        let mut targets: Vec<BimTarget> = Vec::with_capacity(references.len());
        let mut expected_names: Option<(&str, &str)> = None;
        for reference in references {
            super::properties::ensure_addressable(drawing.as_ref(), command, reference)?;
            let id = reference.entity;
            let element = drawing.bim_element(id).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "property.unsupported_key: entity {} has no BIM data to edit; \
                     BIM data is attached by an importer, not by a property edit",
                    id.0
                ))
            })?;
            let (set_name, property_name, current) = slot_value(element, slot, id)?;
            match expected_names {
                None => expected_names = Some((set_name, property_name)),
                Some((set, property)) if set != set_name || property != property_name => {
                    return Err(CadError::InvalidInput(format!(
                        "property.bim_conflict: {key} names {set:?}.{property:?} on one \
                         selected element and {set_name:?}.{property_name:?} on entity {}; \
                         one write cannot address two different properties",
                        id.0
                    )));
                }
                Some(_) => {}
            }
            let resolved = resolve_value(&key, current, value)?;
            targets.push(BimTarget {
                id,
                before: element.clone(),
                current: current.clone(),
                resolved,
            });
        }

        // A value that already equals the stored one is not staged, so a
        // selection that is entirely unchanged has no write and no undo entry.
        let changed: Vec<&BimTarget> = targets
            .iter()
            .filter(|target| target.resolved != target.current)
            .collect();
        if changed.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "property.unchanged".into(),
                    message: format!(
                        "SETENTITYPROPERTY: {key} already has the requested value on every \
                         selected element"
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let transaction = next_transaction_id();
        let reason = format!("set bim property {key}");
        let mut candidate = drawing.as_ref().clone();
        let updates: Vec<(EntityId, BimElement)> = changed
            .iter()
            .map(|target| {
                let mut after = target.before.clone();
                after.psets[slot.set].properties[slot.property].value = target.resolved.clone();
                (target.id, after)
            })
            .collect();
        let change_set = candidate.apply_bim_updates(&reason, transaction, &updates)?;

        // Read the committed rows back for the undo patches: the patch carries
        // what the database stored, never a re-derivation of the draft.
        let mut patches: Vec<BimPatch> = Vec::with_capacity(changed.len());
        for target in &changed {
            let after = candidate
                .bim_element(target.id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("edited BIM element vanished".into()))?;
            patches.push(BimPatch {
                id: target.id,
                before: target.before.clone(),
                after,
            });
        }
        let objects: Vec<ObjectId> = changed.iter().map(|target| ObjectId(target.id.0)).collect();
        let count = objects.len();
        self.publish_candidate_with_bim(
            document_id,
            candidate,
            DrawingUndoRecord::entity_only(change_set.transaction, reason, Vec::new()),
            patches,
        )?;

        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.property.set".into(),
                message: format!("SETENTITYPROPERTY: set {key} on {count} elements"),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// [`Self::publish_candidate`] for a step whose only rows are BIM elements.
    ///
    /// The patches are validated and recorded **with** the record, before the
    /// candidate is published, so a refused step leaves the open document, its
    /// revision and history untouched. A BIM step is applied by its own
    /// replace-only write path, so the record itself must carry no other batch
    /// ([`cad_history::DrawingHistoryStep::validate`] enforces that).
    pub(crate) fn publish_candidate_with_bim(
        &mut self,
        document_id: DocumentId,
        candidate: DrawingDatabase,
        record: DrawingUndoRecord,
        bim_patches: Vec<BimPatch>,
    ) -> CadResult<()> {
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        self.history
            .entry(document_id)
            .or_default()
            .record_drawing_with_bim(record, bim_patches)?;
        document.drawing = Arc::new(candidate);
        Ok(())
    }
}

/// The `(pset name, property name, value)` at `slot` in one element's stored
/// block, or the named refusal.
///
/// An out-of-range index is `property.target_missing`: the key is well formed but
/// this element has no such position. A `Null` value is `property.unsupported_key`
/// — the read side emits no row for it, so there is no writable slot.
fn slot_value(
    element: &BimElement,
    slot: BimPropertyKey,
    id: EntityId,
) -> CadResult<(&str, &str, &BimValue)> {
    let key = slot.key();
    let set = element.psets.get(slot.set).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "property.target_missing: entity {} has no BIM property set at {key}",
            id.0
        ))
    })?;
    let property = set.properties.get(slot.property).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "property.target_missing: entity {} has no BIM property at {key}",
            id.0
        ))
    })?;
    if property.value == BimValue::Null {
        return Err(CadError::Unsupported(format!(
            "property.unsupported_key: {key} is null on entity {}; a null value has no row \
             and no value to replace",
            id.0
        )));
    }
    Ok((&set.name, &property.name, &property.value))
}

/// Resolve one draft string against an element's own stored variant.
///
/// `Text` stores the draft verbatim (a byte over [`BIM_MAX_STRING_BYTES`] is
/// `property.bim_too_long`, never truncated); `Real` keeps its `ifc_type` and
/// `unit`; `Integer` is a strict [`i64`] parse; `Bool` accepts exactly
/// `true`/`false`. Anything else is `property.invalid_value`, which refuses the
/// whole command.
fn resolve_value(key: &str, current: &BimValue, draft: &str) -> CadResult<BimValue> {
    match current {
        BimValue::Text(_) => {
            if draft.len() > BIM_MAX_STRING_BYTES {
                return Err(CadError::InvalidInput(format!(
                    "property.bim_too_long: {key} is {} bytes, over the {BIM_MAX_STRING_BYTES}-byte \
                     string budget",
                    draft.len()
                )));
            }
            Ok(BimValue::Text(draft.to_string()))
        }
        BimValue::Real { ifc_type, unit, .. } => Ok(BimValue::Real {
            value: parse_real(key, draft, unit.as_deref())?,
            ifc_type: ifc_type.clone(),
            unit: unit.clone(),
        }),
        BimValue::Integer(_) => draft.parse::<i64>().map(BimValue::Integer).map_err(|_| {
            CadError::InvalidInput(format!(
                "property.invalid_value: {key} takes an integer, not {draft:?}"
            ))
        }),
        BimValue::Bool(_) => match draft {
            "true" => Ok(BimValue::Bool(true)),
            "false" => Ok(BimValue::Bool(false)),
            _ => Err(CadError::InvalidInput(format!(
                "property.invalid_value: {key} takes true or false, not {draft:?}"
            ))),
        },
        BimValue::Null => Err(CadError::Unsupported(format!(
            "property.unsupported_key: {key} is null; a null value has no value to replace"
        ))),
    }
}

/// Parse a real draft: a finite number, optionally followed by exactly one space
/// and the stored unit token.
///
/// The suffix must equal the stored unit byte for byte (and must be absent when
/// the element declares no unit), so `"4 m"` cannot silently write a unit-less
/// real or convert millimetres into metres. The measure itself is preserved by
/// the caller; this function only produces the number.
fn parse_real(key: &str, draft: &str, unit: Option<&str>) -> CadResult<f64> {
    let (number, suffix) = match draft.split_once(' ') {
        Some((number, suffix)) => (number, Some(suffix)),
        None => (draft, None),
    };
    // A bare number is accepted for a unit-bearing real (the previous value's
    // measure is kept); a suffix is accepted only when it reproduces the stored
    // unit byte for byte, and is refused when the element declares no unit.
    let suffix_matches = match (suffix, unit) {
        (None, _) => true,
        (Some(suffix), Some(unit)) => suffix == unit,
        (Some(_), None) => false,
    };
    if !suffix_matches {
        let expected = match unit {
            Some(unit) => format!("{number:?} or {number:?} {unit}"),
            None => format!("{number:?} (no unit)"),
        };
        return Err(CadError::InvalidInput(format!(
            "property.invalid_value: {key} keeps its stored unit; expected {expected}, \
             got {draft:?}"
        )));
    }
    let value = number.parse::<f64>().map_err(|_| {
        CadError::InvalidInput(format!(
            "property.invalid_value: {key} takes a real number, not {number:?}"
        ))
    })?;
    if !value.is_finite() {
        return Err(CadError::InvalidInput(format!(
            "property.invalid_value: {key} takes a finite real number, not {number:?}"
        )));
    }
    Ok(value)
}
