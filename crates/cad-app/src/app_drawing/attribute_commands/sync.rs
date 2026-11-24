//! `ATTSYNC` and `BATTORDER`: definition-to-reference propagation and attribute
//! display order.
//!
//! Both commands rewrite **every** referencing `INSERT` explicitly (ADR-0055
//! D3); neither relies on a display cache. `ATTSYNC` adds missing tags, drops
//! values whose definition is gone (reporting the unmatched tags in the same
//! diagnostic), refreshes flags/text and forces constant values to their
//! definition default. `BATTORDER` rewrites each definition's stored `order` and
//! reorders every reference's value list in the same transaction.
//!
//! A no-op (the definitions and every value already match) returns
//! `attribute.unchanged` and writes nothing: no revision, no history entry.

use super::*;
use cad_domain::AttributeDefinition;

/// `ATTSYNC`: propagate this block's definitions to all references.
///
/// The payload-free form resolves the target from a selection containing exactly
/// one direct `INSERT`; with any other selection it is the read-only report. The
/// typed form names the block explicitly.
pub(super) fn attsync(
    app: &mut Application,
    session: &SessionState,
    command: &Command,
) -> CadResult<CommandOutcome> {
    let block = match &command.payload {
        CommandPayload::Attribute(AttributeCommand::Sync(payload)) => {
            let (_, drawing) = drawing_of(app, command)?;
            resolve_block(&drawing, &payload.block)?
        }
        CommandPayload::None => {
            let (_, drawing) = drawing_of(app, command)?;
            let refs = session.selection.refs();
            if refs.len() != 1
                || !refs[0].instance.0.is_empty()
                || refs[0].sub_element.is_some()
                || refs[0].document != command.document
            {
                return report::attsync_report(app, command);
            }
            let entity = drawing.entity(refs[0].entity).ok_or_else(|| {
                refusal(
                    "target_missing",
                    format!("entity {} does not exist", refs[0].entity.0),
                )
            })?;
            match &entity.geometry {
                SemanticGeometry::Insert { block, .. } => *block,
                _ => return report::attsync_report(app, command),
            }
        }
        _ => {
            return Err(refusal(
                "sync_payload",
                "ATTSYNC takes a block name, not this payload",
            ))
        }
    };

    let (_, candidate) = drawing_of(app, command)?;
    let definitions = definitions(&candidate, block)?;
    let layers: Vec<LayerId> = definitions
        .iter()
        .map(|(id, _)| {
            candidate
                .entity(*id)
                .map(|entity| entity.layer)
                .unwrap_or(LayerId(0))
        })
        .collect();
    let reference_ids = references(&candidate, block);
    let mut before = Vec::new();
    let mut changed = Vec::new();
    let mut unmatched = 0usize;
    for id in reference_ids {
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
        let known: Vec<&AttributeDefinition> = definitions
            .iter()
            .map(|(_, definition)| definition)
            .collect();
        unmatched += attributes
            .iter()
            .filter(|value| {
                !known
                    .iter()
                    .any(|definition| tags_equal(&definition.tag, &value.tag))
            })
            .count();
        let next = sync_values(&definitions, &layers, &attributes);
        if next == attributes {
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
            "ATTSYNC: block '{}' has {} definition(s) and every reference already matches; \
             nothing was written",
            block_name(&candidate, block),
            definitions.len()
        )));
    }
    let written = changed.len();
    let mut diagnostics = vec![Diagnostic {
        object: None,
        code: "attsync.changed".into(),
        message: format!(
            "ATTSYNC: propagated {} attribute definition(s) of block '{}' to {written} \
             reference(s); one transaction, one undo step",
            definitions.len(),
            block_name(&candidate, block)
        ),
    }];
    if unmatched > 0 {
        diagnostics.push(Diagnostic {
            object: None,
            code: "attsync.unmatched_value".into(),
            message: format!(
                "ATTSYNC: dropped {unmatched} stored value(s) whose tag no current definition \
                 declares; no reference retains a stale value"
            ),
        });
    }
    commit_attribute_write(
        app,
        command,
        candidate,
        "sync attributes",
        Vec::new(),
        Vec::new(),
        changed,
        before,
        diagnostics,
    )
}

fn block_name(drawing: &DrawingDatabase, block: BlockId) -> String {
    drawing
        .block_name(block)
        .map(str::to_string)
        .unwrap_or_else(|| format!("block#{}", block.0))
}

/// `BATTORDER`: rewrite the block's attribute order and every reference's value
/// order.
pub(super) fn battorder(app: &mut Application, command: &Command) -> CadResult<CommandOutcome> {
    let CommandPayload::Attribute(AttributeCommand::Order(payload)) = &command.payload else {
        return Err(refusal(
            "order_payload",
            "BATTORDER needs a typed block and tag order; the payload-free form is the report",
        ));
    };
    let (_, candidate) = drawing_of(app, command)?;
    let block = resolve_block(&candidate, &payload.block)?;
    let definitions = definitions(&candidate, block)?;
    if definitions.is_empty() {
        return Err(refusal(
            "no_definitions",
            format!("block '{}' has no attribute definitions", payload.block),
        ));
    }
    let ordered = validate_order(&definitions, &payload.tags)?;
    let current_order: Vec<u32> = definitions.iter().map(|(_, d)| d.order).collect();
    let new_order: Vec<u32> = ordered.iter().map(|(_, index)| *index as u32).collect();
    let definitions_changed = current_order != new_order;

    let mut before = Vec::new();
    let mut changed = Vec::new();
    let mut dropped = 0usize;
    if definitions_changed {
        for (slot, (entity_id, _)) in ordered.iter().enumerate() {
            let mut entity = candidate.entity(*entity_id).cloned().ok_or_else(|| {
                unsupported("definition_missing", "attribute definition vanished")
            })?;
            let SemanticGeometry::AttributeDefinition(definition) = &mut entity.geometry else {
                return Err(unsupported(
                    "definition_missing",
                    "attribute definition row changed kind",
                ));
            };
            definition.order = slot as u32;
            before.push((
                entity.id,
                Some(candidate.entity(entity.id).cloned().unwrap()),
            ));
            changed.push((entity.id, Some(entity)));
        }
    }

    // Rebuild each reference's list in the new definition order. A stored tag
    // whose definition disappeared is retained at the end and reported.
    let ordered_tags: Vec<String> = ordered
        .iter()
        .map(|(_, index)| definitions[*index].1.tag.clone())
        .collect();
    let reference_ids = references(&candidate, block);
    for id in reference_ids {
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
        dropped += attributes
            .iter()
            .filter(|value| !ordered_tags.iter().any(|tag| tags_equal(tag, &value.tag)))
            .count();
        let mut next: Vec<AttributeValue> = Vec::with_capacity(attributes.len());
        for tag in &ordered_tags {
            if let Some(value) = attributes.iter().find(|value| tags_equal(&value.tag, tag)) {
                next.push(value.clone());
            }
        }
        if next == attributes {
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
            "BATTORDER: block '{}' already stores this attribute order; nothing was written",
            payload.block
        )));
    }
    let written = changed.len();
    let mut diagnostics = vec![Diagnostic {
        object: None,
        code: "battorder.changed".into(),
        message: format!(
            "BATTORDER: block '{}' attribute order is now {}; {written} row(s) rewritten in one \
             transaction",
            payload.block,
            ordered_tags.join(" > ")
        ),
    }];
    if dropped > 0 {
        diagnostics.push(Diagnostic {
            object: None,
            code: "battorder.unmatched_value".into(),
            message: format!(
                "BATTORDER: dropped {dropped} stored value(s) whose tag no current definition \
                 declares"
            ),
        });
    }
    commit_attribute_write(
        app,
        command,
        candidate,
        "reorder attributes",
        Vec::new(),
        Vec::new(),
        changed,
        before,
        diagnostics,
    )
}

/// Validate an explicit complete tag order and return `(entity id, old index)`
/// in the requested order.
fn validate_order(
    definitions: &[(EntityId, AttributeDefinition)],
    tags: &[String],
) -> CadResult<Vec<(EntityId, usize)>> {
    if tags.len() != definitions.len() {
        return Err(refusal(
            "order_mismatch",
            format!(
                "BATTORDER needs every definition's tag exactly once ({} tag(s)), got {}",
                definitions.len(),
                tags.len()
            ),
        ));
    }
    let mut out = Vec::with_capacity(tags.len());
    let mut used = vec![false; definitions.len()];
    for tag in tags {
        let index = definitions
            .iter()
            .position(|(_, definition)| tags_equal(&definition.tag, tag))
            .ok_or_else(|| {
                refusal(
                    "order_mismatch",
                    format!("tag '{tag}' is not an attribute definition of this block"),
                )
            })?;
        if used[index] {
            return Err(refusal(
                "order_mismatch",
                format!("tag '{tag}' is listed more than once"),
            ));
        }
        used[index] = true;
        out.push((definitions[index].0, index));
    }
    Ok(out)
}
