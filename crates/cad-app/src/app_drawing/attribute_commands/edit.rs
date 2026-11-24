//! `ATTEDIT` / `-ATTEDIT` / `EATTEDIT` / `ATTIPEDIT` / `GATTE`: value edits.
//!
//! One shared core resolves the block's definitions and then writes the named
//! tags on the requested references. The selectors differ only in **which**
//! references they reach and whether a text-placement override is admitted:
//!
//! | selector | references | placement |
//! | --- | --- | --- |
//! | `ATTEDIT` | exactly one selected direct `INSERT` | value only |
//! | `ATTIPEDIT` | exactly one selected direct `INSERT` | value only |
//! | `EATTEDIT` | one or more selected direct `INSERT`s | value + position/height/rotation |
//! | `-ATTEDIT` | every `INSERT` of the block (`AllInstances`) | value only |
//! | `GATTE` | every `INSERT` of the block (`AllInstances`) | value only |
//!
//! Tag identity is ASCII case-insensitive. A tag absent from the definition is
//! `attribute.unknown_tag`; a `constant` definition is `attribute.constant`; a
//! placement override on a `locked_position` definition is
//! `attribute.locked_position`. All three are refused before any row is written.

use super::*;
use cad_domain::AttributeFlags;

/// The reference set a value edit is allowed to touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EditTargets {
    /// Exactly one selected direct INSERT.
    OneSelected,
    /// One or more selected direct INSERTs.
    Selected,
    /// Every INSERT of the block in the document.
    AllInstances,
}

/// Dispatch one value-edit selector.
pub(super) fn edit_values(
    app: &mut Application,
    session: &SessionState,
    command: &Command,
    targets: EditTargets,
    selector: &str,
) -> CadResult<CommandOutcome> {
    let CommandPayload::Attribute(AttributeCommand::Edit(payload)) = &command.payload else {
        return Err(refusal(
            "edit_payload",
            format!("{selector} needs a typed attribute edit; the payload-free form is refused"),
        ));
    };
    match (targets, payload.scope) {
        (EditTargets::AllInstances, AttributeEditScope::AllInstances)
        | (EditTargets::OneSelected | EditTargets::Selected, AttributeEditScope::Selection) => {}
        _ => {
            return Err(refusal(
                "scope_mismatch",
                format!("{selector} received the wrong attribute edit scope"),
            ))
        }
    }
    if payload.assignments.is_empty() {
        return Err(refusal(
            "assignment_empty",
            format!("{selector} needs at least one TAG <value> assignment"),
        ));
    }
    let mut seen: Vec<&str> = Vec::new();
    for assignment in &payload.assignments {
        if assignment.tag.trim().is_empty() {
            return Err(refusal("invalid_tag", "an assignment tag may not be empty"));
        }
        if seen.iter().any(|tag| tags_equal(tag, &assignment.tag)) {
            return Err(refusal(
                "duplicate_assignment",
                format!("tag '{}' is assigned twice", assignment.tag),
            ));
        }
        seen.push(&assignment.tag);
    }
    if let Some(position) = payload.position {
        if !is_finite_point(position) {
            return Err(refusal(
                "invalid_text",
                "the attribute position override must be finite",
            ));
        }
    }
    if let Some(height) = payload.height {
        if !height.is_finite() || height <= 0.0 {
            return Err(refusal(
                "invalid_text",
                format!("the attribute height must be positive and finite, got {height}"),
            ));
        }
    }
    if let Some(rotation) = payload.rotation {
        if !rotation.is_finite() {
            return Err(refusal(
                "invalid_text",
                "the attribute rotation must be finite",
            ));
        }
    }

    let (_, candidate) = drawing_of(app, command)?;
    let block = resolve_block(&candidate, &payload.block)?;
    let definitions = definitions(&candidate, block)?;
    if definitions.is_empty() {
        return Err(refusal(
            "no_definitions",
            format!("block '{}' has no attribute definitions", payload.block),
        ));
    }
    let mut resolved: Vec<(usize, &AttributeDefinition, LayerId, EntityId)> = Vec::new();
    for assignment in &payload.assignments {
        let index = definitions
            .iter()
            .position(|(_, definition)| tags_equal(&definition.tag, &assignment.tag))
            .ok_or_else(|| {
                refusal(
                    "unknown_tag",
                    format!(
                        "block '{}' has no attribute tag '{}'",
                        payload.block, assignment.tag
                    ),
                )
            })?;
        resolved.push((
            index,
            &definitions[index].1,
            candidate
                .entity(definitions[index].0)
                .map(|entity| entity.layer)
                .unwrap_or(LayerId(0)),
            definitions[index].0,
        ));
    }
    for (_, definition, _, _) in &resolved {
        if definition.flags.constant {
            return Err(refusal(
                "constant",
                format!(
                    "attribute '{}' is constant; its value is fixed at definition time and cannot \
                     be edited",
                    definition.tag
                ),
            ));
        }
        if (payload.position.is_some() || payload.height.is_some() || payload.rotation.is_some())
            && definition.flags.locked_position
        {
            return Err(refusal(
                "locked_position",
                format!(
                    "attribute '{}' has its position locked; only its text value can be edited",
                    definition.tag
                ),
            ));
        }
    }

    let targets = match targets {
        EditTargets::AllInstances => references(&candidate, block),
        EditTargets::OneSelected | EditTargets::Selected => {
            let selected = selected_references(session, &candidate, command, block, selector)?;
            if targets == EditTargets::OneSelected && selected.len() != 1 {
                return Err(refusal(
                    "selection_count",
                    format!(
                        "{selector} edits one block reference at a time; {} are selected",
                        selected.len()
                    ),
                ));
            }
            selected
        }
    };
    if targets.is_empty() {
        return Err(refusal(
            "no_references",
            format!(
                "{selector}: no INSERT of block '{}' was found",
                payload.block
            ),
        ));
    }

    let mut before = Vec::new();
    let mut changed = Vec::new();
    for id in targets {
        let Some(entity) = candidate.entity(id).cloned() else {
            continue;
        };
        let SemanticGeometry::Insert {
            block: referenced,
            transform,
            attributes,
        } = entity.geometry.clone()
        else {
            continue;
        };
        if referenced != block {
            continue;
        }
        let mut next = attributes.clone();
        let mut touched = false;
        for (assignment, (index, definition, layer, _)) in
            payload.assignments.iter().zip(resolved.iter())
        {
            let _ = index;
            let value = match next
                .iter_mut()
                .find(|value| tags_equal(&value.tag, &assignment.tag))
            {
                Some(value) => value,
                None => {
                    next.push(value_from_definition(definition, *layer));
                    next.last_mut().expect("just pushed")
                }
            };
            if value.value != assignment.value {
                value.value = assignment.value.clone();
                touched = true;
            }
            if value.flags != definition.flags {
                value.flags = AttributeFlags {
                    // Keep only the flags the definition owns; the value mirrors
                    // the definition so a detached value stays self-describing.
                    ..definition.flags
                };
                touched = true;
            }
            if let Some(position) = payload.position {
                if value.text.position != position {
                    value.text.position = position;
                    touched = true;
                }
            }
            if let Some(height) = payload.height {
                if value.text.height != height {
                    value.text.height = height;
                    touched = true;
                }
            }
            if let Some(rotation) = payload.rotation {
                if value.text.rotation != rotation {
                    value.text.rotation = rotation;
                    touched = true;
                }
            }
        }
        if !touched {
            continue;
        }
        let mut after = entity.clone();
        after.geometry = SemanticGeometry::Insert {
            block: referenced,
            transform,
            attributes: next,
        };
        before.push((id, Some(entity)));
        changed.push((id, Some(after)));
    }
    if changed.is_empty() {
        return Ok(unchanged(format!(
            "{selector}: every targeted reference already stores the requested value(s); nothing \
             was written"
        )));
    }
    let references_written = changed.len();
    let diagnostics = vec![Diagnostic {
        object: None,
        code: "attedit.changed".into(),
        message: format!(
            "{selector}: wrote {} assignment(s) on {references_written} block reference(s) of \
             block '{}'; one transaction, one undo step",
            payload.assignments.len(),
            payload.block
        ),
    }];
    commit_attribute_write(
        app,
        command,
        candidate,
        "edit attribute values",
        Vec::new(),
        Vec::new(),
        changed,
        before,
        diagnostics,
    )
}

/// Resolve the current selection to direct `INSERT`s of `block`.
fn selected_references(
    session: &SessionState,
    drawing: &DrawingDatabase,
    command: &Command,
    block: BlockId,
    selector: &str,
) -> CadResult<Vec<EntityId>> {
    let refs = session.selection.refs();
    if refs.is_empty() {
        return Err(refusal(
            "selection_empty",
            format!("{selector} needs a selected block reference"),
        ));
    }
    let mut out = Vec::new();
    for reference in refs {
        if reference.document != command.document {
            return Err(refusal(
                "target_missing",
                format!("entity {} belongs to another document", reference.entity.0),
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(refusal(
                "not_addressable",
                format!(
                    "entity {} is a block-instance member or sub-element, not a direct reference",
                    reference.entity.0
                ),
            ));
        }
        let entity = drawing.entity(reference.entity).ok_or_else(|| {
            refusal(
                "target_missing",
                format!("entity {} does not exist", reference.entity.0),
            )
        })?;
        match &entity.geometry {
            SemanticGeometry::Insert {
                block: referenced, ..
            } if *referenced == block => {}
            SemanticGeometry::Insert { .. } => {
                return Err(refusal(
                    "block_mismatch",
                    format!(
                        "entity {} references another block, not '{}'",
                        reference.entity.0, block.0
                    ),
                ))
            }
            _ => {
                return Err(refusal(
                    "target_not_insert",
                    format!("entity {} is not a block reference", reference.entity.0),
                ))
            }
        }
        if !out.contains(&entity.id) {
            out.push(entity.id);
        }
    }
    Ok(out)
}
