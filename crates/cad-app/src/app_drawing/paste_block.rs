//! `PASTEBLOCK`: turn the in-process clipboard into a new **auto-named block
//! definition** plus one reference at the picked insertion point.
//!
//! This is the block-shaped sibling of [`super::clipboard`]'s `PASTECLIP`: the
//! clipboard is still the input and is **not** cleared, but instead of inserting
//! loose copies the entries become the members of a fresh definition
//! ([`cad_db::BlockDefinition`] + [`cad_db::BlockMetadata`]) and exactly one
//! reference is placed in model space. The definition, the member inserts and
//! the reference insert commit through the database's single combined block +
//! entity batch, so they are one revision and one undo step.
//!
//! ## Names
//!
//! The definition is auto-named `PasteBlock1`, `PasteBlock2`, … — the first
//! suffix whose name does not resolve in the drawing (`block_by_name`, exact then
//! ASCII case-insensitive, exactly like `BLOCK`'s duplicate rule). This build
//! deliberately does **not** use AutoCAD's anonymous `*U…` spelling: the name is
//! stable, inspectable and `INSERT`-addressable, and the 10_000-suffix cap keeps
//! the scan bounded (`pasteblock.name_exhausted`). The name is otherwise not user
//! input, so it needs no name-grammar pass.
//!
//! ## Geometry
//!
//! Member geometry is copied **bit-for-bit** and only its `space` becomes
//! [`SpaceId::Block`]`(new_block)` — the same rule `BLOCK` uses. The stored base
//! point is the clipboard base (the world origin when the clipboard has none), and
//! the reference carries `T(insertion) · T(−base)`, the one translation rule
//! `PASTECLIP`/`INSERT` use, so the base lands on the picked point.
//!
//! ## Refusals (all before any write, never a partial paste)
//!
//! | code | meaning |
//! | --- | --- |
//! | `pasteblock.invalid_point` | the insertion point, the clipboard base or the translation is non-finite |
//! | `pasteblock.empty_clipboard` | the clipboard holds no entries |
//! | `pasteblock.not_model_space` | an entry is paper space or a block member |
//! | `pasteblock.missing_block` | a nested `INSERT` entry references a block that does not exist |
//! | `pasteblock.name_exhausted` | `PasteBlock1`…`PasteBlock10000` are all taken |
//!
//! A locked member/active layer is refused by the stager's own `layer.locked`
//! chokepoint. Viewer mode is refused by the command's Work-only authorization.
//!
//! ## Result
//!
//! The success diagnostic is `drawing.pasteblock`; the session selection becomes
//! exactly the new reference, and the clipboard is left untouched.

use super::*;
use cad_db::{BlockDefinition, BlockMetadata, BlockPatch, BlockRow};
use cad_history::{DrawingPatch, DrawingUndoRecord};

/// The first `PasteBlockN` suffix tried, and the hard cap on the scan.
///
/// A bound keeps a hostile table from turning name generation into an unbounded
/// loop; reaching it is `pasteblock.name_exhausted`, never a reused name.
const MAX_PASTEBLOCK_SUFFIX: usize = 10_000;

/// A `PASTEBLOCK` refusal: `InvalidInput` carrying its stable code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// Paste the clipboard as a new auto-named block plus one reference.
    ///
    /// One transaction / one undo step; every refusal leaves the drawing, the
    /// block table and the history untouched and the clipboard intact.
    pub(crate) fn paste_block(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::PasteBlock { insertion } = command.payload else {
            return Err(CadError::InvalidInput(
                "PASTEBLOCK needs a PasteBlock { insertion } payload".into(),
            ));
        };
        // 1. The insertion point and the one translation rule.
        if !is_finite_point(insertion) {
            return Err(refusal(
                "pasteblock.invalid_point",
                "PASTEBLOCK insertion point must be finite",
            ));
        }
        if session.clipboard.is_empty() {
            return Err(refusal(
                "pasteblock.empty_clipboard",
                "PASTEBLOCK needs a non-empty clipboard",
            ));
        }
        let base = session.clipboard_base.unwrap_or(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        if !is_finite_point(base) {
            return Err(refusal(
                "pasteblock.invalid_point",
                "PASTEBLOCK clipboard base point must be finite",
            ));
        }
        let translation = Point3 {
            x: insertion.x - base.x,
            y: insertion.y - base.y,
            z: insertion.z - base.z,
        };
        if !is_finite_point(translation) {
            return Err(refusal(
                "pasteblock.invalid_point",
                "PASTEBLOCK translation must be finite",
            ));
        }

        let document_id = command.document;
        // 2. Validate every entry and generate the name against the authoritative
        //    drawing, before the candidate clone or any transaction.
        let name = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            for entry in &session.clipboard {
                // Model space only: paper space and block members cannot become
                // members of a new definition.
                if entry.entity.space != SpaceId::Model {
                    return Err(refusal(
                        "pasteblock.not_model_space",
                        format!(
                            "PASTEBLOCK entry {} is not a model-space entity; paper space and \
                             block members cannot become members of a new block",
                            entry.entity.id.0
                        ),
                    ));
                }
                // A nested reference is allowed, but only to a block that exists.
                if let SemanticGeometry::Insert { block, .. } = &entry.entity.geometry {
                    if drawing.block(*block).is_none() {
                        return Err(refusal(
                            "pasteblock.missing_block",
                            format!(
                                "PASTEBLOCK entry {} references block {:?}, which does not exist \
                                 in this drawing",
                                entry.entity.id.0, block
                            ),
                        ));
                    }
                }
            }
            let mut suffix = 1usize;
            loop {
                let candidate = format!("PasteBlock{suffix}");
                if drawing.block_by_name(&candidate).is_none() {
                    break candidate;
                }
                if suffix >= MAX_PASTEBLOCK_SUFFIX {
                    return Err(refusal(
                        "pasteblock.name_exhausted",
                        format!(
                            "PASTEBLOCK could not find a free name among PasteBlock1..PasteBlock{MAX_PASTEBLOCK_SUFFIX}"
                        ),
                    ));
                }
                suffix += 1;
            }
        };

        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();

        // 3. The definition id, the fresh member ids and the reference id. Every
        //    allocation happens before the transaction borrows the candidate.
        let new_block = candidate.allocate_block_id();
        let mut members: Vec<(EntityId, DbEntity, cad_db::EntityDisplayState)> =
            Vec::with_capacity(session.clipboard.len());
        for entry in &session.clipboard {
            let source = &entry.entity;
            let new_id = candidate.allocate_entity_id();
            let member = DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(new_id.0),
                    type_key: source.object.type_key.clone(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: new_id,
                // Layer and geometry are preserved exactly; a nested INSERT keeps
                // its block reference untouched.
                layer: source.layer,
                space: SpaceId::Block(new_block),
                geometry: source.geometry.clone(),
                draw_order: source.draw_order,
            };
            members.push((new_id, member, entry.display.clone()));
        }
        // The stored member list is ordered by draw order (the database's own
        // membership rule), with the id as the stable tie-break.
        members.sort_by_key(|(id, member, _)| (member.draw_order, id.0));
        let member_ids: Vec<EntityId> = members.iter().map(|(id, _, _)| *id).collect();
        let member_max_order = members
            .iter()
            .map(|(_, member, _)| member.draw_order)
            .max()
            .unwrap_or(0);

        let new_ref = candidate.allocate_entity_id();
        let to_origin = Transform3::translation(Point3 {
            x: -base.x,
            y: -base.y,
            z: -base.z,
        });
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
                transform: Transform3::translation(insertion).matrix_mul(&to_origin),
            },
            draw_order: member_max_order,
        };
        let block_row = BlockRow {
            definition: BlockDefinition {
                id: new_block,
                entities: member_ids,
                dynamic_visibility: None,
            },
            metadata: Some(BlockMetadata {
                name: name.clone(),
                base_point: base,
            }),
        };

        // 4. One transaction for the definition row, the member inserts and the
        //    reference insert.
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("paste block", transaction)?;
            for (_, member, _) in &members {
                staged.insert_entity(member.clone())?;
            }
            staged.stage_create_block(block_row.clone())?;
            staged.insert_entity(reference.clone())?;
            staged.commit()?
        };

        // 5. The transaction cannot stage display state for a row that did not
        //    exist yet, so restore each member's captured display after the commit
        //    (the BREAK/PASTECLIP precedent) and record it on the patch so redo
        //    re-applies it too.
        let mut patches: Vec<DrawingPatch> = Vec::with_capacity(members.len() + 1);
        for (id, _, display) in &members {
            candidate.restore_entity_display_state(*id, display);
            let after = candidate
                .entity(*id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("pasted block member vanished".into()))?;
            patches.push(DrawingPatch {
                id: *id,
                before: None,
                after: Some(after),
                before_state: None,
                after_state: Some(display.clone()),
            });
        }
        let committed_reference = candidate
            .entity(new_ref)
            .cloned()
            .ok_or_else(|| CadError::Invariant("pasted block reference vanished".into()))?;
        patches.push(DrawingPatch {
            id: new_ref,
            before: None,
            after: Some(committed_reference),
            before_state: None,
            // The reference has no captured display: nothing to restore.
            after_state: None,
        });
        // Read the committed definition back so the block patch carries exactly
        // the stored row.
        let committed_row = BlockRow {
            definition: candidate
                .block(new_block)
                .cloned()
                .ok_or_else(|| CadError::Invariant("pasted block definition vanished".into()))?,
            metadata: candidate.block_metadata(new_block).cloned(),
        };
        let member_count = patches.len() - 1;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "paste block".into(),
                patches,
                block_patches: vec![BlockPatch {
                    id: new_block,
                    before: None,
                    after: Some(committed_row),
                }],
                merge_key: None,
            },
        )?;

        // 6. The new reference is the only new model-space object; select it.
        session.selection = SelectionSet::from_refs([SelectionRef {
            document: document_id,
            entity: new_ref,
            instance: InstancePath(Vec::new()),
            sub_element: None,
        }]);

        Ok(CommandOutcome {
            objects: vec![ObjectId(new_ref.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(new_ref.0)),
                code: "drawing.pasteblock".into(),
                message: format!(
                    "PASTEBLOCK: defined block {} '{}' with {member_count} member(s); reference {} \
                     at ({}, {}, {})",
                    new_block.0, name, new_ref.0, insertion.x, insertion.y, insertion.z
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
