//! `REFEDIT` / `-REFEDIT` / `REFSET` / `REFCLOSE`: the bounded 2D in-place
//! block-reference edit session.
//!
//! # What a session is
//!
//! `REFEDIT` takes exactly one selected, directly addressable **model-space**
//! `INSERT` whose placement is a similarity (uniform scale `> 0`, rotation,
//! translation; positive determinant). Every visible member of that INSERT's
//! block definition is cloned into model space with the placement transform
//! baked in (fresh ids, the member's layer and render attributes preserved) in
//! **one transaction / one undo step**, and the source INSERT is hidden through
//! the existing session hidden set so it is not drawn twice. The selection
//! becomes the extracted clones and the session records the clone → member map.
//!
//! `REFSET ADD|REMOVE` (bare `REFSET` = `ADD`) only edits the session working
//! set: no drawing write, no revision, no history. `ADD` may add plain host
//! objects; on save they move into the block definition. `REMOVE` drops an entry
//! from the working set: its original member is deleted from the block on save
//! while the model-space copy stays as an ordinary host object.
//!
//! `REFCLOSE SAVE|DISCARD` closes the session:
//!
//! * `SAVE` inverse-transforms every working clone back into block coordinates
//!   and writes the **one** block row plus the member updates/inserts/deletes and
//!   the clone deletions through
//!   [`cad_db::DrawingDatabase::apply_drawing_changes_with_blocks`] — the single
//!   block-table authority — as one transaction / one undo step. The source
//!   INSERT is unhidden again.
//! * `DISCARD` deletes every extracted clone (one transaction) and leaves the
//!   block definition untouched.
//!
//! # Boundaries (v1, declared)
//!
//! * One source INSERT, one block definition. A second `REFEDIT` while active is
//!   `refedit.active`.
//! * Only **visible** members are extracted (the `NCOPY`/`EXPLODE` rule); a
//!   member hidden by an active dynamic-visibility state has no clone and is left
//!   untouched by a save.
//! * **No reference fade / no reference lock**: this build has no such display
//!   concept, so other host objects stay visible and selectable during a session.
//!   `REFSET ADD` is the explicit way to pull one into the working set.
//! * Members are edited only along their clones' **geometry**; a clone's layer /
//!   draw order are not written back to the member. A nested INSERT member keeps
//!   its own block reference and gets the inverse placement composed onto its
//!   transform, exactly like the extraction composes the forward one.
//! * Only the 2D geometry kinds `cad_db::transform_geometry` can represent under
//!   a similarity are extractable; anything else is `refedit.clone_failed`
//!   **before** any write (all-or-nothing).
//! * Closing the document (a host session rebuild) drops the session; the
//!   extracted clones are not deleted by this module in that case.
//! * Undo/redo reconciles a session whose extracted clones all vanished (for
//!   example undoing the `REFEDIT` extraction): the stale session is dropped and
//!   the source INSERT unhidden. The session itself is not undoable state, so an
//!   undo never *restores* an active session.
//!
//! # Refusals (all before any write; no revision, no history)
//!
//! | code | meaning |
//! | --- | --- |
//! | `refedit.active` | a session is already active |
//! | `refedit.selection` | not exactly one directly addressable model-space entity |
//! | `refedit.not_insert` | the selected entity is not an `INSERT` |
//! | `refedit.non_uniform` | the placement is mirrored, non-uniform, degenerate or non-finite |
//! | `refedit.block_missing` | the INSERT names a block this drawing does not have |
//! | `refedit.clone_failed` | a member cannot be represented under the similarity |
//! | `refedit.no_session` | `REFSET`/`REFCLOSE` with no active session |
//! | `refedit.selection_empty` | `REFSET` with an empty selection |
//! | `refset.invalid_option` | `REFSET` with a non-option payload |
//! | `refclose.needs_option` | bare `REFCLOSE` |
//! | `refclose.invalid_option` | `REFCLOSE` with a non-option payload |
//! | `refedit.save_blocked` | `SAVE` requested while a session is active (session guard) |
//!
//! `REFSET ADD|REMOVE` / `REFCLOSE SAVE|DISCARD` are typed argument forms of the
//! two selectors; an option outside those grammars is refused by the session
//! argument layer (`refset.invalid_option` / `refclose.invalid_option`).

use super::*;
use crate::app_drawing::{class_name, next_transaction_id};
use cad_db::{
    transform_geometry, BlockDefinition, BlockPatch, BlockRow, DbEntity, EntityDisplayState,
    EntityRenderAttributes,
};
use cad_history::{DrawingPatch, DrawingUndoRecord};
use std::collections::{BTreeMap, BTreeSet};

/// Relative tolerance for the placement's similarity test.
const SIMILARITY_TOLERANCE: f64 = 1e-9;

fn invalid(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{prefix}.{code}: {detail}"))
}

fn is_finite_transform(transform: &Transform3) -> bool {
    transform
        .matrix
        .iter()
        .flatten()
        .all(|value| value.is_finite())
}

impl Application {
    /// Route one of the four in-place block-edit selectors.
    pub(crate) fn refedit_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.id {
            CommandId::RefEdit | CommandId::DashRefEdit => self.refedit_begin(session, command),
            CommandId::RefSet => self.refset(session, command),
            CommandId::RefClose => self.refclose(session, command),
            other => Err(CadError::Invariant(format!(
                "{other:?} is not a REFEDIT command"
            ))),
        }
    }

    /// Drop a session the drawing no longer backs (undo/redo, or a host edit that
    /// removed every extracted clone) and unhide the source reference.
    ///
    /// The session is derived state, so this is a reconciliation, not a write:
    /// no transaction, no revision, no history entry. A session with at least one
    /// surviving clone stays active — the user may still save or discard it.
    pub(crate) fn reconcile_refedit(&self, session: &mut SessionState) {
        let Some(active) = session.refedit.active() else {
            return;
        };
        let Some(document) = self.workspace.documents.get(&session.document) else {
            return;
        };
        let stale = active
            .clone_of
            .keys()
            .all(|id| document.drawing.entity(*id).is_none());
        if !stale {
            return;
        }
        let source = active.source.clone();
        session.hidden_entities.remove(&source);
        session.refedit.clear();
    }

    /// `REFEDIT` / `-REFEDIT`: extract the members of one INSERT and start the
    /// session (one transaction / one undo step).
    fn refedit_begin(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none(command, "refedit")?;
        if session.refedit.is_active() {
            return Err(invalid(
                "refedit",
                "active",
                "an in-place block edit is already active; finish it with REFCLOSE SAVE|DISCARD",
            ));
        }
        let document_id = command.document;
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("refedit", "selection", "document not open"))?
            .drawing
            .as_ref()
            .clone();

        // --- exactly one directly addressable model-space entity ------------
        let refs = session.selection.refs();
        if refs.len() != 1 {
            return Err(invalid(
                "refedit",
                "selection",
                format!(
                    "REFEDIT needs exactly one selected model-space INSERT, got {}",
                    refs.len()
                ),
            ));
        }
        let reference = refs[0].clone();
        if reference.document != document_id
            || !reference.instance.0.is_empty()
            || reference.sub_element.is_some()
        {
            return Err(invalid(
                "refedit",
                "selection",
                format!(
                    "entity {} is not a directly addressable model-space object (block-instance \
                     member or sub-element)",
                    reference.entity.0
                ),
            ));
        }
        let source = candidate.entity(reference.entity).cloned().ok_or_else(|| {
            invalid(
                "refedit",
                "selection",
                format!("entity {} does not exist", reference.entity.0),
            )
        })?;
        if source.space != SpaceId::Model {
            return Err(invalid(
                "refedit",
                "selection",
                format!("entity {} is not in model space", source.id.0),
            ));
        }

        // --- the placement must be a positive-determinant similarity --------
        let SemanticGeometry::Insert { block, transform } = &source.geometry else {
            return Err(invalid(
                "refedit",
                "not_insert",
                format!(
                    "entity {} is {}; REFEDIT needs a block reference",
                    source.id.0,
                    class_name(&source.geometry)
                ),
            ));
        };
        let block = *block;
        let transform = *transform;
        let determinant = transform.determinant();
        if !is_finite_transform(&transform)
            || !transform.is_uniform_scale(SIMILARITY_TOLERANCE)
            || !determinant.is_finite()
            || determinant <= 0.0
        {
            return Err(invalid(
                "refedit",
                "non_uniform",
                "the block reference placement is not a positive-determinant similarity (uniform \
                 scale > 0, rotation, translation); a mirrored or non-uniformly scaled reference \
                 cannot be edited in place",
            ));
        }
        let definition = candidate.block(block).cloned().ok_or_else(|| {
            invalid(
                "refedit",
                "block_missing",
                format!("block {} is not in the database", block.0),
            )
        })?;
        let member_ids = match candidate.block_visible_entities(block) {
            Some(visible) => visible,
            None => definition.entities.clone(),
        };

        // --- extract every visible member into model space -------------------
        let base_order = candidate
            .entities()
            .filter(|entity| entity.space == SpaceId::Model)
            .map(|entity| entity.draw_order)
            .max()
            .map(|order| order.saturating_add(1))
            .unwrap_or(0);
        let mut clones: Vec<DbEntity> = Vec::with_capacity(member_ids.len());
        let mut display: Vec<(EntityId, EntityRenderAttributes)> =
            Vec::with_capacity(member_ids.len());
        let mut clone_of: BTreeMap<EntityId, EntityId> = BTreeMap::new();
        let mut members: Vec<RefEditMember> = Vec::with_capacity(member_ids.len());
        for (index, member) in member_ids.iter().enumerate() {
            let child = candidate.entity(*member).cloned().ok_or_else(|| {
                invalid(
                    "refedit",
                    "clone_failed",
                    format!("block member {} is not in the database", member.0),
                )
            })?;
            let geometry = place_in_model_space(&child.geometry, &transform)?;
            let type_key = class_name(&geometry).to_string();
            let attributes = candidate.entity_render_attributes(*member);
            let new_id = candidate.allocate_entity_id();
            let clone = DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(new_id.0),
                    type_key,
                    revision: Revision(0),
                    source_handle: None,
                },
                id: new_id,
                layer: child.layer,
                space: SpaceId::Model,
                geometry,
                draw_order: base_order + index as i64,
            };
            clone_of.insert(new_id, *member);
            members.push(RefEditMember {
                original: *member,
                display: candidate.entity_display_state(*member),
            });
            display.push((new_id, attributes));
            clones.push(clone);
        }
        if clones.is_empty() {
            return Err(invalid(
                "refedit",
                "clone_failed",
                format!(
                    "block {} has no visible member geometry to extract",
                    block.0
                ),
            ));
        }

        // --- one transaction: the clones and their display attributes -------
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("reference edit", transaction)?;
            for clone in &clones {
                staged.insert_entity(clone.clone())?;
            }
            for (id, attributes) in &display {
                staged.update_entity_display(*id, attributes.clone())?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = clones
            .iter()
            .map(|clone| DrawingPatch {
                id: clone.id,
                before: None,
                after: Some(clone.clone()),
                before_state: None,
                after_state: Some(candidate.entity_display_state(clone.id)),
            })
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "reference edit".into(),
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

        // --- publish the session state (never a second drawing authority) ---
        session.hidden_entities.insert(reference.clone());
        let working: BTreeSet<EntityId> = clone_of.keys().copied().collect();
        session.refedit.begin(RefEditActive {
            document: document_id,
            source: reference,
            block,
            insert_transform: transform,
            clone_of,
            working,
            members,
        });
        session.selection = SelectionSet::from_refs(clones.iter().map(|clone| SelectionRef {
            document: document_id,
            entity: clone.id,
            instance: InstancePath::default(),
            sub_element: None,
        }));

        let count = clones.len();
        Ok(CommandOutcome {
            objects: clones.iter().map(|clone| ObjectId(clone.id.0)).collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(source.id.0)),
                code: "refedit.begin".into(),
                message: format!(
                    "REFEDIT: extracted {count} member(s) of block {} into model space; the source \
                     reference {} is hidden until REFCLOSE",
                    block.0, source.id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Place one extracted member geometry in model space.
fn place_in_model_space(
    geometry: &SemanticGeometry,
    transform: &Transform3,
) -> CadResult<SemanticGeometry> {
    match geometry {
        // A nested INSERT is a placement, not baked geometry: compose the
        // source placement onto its own transform (the `NCOPY`/`EXPLODE` rule).
        SemanticGeometry::Insert {
            block,
            transform: nested,
        } => Ok(SemanticGeometry::Insert {
            block: *block,
            transform: transform.matrix_mul(nested),
        }),
        other => transform_geometry(other, transform).map_err(|error| {
            invalid(
                "refedit",
                "clone_failed",
                format!(
                    "block member geometry {} cannot be placed under this similarity: {error}",
                    class_name(other)
                ),
            )
        }),
    }
}

/// Inverse-transform one working clone back into block coordinates.
fn place_in_block_space(
    geometry: &SemanticGeometry,
    inverse: &Transform3,
) -> CadResult<SemanticGeometry> {
    match geometry {
        SemanticGeometry::Insert {
            block,
            transform: nested,
        } => Ok(SemanticGeometry::Insert {
            block: *block,
            transform: inverse.matrix_mul(nested),
        }),
        other => transform_geometry(other, inverse).map_err(|error| {
            invalid(
                "refedit",
                "clone_failed",
                format!(
                    "working geometry {} cannot be written back into the block: {error}",
                    class_name(other)
                ),
            )
        }),
    }
}

/// The exact inverse of a positive-determinant similarity.
///
/// The placement was validated at `REFEDIT` time; this recomputes the inverse so
/// a save never trusts a stale derivation. A non-similarity is
/// `refedit.non_uniform` (nothing is written).
fn similarity_inverse(transform: &Transform3) -> CadResult<Transform3> {
    let determinant = transform.determinant();
    if !is_finite_transform(transform)
        || !determinant.is_finite()
        || determinant.abs() <= f64::EPSILON
        || !transform.is_uniform_scale(SIMILARITY_TOLERANCE)
    {
        return Err(invalid(
            "refedit",
            "non_uniform",
            "the stored placement is not an invertible similarity",
        ));
    }
    let m = &transform.matrix;
    let a = [
        [m[0][0], m[0][1], m[0][2]],
        [m[1][0], m[1][1], m[1][2]],
        [m[2][0], m[2][1], m[2][2]],
    ];
    let cofactor = |row: usize, col: usize| -> f64 {
        let rows = [(row + 1) % 3, (row + 2) % 3];
        let cols = [(col + 1) % 3, (col + 2) % 3];
        a[rows[0]][cols[0]] * a[rows[1]][cols[1]] - a[rows[0]][cols[1]] * a[rows[1]][cols[0]]
    };
    let inv_det = 1.0 / determinant;
    // inv[i][j] = cofactor(j, i) / det (the inverse is the transposed adjugate).
    let mut inv = [[0.0f64; 3]; 3];
    for (i, row) in inv.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = cofactor(j, i) * inv_det;
        }
    }
    let translation = [m[0][3], m[1][3], m[2][3]];
    let mut matrix = [[0.0f64; 4]; 4];
    for (i, row) in inv.iter().enumerate() {
        for (j, value) in row.iter().enumerate() {
            matrix[i][j] = *value;
        }
        matrix[i][3] =
            -(row[0] * translation[0] + row[1] * translation[1] + row[2] * translation[2]);
    }
    matrix[3][3] = 1.0;
    Ok(Transform3 { matrix })
}

/// The block member list the batch leaves behind, in the database's own order.
///
/// The stored member list must be **exactly** the post-batch entities whose space
/// is `SpaceId::Block(block)`, ordered by `draw_order` with the ascending entity
/// id as the stable tie-break — the same order the database's own post-batch view
/// builds (see `cad_db::drawing::block_edit`).
fn expected_block_members(
    candidate: &DrawingDatabase,
    block: BlockId,
    changes: &[(EntityId, Option<DbEntity>)],
) -> Vec<EntityId> {
    let batch: BTreeMap<EntityId, Option<&DbEntity>> = changes
        .iter()
        .map(|(id, change)| (*id, change.as_ref()))
        .collect();
    let mut rows: Vec<(i64, u128)> = candidate
        .entities()
        .filter(|entity| !batch.contains_key(&entity.id))
        .chain(batch.values().filter_map(|change| *change))
        .filter(|entity| entity.space == SpaceId::Block(block))
        .map(|entity| (entity.draw_order, entity.id.0))
        .collect();
    rows.sort_by_key(|(order, id)| (*order, *id));
    rows.into_iter().map(|(_, id)| EntityId(id)).collect()
}

/// Reject a non-empty payload: the four selectors take their option from the
/// typed argument channel, never from a point/selection payload.
fn require_none(command: &Command, prefix: &str) -> CadResult<()> {
    if matches!(command.payload, CommandPayload::None) {
        Ok(())
    } else {
        Err(invalid(
            prefix,
            "invalid_option",
            format!(
                "{} takes no payload; the option is ADD|REMOVE / SAVE|DISCARD",
                prefix.to_ascii_uppercase()
            ),
        ))
    }
}

/// One changed entity's before/after row and display state, captured around the
/// single write so an undo replays the exact stored value.
struct CapturedChange {
    id: EntityId,
    before: Option<DbEntity>,
    before_state: EntityDisplayState,
}

/// Snapshot every staged entity before the write (all staged ids exist).
fn capture_before(
    candidate: &DrawingDatabase,
    changes: &[(EntityId, Option<DbEntity>)],
) -> Vec<CapturedChange> {
    changes
        .iter()
        .map(|(id, _)| CapturedChange {
            id: *id,
            before: candidate.entity(*id).cloned(),
            before_state: candidate.entity_display_state(*id),
        })
        .collect()
}

/// Build the patch list from the captured before side and the committed candidate.
fn patches_after(candidate: &DrawingDatabase, captured: Vec<CapturedChange>) -> Vec<DrawingPatch> {
    captured
        .into_iter()
        .map(|change| {
            let after = candidate.entity(change.id).cloned();
            let after_state = after
                .as_ref()
                .map(|_| candidate.entity_display_state(change.id));
            DrawingPatch {
                id: change.id,
                before: change.before,
                after,
                before_state: Some(change.before_state),
                after_state,
            }
        })
        .collect()
}

/// `REFSET ADD|REMOVE` and `REFCLOSE SAVE|DISCARD` live in sibling modules so
/// this file stays inside the code-size ceiling.
mod close;
mod working_set;
