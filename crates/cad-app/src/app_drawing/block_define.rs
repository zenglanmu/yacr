//! `BLOCK`: define a named block from the current selection at a picked base.
//!
//! The capture is the selection snapshot taken at tool start
//! ([`crate::draw_tool::DrawToolKind::BlockDefine`]), one free base point and the
//! terminal text channel's block name. This command writes **both** tables — the
//! block definition plus its member entities — through the database's single
//! combined write path, so the definition, the member space changes and the
//! reference insert are one revision and one undo step.
//!
//! ## Members keep their geometry
//!
//! The selected entities are not translated. Each member's stored model-space
//! geometry is copied **bit-for-bit** and only its `space` becomes
//! [`SpaceId::Block`]`(new_id)`. The definition stores the picked `base` as its
//! [`BlockMetadata::base_point`], and the new reference uses the same
//! `T(base) · T(−base)` composition as `INSERT` (the identity), so the picked
//! base maps to itself. A later `INSERT` at `I` composes `T(I) · T(−base)`, which
//! is what places the block's base at `I`; translating the members by `−base`
//! here would double-subtract.
//!
//! ## Refusals (all before any write)
//!
//! | code | meaning |
//! | --- | --- |
//! | `block.selection_empty` | `refs` is empty |
//! | `block.invalid_point` | the base point is non-finite |
//! | `block.invalid_name` | the trimmed name is empty, multi-line, over 1024 characters, or contains an AutoCAD-illegal character |
//! | `block.duplicate_name` | an exact or ASCII-case-insensitive block of that name already exists |
//! | `block.not_addressable` | a ref has an instance path or sub-element |
//! | `block.target_missing` | a ref is another document's or does not resolve |
//! | `block.not_model_space` | a resolved ref is paper space or a block member |
//!
//! A locked member layer or a locked active layer is refused by the stager's own
//! `layer.locked` chokepoint before anything is committed.
//!
//! ## Result
//!
//! On success the session selection is cleared (the selected ids are block
//! members now) and a `drawing.block` diagnostic names the definition, the name,
//! the member count and the inserted reference. Work-only.

use super::*;
use cad_db::{BlockDefinition, BlockMetadata, BlockPatch, BlockRow};
use cad_history::{DrawingPatch, DrawingUndoRecord};

/// The characters AutoCAD forbids in a block name.
///
/// Stated here as one list so the refusal is a named `block.invalid_name` rather
/// than a database invariant. The same punctuation is rejected for layer names
/// by the layer-table writer.
const ILLEGAL_BLOCK_NAME_CHARS: &[char] = &[
    '<', '>', '/', '\\', '"', ':', ';', '?', '*', '|', '=', '`', '\'',
];

/// A `BLOCK` refusal: `InvalidInput` carrying its stable code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// Define a named block from the selection snapshot at the picked base point.
    ///
    /// One transaction / one undo step; every refusal leaves the drawing and the
    /// block table untouched and creates no history entry.
    pub(crate) fn define_block(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::DefineBlock { refs, base, name } = &command.payload else {
            return Err(CadError::InvalidInput(
                "BLOCK needs a DefineBlock { refs, base, name } payload".into(),
            ));
        };
        // 1. The selection snapshot and the picked base point.
        if refs.is_empty() {
            return Err(refusal(
                "block.selection_empty",
                "BLOCK needs at least one selected entity to become a block member",
            ));
        }
        if !is_finite_point(*base) {
            return Err(refusal(
                "block.invalid_point",
                "BLOCK base point must be finite",
            ));
        }
        // 2. The name grammar. The duplicate rule is checked against the
        //    authoritative drawing below, after this cheap syntax pass.
        let name = name.trim();
        if name.is_empty() {
            return Err(refusal(
                "block.invalid_name",
                "BLOCK needs a non-empty, non-whitespace block name",
            ));
        }
        if name.contains('\r') || name.contains('\n') {
            return Err(refusal(
                "block.invalid_name",
                "BLOCK block name must be a single line",
            ));
        }
        let characters = name.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(refusal(
                "block.invalid_name",
                format!("BLOCK allows at most {MAX_TOOL_TEXT_CHARS} characters, got {characters}"),
            ));
        }
        if let Some(illegal) = name
            .chars()
            .find(|character| ILLEGAL_BLOCK_NAME_CHARS.contains(character))
        {
            return Err(refusal(
                "block.invalid_name",
                format!("BLOCK block name must not contain '{illegal}'"),
            ));
        }

        let document_id = command.document;
        // 3. Resolve and validate every ref against the authoritative drawing as
        //    plain data, and reject a duplicate name before any transaction.
        let sources: Vec<DbEntity> = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            if drawing.block_by_name(name).is_some() {
                return Err(refusal(
                    "block.duplicate_name",
                    format!(
                        "this drawing already has a block named '{name}'; names are unique \
                         case-insensitively"
                    ),
                ));
            }
            let mut seen = std::collections::BTreeSet::new();
            let mut sources = Vec::with_capacity(refs.len());
            for reference in refs {
                if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                    return Err(refusal(
                        "block.not_addressable",
                        format!(
                            "BLOCK target {} is a block-instance member or sub-element; only \
                             directly addressable model-space entities can become members",
                            reference.entity.0
                        ),
                    ));
                }
                if reference.document != document_id {
                    return Err(refusal(
                        "block.target_missing",
                        format!(
                            "BLOCK target {} belongs to another document",
                            reference.entity.0
                        ),
                    ));
                }
                let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                    refusal(
                        "block.target_missing",
                        format!("BLOCK target {} does not exist", reference.entity.0),
                    )
                })?;
                if entity.space != SpaceId::Model {
                    return Err(refusal(
                        "block.not_model_space",
                        format!(
                            "BLOCK target {} is not a model-space entity; paper space and \
                             block members cannot become members of a new block",
                            entity.id.0
                        ),
                    ));
                }
                // A repeated ref would stage the same member twice, which the
                // database refuses; collapse it here instead of failing late.
                if seen.insert(entity.id) {
                    sources.push(entity);
                }
            }
            // Ascending draw order, with the id as the stable tie-break so the
            // stored member list matches the database's own ordering rule.
            sources.sort_by_key(|entity| (entity.draw_order, entity.id.0));
            sources
        };

        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();

        // 4. The definition id and every member row. Geometry is untouched: only
        //    `space` changes, so a member's world position is exactly its stored
        //    model-space position.
        let new_block = candidate.allocate_block_id();
        let member_ids: Vec<EntityId> = sources.iter().map(|entity| entity.id).collect();
        let members: Vec<DbEntity> = sources
            .iter()
            .map(|before| {
                let mut after = before.clone();
                after.space = SpaceId::Block(new_block);
                after
            })
            .collect();
        let member_draw_order_max = sources
            .iter()
            .map(|entity| entity.draw_order)
            .max()
            .unwrap_or(0);
        let before_displays: Vec<cad_db::EntityDisplayState> = sources
            .iter()
            .map(|entity| candidate.entity_display_state(entity.id))
            .collect();

        // 5. The reference: same composition `INSERT` uses, with
        //    `insertion = base`, `base_point = base`, i.e. the identity.
        let to_origin = Transform3::translation(Point3 {
            x: -base.x,
            y: -base.y,
            z: -base.z,
        });
        let transform = Transform3::translation(*base).matrix_mul(&to_origin);
        let new_ref = candidate.allocate_entity_id();
        let reference = DbEntity {
            object: cad_db::DbObject {
                id: ObjectId(new_ref.0),
                type_key: "AcDbBlockReference".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: new_ref,
            layer: session.active_layer,
            space: SpaceId::Model,
            geometry: SemanticGeometry::Insert {
                block: new_block,
                transform,
            },
            // The highest member order: an unselected entity with a higher draw
            // order still draws above the new reference.
            draw_order: member_draw_order_max,
        };

        let block_row = BlockRow {
            definition: BlockDefinition {
                id: new_block,
                entities: member_ids.clone(),
                dynamic_visibility: None,
            },
            metadata: Some(BlockMetadata {
                name: name.to_string(),
                base_point: *base,
            }),
        };

        // 6. One transaction for the definition, the member space changes and the
        //    reference insert. A locked member/active layer is refused by the
        //    stager before anything is committed.
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("define block", transaction)?;
            staged.stage_create_block(block_row.clone())?;
            for member in &members {
                staged.update_entity(member.clone())?;
            }
            staged.insert_entity(reference.clone())?;
            staged.commit()?
        };

        // 7. Read every committed row back so the patches carry the exact stored
        //    value, never a re-derivation.
        let mut patches: Vec<DrawingPatch> = Vec::with_capacity(members.len() + 1);
        for (index, before) in sources.iter().enumerate() {
            let after = candidate
                .entity(before.id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("block member vanished after commit".into()))?;
            patches.push(DrawingPatch {
                id: before.id,
                before: Some(before.clone()),
                after: Some(after),
                before_state: Some(before_displays[index].clone()),
                after_state: Some(candidate.entity_display_state(before.id)),
            });
        }
        let committed_reference = candidate
            .entity(new_ref)
            .cloned()
            .ok_or_else(|| CadError::Invariant("block reference vanished after commit".into()))?;
        patches.push(DrawingPatch {
            id: new_ref,
            before: None,
            after: Some(committed_reference.clone()),
            before_state: None,
            after_state: Some(candidate.entity_display_state(new_ref)),
        });
        let member_count = member_ids.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "define block".into(),
                patches,
                block_patches: vec![BlockPatch {
                    id: new_block,
                    before: None,
                    after: Some(block_row),
                }],
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        // 8. The selected ids are now block members, not selectable model-space
        //    objects; clear the selection exactly like `select_all` replaces it.
        session.selection = SelectionSet::new();

        Ok(CommandOutcome {
            objects: vec![ObjectId(new_ref.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(new_ref.0)),
                code: "drawing.block".into(),
                message: format!(
                    "BLOCK: defined block {} '{}' with {member_count} member(s); reference {} \
                     at ({}, {}, {})",
                    new_block.0, name, new_ref.0, base.x, base.y, base.z
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
