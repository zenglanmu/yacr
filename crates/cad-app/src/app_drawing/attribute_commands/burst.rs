//! `BURST`: explode one block level **and** convert its attribute values to
//! `TEXT`.
//!
//! `EXPLODE` deliberately drops `INSERT` attribute values (it has no preserving
//! product for them); `BURST` is the command whose whole point is to keep them.
//! The decomposition is exactly [`crate::app_drawing::explode`]/`EXPLODE`'s
//! one-level walk — members are transformed by the reference's placement, a
//! nested reference stays a reference and definitions are not drawn twice — with
//! one addition: every stored value (constant values included) becomes a real
//! `AcDbText` at the value's block-local placement, transformed into the
//! reference's space.
//!
//! The whole command is one transaction / one undo step. A selected non-INSERT,
//! an XREF reference, a non-uniform placement whose exact image is not
//! representable, or a value that cannot be transformed refuses the command
//! before anything is written.

use super::super::explode::affine_conic_image;
use super::*;
use cad_db::{EntityDisplayState, EntityRenderAttributes};
use cad_history::{drawing_patch, drawing_patch_deleted, DrawingPatch, DrawingUndoRecord};

const INSERT_SIMILARITY_TOLERANCE: f64 = 1e-9;

/// One created entity before it gets a fresh id.
struct Product {
    layer: LayerId,
    space: SpaceId,
    type_key: String,
    geometry: SemanticGeometry,
    /// The converted attribute value's own appearance, when this product came
    /// from a value (a block member keeps the ordinary entity display state).
    display: Option<EntityDisplayState>,
}

/// BURST the current selection.
pub(super) fn burst(
    app: &mut Application,
    session: &SessionState,
    command: &Command,
) -> CadResult<CommandOutcome> {
    if !matches!(command.payload, CommandPayload::None) {
        return Err(refusal(
            "burst_payload",
            "BURST takes no payload; the current selection is the input",
        ));
    }
    let refs = session.selection.refs().to_vec();
    if refs.is_empty() {
        return Err(refusal(
            "selection_empty",
            "BURST needs at least one selected block reference",
        ));
    }
    let (_, mut candidate) = drawing_of(app, command)?;
    let mut sources = Vec::new();
    for reference in &refs {
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
                    "entity {} is a block-instance member or sub-element; BURST needs a direct \
                     block reference",
                    reference.entity.0
                ),
            ));
        }
        let entity = candidate.entity(reference.entity).cloned().ok_or_else(|| {
            refusal(
                "target_missing",
                format!("entity {} does not exist", reference.entity.0),
            )
        })?;
        if !matches!(entity.geometry, SemanticGeometry::Insert { .. }) {
            return Err(refusal(
                "target_not_insert",
                format!(
                    "entity {} is {}; BURST explodes block references only",
                    entity.id.0,
                    class_name(&entity.geometry)
                ),
            ));
        }
        if !sources
            .iter()
            .any(|source: &DbEntity| source.id == entity.id)
        {
            sources.push(entity);
        }
    }

    let display_states: Vec<EntityDisplayState> = sources
        .iter()
        .map(|source| candidate.entity_display_state(source.id))
        .collect();
    let mut products: Vec<Product> = Vec::new();
    let mut diagnostics = Vec::new();
    for source in &sources {
        let SemanticGeometry::Insert {
            block,
            transform,
            attributes,
        } = &source.geometry
        else {
            continue;
        };
        burst_insert(
            &candidate,
            &source.space,
            *block,
            transform,
            attributes,
            &mut products,
            &mut diagnostics,
        )?;
    }
    if products.is_empty() {
        return Err(refusal(
            "nothing_produced",
            "BURST produced no geometry from the selection",
        ));
    }

    let base_order = candidate
        .model_space()
        .iter()
        .map(|entity| entity.draw_order)
        .max()
        .map(|max| max.saturating_add(1))
        .unwrap_or(0);
    let mut children: Vec<(EntityId, DbEntity, Option<EntityDisplayState>)> = Vec::new();
    for (index, product) in products.into_iter().enumerate() {
        let id = candidate.allocate_entity_id();
        let entity = DbEntity {
            object: cad_db::DbObject {
                id: ObjectId(id.0),
                type_key: product.type_key,
                revision: Revision(0),
                source_handle: None,
            },
            id,
            layer: product.layer,
            space: product.space,
            geometry: product.geometry,
            draw_order: base_order + index as i64,
        };
        children.push((id, entity, product.display));
    }

    let transaction = next_transaction_id();
    let change_set = {
        let mut staged =
            candidate.begin_drawing_transaction("burst block reference", transaction)?;
        for source in &sources {
            staged.delete_entity(source.id)?;
        }
        for (id, child, display) in &children {
            staged.insert_entity(child.clone())?;
            if let Some(state) = display {
                staged.update_entity_display_state(*id, state)?;
            }
        }
        staged.commit()?
    };

    let mut patches: Vec<DrawingPatch> = sources
        .iter()
        .zip(display_states)
        .map(|(source, state)| drawing_patch_deleted(source.id, source.clone(), state))
        .collect();
    for (id, child, _) in &children {
        let mut patch = drawing_patch(*id, None, Some(child.clone()));
        patch.after_state = Some(candidate.entity_display_state(*id));
        patches.push(patch);
    }
    diagnostics.push(Diagnostic {
        object: None,
        code: "burst.exploded".into(),
        message: format!(
            "BURST: {} entity row(s) produced from {} block reference(s); attribute values were \
             converted to TEXT (constant values included) and the sources were deleted",
            children.len(),
            sources.len()
        ),
    });
    app.publish_candidate(
        command.document,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: "burst block reference".into(),
            patches,
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            style_patches: Vec::new(),
            dim_style_patches: Vec::new(),
            mleader_style_patches: Vec::new(),
            layout_patches: Vec::new(),
            merge_key: None,
        },
    )?;
    Ok(CommandOutcome {
        objects: children.iter().map(|(id, _, _)| ObjectId(id.0)).collect(),
        changes: Some(Arc::new(change_set)),
        diagnostics,
        measurement: None,
        point_report: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn burst_insert(
    drawing: &DrawingDatabase,
    space: &SpaceId,
    block: BlockId,
    transform: &Transform3,
    attributes: &[AttributeValue],
    out: &mut Vec<Product>,
    diagnostics: &mut Vec<Diagnostic>,
) -> CadResult<()> {
    if drawing.is_xref_block(block) {
        return Err(unsupported(
            "xref",
            "BURST refuses an XREF block reference: an external reference cannot be exploded",
        ));
    }
    let definition = drawing.block(block).ok_or_else(|| {
        unsupported(
            "unknown_block",
            format!("block {} is not in the database", block.0),
        )
    })?;
    let members: Vec<EntityId> = match drawing.block_visible_entities(block) {
        Some(visible) => visible,
        None => definition.entities.clone(),
    };
    let uniform = transform.is_uniform_scale(INSERT_SIMILARITY_TOLERANCE);
    for member in &members {
        let child = drawing.entity(*member).ok_or_else(|| {
            unsupported(
                "unknown_member",
                format!("block member {} is not in the database", member.0),
            )
        })?;
        // The definition itself is a template; the reference's value list is the
        // concrete text. Drawing the definition too would double every value.
        if matches!(child.geometry, SemanticGeometry::AttributeDefinition(_)) {
            continue;
        }
        if child.object.type_key == "AcDbMultiLeader" {
            return Err(unsupported(
                "member_multileader",
                format!(
                    "BURST refuses the in-block {} by type_key, exactly like EXPLODE",
                    child.object.type_key
                ),
            ));
        }
        match &child.geometry {
            SemanticGeometry::Insert {
                block: nested,
                transform: nested_transform,
                attributes: nested_attributes,
            } => {
                out.push(Product {
                    layer: child.layer,
                    space: space.clone(),
                    type_key: "AcDbBlockReference".into(),
                    geometry: SemanticGeometry::Insert {
                        block: *nested,
                        transform: transform.matrix_mul(nested_transform),
                        attributes: nested_attributes.clone(),
                    },
                    display: None,
                });
            }
            other => {
                let placed = match transform_geometry(other, transform) {
                    Ok(placed) => placed,
                    Err(CadError::Unsupported(_)) if !uniform => {
                        affine_conic_image(other, transform).ok_or_else(|| {
                            unsupported(
                                "member_not_placeable",
                                format!(
                                    "block member {} ({}) has no exact image under the \
                                     non-uniform placement",
                                    member.0,
                                    geometry_kind(other)
                                ),
                            )
                        })?
                    }
                    Err(error) => {
                        return Err(unsupported(
                            "member_not_placeable",
                            format!("cannot place block member {}: {error}", member.0),
                        ))
                    }
                };
                out.push(Product {
                    layer: child.layer,
                    space: space.clone(),
                    type_key: child.object.type_key.clone(),
                    geometry: placed,
                    display: None,
                });
            }
        }
    }

    // Convert this reference's own values. Definitions were skipped above, so
    // every text product here is exactly one stored value.
    for value in attributes {
        if value.value.is_empty() {
            diagnostics.push(Diagnostic {
                object: None,
                code: "burst.empty_value".into(),
                message: format!(
                    "BURST: attribute '{}' has an empty value; no TEXT was produced for it",
                    value.tag
                ),
            });
            continue;
        }
        let local = value.text.as_text(value.value.clone());
        let placed = match transform_geometry(&local, transform) {
            Ok(placed) => placed,
            Err(CadError::Unsupported(_)) if !uniform => affine_conic_image(&local, transform)
                .ok_or_else(|| {
                    unsupported(
                        "attribute_not_placeable",
                        format!(
                            "attribute '{}' has no exact text image under the non-uniform \
                             placement",
                            value.tag
                        ),
                    )
                })?,
            Err(error) => {
                return Err(unsupported(
                    "attribute_not_placeable",
                    format!("cannot place attribute '{}': {error}", value.tag),
                ))
            }
        };
        out.push(Product {
            layer: value.layer,
            space: space.clone(),
            type_key: "AcDbText".into(),
            geometry: placed,
            display: Some(EntityDisplayState {
                attributes: EntityRenderAttributes {
                    transparency: value.appearance.transparency,
                    color: value.appearance.color,
                    lineweight: value.appearance.lineweight,
                    linetype: value.appearance.linetype.clone(),
                    linetype_scale: value.appearance.linetype_scale,
                    ..EntityRenderAttributes::default()
                },
                visibility: Default::default(),
            }),
        });
    }
    Ok(())
}
