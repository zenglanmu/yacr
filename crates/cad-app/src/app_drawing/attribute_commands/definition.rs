//! `ATTDEF` / `-ATTDEF` / `ATTREDEF`: definition creation and redefinition.
//!
//! `ATTDEF` adds one member entity to an existing block definition. It does not
//! rewrite existing references — AutoCAD's semantics are "the next insertion
//! prompts for it; ATTSYNC is what applies the definition to existing
//! references", and inventing values on every reference here would make
//! `ATTSYNC` unobservable. `ATTREDEF` is the definition **and** propagation
//! command: it writes the definition row and every referencing `INSERT`'s value
//! list in the same transaction (ADR-0055 D3).
//!
//! Every check runs before the combined write. A refusal leaves the block table,
//! every reference, the allocators and the revision untouched.

use super::*;
use cad_db::BlockDefinition;
use cad_domain::{AttributeDefinition, AttributeText, TextAlignH, TextAlignV, TextLineSpacing};

/// The text style/font a new definition resolves through (the same rule TEXT and
/// MTEXT creation use).
fn resolved_text(drawing: &DrawingDatabase, payload: &AttributeDefinitionCommand) -> AttributeText {
    let (style, font) = super::super::resolve_created_text_style(drawing);
    AttributeText {
        position: payload.position,
        height: payload.height,
        rotation: payload.rotation,
        style,
        width_factor: payload.width_factor,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: payload.wrap_width,
        line_spacing: TextLineSpacing::default(),
        font,
        oblique: payload.oblique,
    }
}

/// Validate a definition payload against the drawing without writing.
fn validate_definition(
    drawing: &DrawingDatabase,
    definitions: &[(EntityId, AttributeDefinition)],
    payload: &AttributeDefinitionCommand,
    redefinition: bool,
) -> CadResult<Option<usize>> {
    let tag = payload.tag.trim();
    if tag.is_empty() || tag.contains(['\r', '\n']) {
        return Err(refusal(
            "invalid_tag",
            "an attribute tag must be a non-empty single line",
        ));
    }
    if tag.chars().count() > MAX_TOOL_TEXT_CHARS {
        return Err(refusal(
            "invalid_tag",
            format!(
                "an attribute tag allows at most {MAX_TOOL_TEXT_CHARS} characters, got {}",
                tag.chars().count()
            ),
        ));
    }
    if !is_finite_point(payload.position) {
        return Err(refusal(
            "invalid_text",
            "the attribute position must be finite",
        ));
    }
    if !payload.height.is_finite() || payload.height <= 0.0 {
        return Err(refusal(
            "invalid_text",
            format!(
                "the attribute height must be positive and finite, got {}",
                payload.height
            ),
        ));
    }
    if !payload.rotation.is_finite() || !payload.oblique.is_finite() {
        return Err(refusal(
            "invalid_text",
            "the attribute rotation and oblique angle must be finite",
        ));
    }
    if !payload.width_factor.is_finite() || payload.width_factor <= 0.0 {
        return Err(refusal(
            "invalid_text",
            format!(
                "the attribute width factor must be positive and finite, got {}",
                payload.width_factor
            ),
        ));
    }
    if let Some(width) = payload.wrap_width {
        if !width.is_finite() || width <= 0.0 {
            return Err(refusal(
                "invalid_text",
                "the attribute wrap width must be positive and finite when set",
            ));
        }
    }
    let existing = definitions
        .iter()
        .position(|(_, definition)| tags_equal(&definition.tag, tag));
    if !redefinition && existing.is_some() {
        return Err(refusal(
            "duplicate_tag",
            format!(
                "block '{}' already defines attribute tag '{tag}'; tags are unique \
                 case-insensitively inside one block",
                payload.block
            ),
        ));
    }
    let _ = drawing;
    Ok(existing)
}

/// The block row after a definition member is added (or the stored row for a
/// pure update). `new_member` is appended because it carries the block's next
/// highest draw order; the combined path re-validates the exact list.
fn block_row_with_member(
    drawing: &DrawingDatabase,
    block: BlockId,
    new_member: Option<EntityId>,
) -> CadResult<BlockRow> {
    let stored = drawing
        .block(block)
        .ok_or_else(|| unsupported("unknown_block", format!("block {} is not stored", block.0)))?
        .clone();
    let mut entities = stored.entities.clone();
    if let Some(member) = new_member {
        entities.push(member);
    }
    Ok(BlockRow {
        definition: BlockDefinition {
            id: block,
            entities,
            dynamic_visibility: stored.dynamic_visibility,
        },
        metadata: drawing.block_metadata(block).cloned(),
    })
}

/// The next draw order for a new block member.
fn next_member_draw_order(drawing: &DrawingDatabase, block: BlockId) -> i64 {
    drawing
        .block(block)
        .map(|definition| definition.entities.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|member| drawing.entity(*member))
        .map(|member| member.draw_order)
        .max()
        .map(|max| max.saturating_add(1))
        .unwrap_or(0)
}

/// Add one `ATTDEF` member to an existing block.
pub(super) fn attdef(
    app: &mut Application,
    session: &SessionState,
    command: &Command,
) -> CadResult<CommandOutcome> {
    let CommandPayload::Attribute(AttributeCommand::Definition(payload)) = &command.payload else {
        return Err(refusal(
            "def_payload",
            "ATTDEF needs a typed attribute definition; the payload-free form is the report",
        ));
    };
    let (_, mut candidate) = drawing_of(app, command)?;
    let block = resolve_block(&candidate, &payload.block)?;
    let current = definitions(&candidate, block)?;
    validate_definition(&candidate, &current, payload, false)?;

    let order = payload.order.unwrap_or_else(|| {
        current
            .iter()
            .map(|(_, d)| d.order)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    });
    let definition = AttributeDefinition {
        tag: payload.tag.trim().to_string(),
        prompt: payload.prompt.clone(),
        default_value: payload.default_value.clone(),
        flags: payload.flags,
        order,
        text: resolved_text(&candidate, payload),
        source_handle: None,
    };
    let id = candidate.allocate_entity_id();
    let entity = DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id.0),
            type_key: "AcDbAttributeDefinition".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer: session.active_layer,
        space: SpaceId::Block(block),
        geometry: SemanticGeometry::AttributeDefinition(Box::new(definition)),
        draw_order: next_member_draw_order(&candidate, block),
    };
    let before = vec![(id, None)];
    let block_row = block_row_with_member(&candidate, block, Some(id))?;
    let block_patches = vec![BlockPatch {
        id: block,
        before: Some(block_row_with_member(&candidate, block, None)?),
        after: Some(block_row.clone()),
    }];
    let changed = vec![(id, Some(entity))];
    let diagnostics = vec![Diagnostic {
        object: Some(ObjectId(id.0)),
        code: "attdef.created".into(),
        message: format!(
            "ATTDEF: added attribute '{}' to block '{}' as entity {} (order {order}); existing \
             references are unchanged until ATTSYNC",
            payload.tag.trim(),
            payload.block,
            id.0
        ),
    }];
    commit_attribute_write(
        app,
        command,
        candidate,
        "define attribute",
        vec![(block, Some(block_row))],
        block_patches,
        changed,
        before,
        diagnostics,
    )
}

/// Redefine an attribute (add it when absent) and propagate the definition to
/// every reference in the same transaction.
pub(super) fn attredef(
    app: &mut Application,
    session: &SessionState,
    command: &Command,
) -> CadResult<CommandOutcome> {
    let CommandPayload::Attribute(AttributeCommand::Definition(payload)) = &command.payload else {
        return Err(refusal(
            "def_payload",
            "ATTREDEF needs a typed attribute definition; the payload-free form is the report",
        ));
    };
    let (_, mut candidate) = drawing_of(app, command)?;
    let block = resolve_block(&candidate, &payload.block)?;
    let current = definitions(&candidate, block)?;
    let existing = validate_definition(&candidate, &current, payload, true)?;

    let order = payload
        .order
        .or_else(|| existing.map(|index| current[index].1.order))
        .unwrap_or_else(|| {
            current
                .iter()
                .map(|(_, d)| d.order)
                .max()
                .unwrap_or(0)
                .saturating_add(1)
        });
    let mut updated = AttributeDefinition {
        tag: payload.tag.trim().to_string(),
        prompt: payload.prompt.clone(),
        default_value: payload.default_value.clone(),
        flags: payload.flags,
        order,
        text: resolved_text(&candidate, payload),
        source_handle: existing.and_then(|index| current[index].1.source_handle.clone()),
    };
    let mut new_definitions = current.clone();
    let mut before = Vec::new();
    let mut changed = Vec::new();
    let mut added_member: Option<EntityId> = None;
    let definition_entity: Option<EntityId>;
    if let Some(index) = existing {
        let (id, old) = current[index].clone();
        let mut entity = candidate.entity(id).cloned().ok_or_else(|| {
            unsupported("definition_missing", format!("definition {id:?} vanished"))
        })?;
        updated.source_handle = old.source_handle.clone();
        entity.geometry = SemanticGeometry::AttributeDefinition(Box::new(updated.clone()));
        new_definitions[index].1 = updated.clone();
        definition_entity = Some(id);
        before.push((id, Some(candidate.entity(id).cloned().unwrap())));
        changed.push((id, Some(entity)));
    } else {
        let id = candidate.allocate_entity_id();
        let entity = DbEntity {
            object: cad_db::DbObject {
                id: ObjectId(id.0),
                type_key: "AcDbAttributeDefinition".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id,
            layer: session.active_layer,
            space: SpaceId::Block(block),
            geometry: SemanticGeometry::AttributeDefinition(Box::new(updated.clone())),
            draw_order: next_member_draw_order(&candidate, block),
        };
        added_member = Some(id);
        definition_entity = Some(id);
        before.push((id, None));
        changed.push((id, Some(entity)));
        new_definitions.push((id, updated.clone()));
    }
    new_definitions.sort_by_key(|(_, definition)| definition.order);

    // D3: the definition change reaches every reference explicitly. No display
    // cache is allowed to carry the old value.
    let reference_ids = references(&candidate, block);
    let definition_layers: Vec<LayerId> = new_definitions
        .iter()
        .map(|(id, _)| {
            candidate
                .entity(*id)
                .map(|entity| entity.layer)
                .unwrap_or(session.active_layer)
        })
        .collect();
    let mut synced = 0usize;
    let mut dropped = 0usize;
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
            .filter(|value| {
                !new_definitions
                    .iter()
                    .any(|(_, definition)| tags_equal(&definition.tag, &value.tag))
            })
            .count();
        let next = sync_values(&new_definitions, &definition_layers, &attributes);
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
        synced += 1;
    }

    let block_changes = if let Some(member) = added_member {
        let after = block_row_with_member(&candidate, block, Some(member))?;
        (
            vec![(block, Some(after.clone()))],
            vec![BlockPatch {
                id: block,
                before: Some(block_row_with_member(&candidate, block, None)?),
                after: Some(after),
            }],
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let mut diagnostics = vec![Diagnostic {
        object: definition_entity.map(|id| ObjectId(id.0)),
        code: "attredef.redefined".into(),
        message: format!(
            "ATTREDEF: attribute '{}' in block '{}' is {}{}; {synced} reference(s) had their \
             values written explicitly",
            payload.tag.trim(),
            payload.block,
            if existing.is_some() {
                "redefined"
            } else {
                "added"
            },
            format_order(order),
        ),
    }];
    if dropped > 0 {
        diagnostics.push(Diagnostic {
            object: None,
            code: "attredef.unmatched_value".into(),
            message: format!(
                "ATTREDEF: dropped {dropped} stored value(s) whose tag no current definition \
                 declares, so no reference retains a stale value"
            ),
        });
    }
    commit_attribute_write(
        app,
        command,
        candidate,
        "redefine attribute",
        block_changes.0,
        block_changes.1,
        changed,
        before,
        diagnostics,
    )
}

fn format_order(order: u32) -> String {
    format!(" at order {order}")
}
