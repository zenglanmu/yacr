//! The sanctioned controlled write paths for the drawing database.

use cad_domain::*;

use super::{DrawingDatabase, DrawingTransaction};
use crate::change::{ChangeMask, ChangeSet, ObjectChange};
use crate::entity::DbEntity;

impl DrawingDatabase {
    /// Switch a dynamic block's active visibility state.
    ///
    /// This is the controlled write path for a visibility grip switch: it
    /// validates the state against the block's own descriptor (an unknown state
    /// is rejected, never guessed), updates the authoritative active state,
    /// raises the revision, and publishes a [`ChangeSet`] whose changes are the
    /// entities that entered or left the visible set. The scene cache therefore
    /// invalidates only this block's affected geometry (spec §4.6 / 增量更新),
    /// and the representation re-derives from the database without re-importing
    /// the base drawing.
    ///
    /// A block with no visibility descriptor cannot be toggled and is rejected
    /// explicitly. A block whose active state was unresolved can be switched:
    /// before the switch every governed member is considered visible, so the
    /// emitted delta honestly reflects whatever the target state hides.
    pub fn set_block_visibility_state(
        &mut self,
        block: BlockId,
        state: &str,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        let before = self.revision;
        let current = self
            .blocks
            .get(&block)
            .and_then(|b| b.dynamic_visibility.as_ref())
            .ok_or_else(|| {
                CadError::InvalidInput(format!("block {block:?} has no dynamic visibility states"))
            })?;
        if !current.has_state(state) {
            return Err(CadError::InvalidInput(format!(
                "block {block:?} does not define visibility state '{state}'"
            )));
        }
        if current.active_state.as_deref() == Some(state) {
            // Already active: no authoritative change, so no revision bump and
            // no fabricated ChangeSet.
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
            });
        }

        // Entities that enter or leave the visible set. Ungoverned entities are
        // visible in both states and never appear here.
        let visible_before = self.block_visible_entities(block).unwrap_or_default();
        let members = &self.blocks[&block].entities;
        let target_visible: Vec<EntityId> = match current.state(state) {
            Some(s) => members
                .iter()
                .copied()
                .filter(|e| !current.member_entities.contains(e) || s.entities.contains(e))
                .collect(),
            None => Vec::new(),
        };
        let mut affected: Vec<EntityId> = visible_before
            .iter()
            .chain(target_visible.iter())
            .copied()
            .collect();
        affected.sort();
        affected.dedup();
        let affected: Vec<EntityId> = affected
            .into_iter()
            .filter(|e| {
                let before = visible_before.contains(e);
                let after = target_visible.contains(e);
                before != after
            })
            .collect();

        if let Some(block_def) = self.blocks.get_mut(&block) {
            if let Some(vis) = block_def.dynamic_visibility.as_mut() {
                vis.active_state = Some(state.to_string());
            }
        }
        let after = Revision(before.0 + 1);
        self.revision = after;
        let changes = affected
            .into_iter()
            .map(|e| ObjectChange::Update(ObjectId(e.0), ChangeMask::GEOMETRY))
            .collect();
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes,
        })
    }

    // -----------------------------------------------------------------------
    // Controlled drawing write path (docs/drawing-edit.md §1)
    //
    // The base drawing is read-only after import (spec §4.3); these methods are
    // the single sanctioned mutation path, mirroring
    // `set_block_visibility_state`: validate everything first, then mutate, then
    // raise the revision and publish an ordered ChangeSet. A failure at any point
    // leaves the database and its revision untouched.
    // -----------------------------------------------------------------------

    /// The next id [`Self::allocate_entity_id`] will return.
    pub fn next_entity_id(&self) -> EntityId {
        EntityId(self.next_entity_id)
    }

    /// Allocate a fresh, unused entity id.
    ///
    /// The allocator is monotonic and is never rewound by inserts or deletes, so
    /// a deleted id is never handed out again — an id carries a generation-like
    /// meaning and a stale reference can never alias a new entity. Allocation is
    /// not itself a content write, so it does not raise the revision.
    pub fn allocate_entity_id(&mut self) -> EntityId {
        let id = EntityId(self.next_entity_id);
        self.next_entity_id = self.next_entity_id.saturating_add(1);
        // Allocation changes no content and does not raise the revision, so the
        // revision-tagged bounds memo is still exact. It is deliberately **not**
        // cleared here: the memo is only served when its tag equals the current
        // revision, and a later content write raises the revision (or invalidates
        // explicitly), so no stale extent can escape. Clearing it would force a
        // full bounds recompute after every edit and defeat the insert-only fast
        // path in `apply_drawing_changes_with_blocks_and_display`.
        id
    }

    /// Begin a staged, all-or-nothing drawing transaction.
    ///
    /// Nothing is written until [`DrawingTransaction::commit`]; an empty or
    /// rolled-back transaction leaves the database exactly as it was.
    pub fn begin_drawing_transaction(
        &mut self,
        reason: &str,
        id: TransactionId,
    ) -> CadResult<DrawingTransaction<'_>> {
        DrawingTransaction::new(self, reason, id)
    }

    /// The single validated write path for drawing entities.
    ///
    /// `changes` maps an entity id to its new value (`None` = delete). All
    /// changes are validated before any mutation, so on any error the database
    /// and revision are unchanged. A non-empty, fully valid batch raises the
    /// revision by one and returns a [`ChangeSet`] in the caller's order. An
    /// empty batch returns a `before == after` ChangeSet with no bump and no
    /// fabricated changes.
    ///
    /// This is shared by [`DrawingTransaction`] and by history/undo: both must go
    /// through here so validation, layer-lock enforcement and change-tracking
    /// cannot be bypassed.
    ///
    /// A write that would create, modify or delete an entity on a **locked**
    /// layer is refused with `layer.locked` in phase 1 (before any mutation), so
    /// the failure is all-or-nothing exactly like a validation failure. See
    /// `drawing::lock` for the rule, its two chokepoints and the documented
    /// history-replay and display-write decisions.
    pub fn apply_drawing_changes(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        changes: Vec<(EntityId, Option<DbEntity>)>,
    ) -> CadResult<ChangeSet> {
        // The entity-only batch is the combined block+entity path with no block
        // rows. Keeping one implementation means validation, the layer-lock
        // chokepoint, allocator bumps and change tracking cannot diverge between
        // "entity only" and "entity plus block" writes.
        self.apply_drawing_changes_with_blocks(reason, transaction, &[], changes)
    }

    /// Ensure entity `id` is listed as a member of its block, if it is in one.
    ///
    /// `pub(super)`: the combined block+entity apply path in the sibling
    /// `block_edit` module performs the same mutation step.
    pub(super) fn attach_block_membership(&mut self, id: EntityId) {
        let Some(entity) = self.entities.get(&id) else {
            return;
        };
        let SpaceId::Block(block) = entity.space else {
            return;
        };
        if let Some(definition) = self.blocks.get_mut(&block) {
            if !definition.entities.contains(&id) {
                definition.entities.push(id);
            }
        }
    }

    /// Remove every dangling reference to a deleted entity.
    ///
    /// A delete must leave no reachable reference behind (docs/drawing-edit.md
    /// §1.6): the entity row, its render attributes (keyed by id) and its
    /// membership in any block member list are all removed. Dynamic-visibility
    /// state/member lists are pruned too, since they are keyed by entity id;
    /// leaving them would let a later switch name a nonexistent entity.
    pub(super) fn remove_entity(&mut self, id: EntityId) {
        self.entities.remove(&id);
        self.render_attributes.remove(&id);
        self.detach_block_membership(id);
    }

    /// Prune old block references on deletion or a change of owning space.
    /// Render attributes are retained when the entity itself still exists.
    pub(super) fn detach_block_membership(&mut self, id: EntityId) {
        for definition in self.blocks.values_mut() {
            definition.entities.retain(|e| *e != id);
            if let Some(visibility) = definition.dynamic_visibility.as_mut() {
                visibility.member_entities.retain(|e| *e != id);
                for state in &mut visibility.states {
                    state.entities.retain(|e| *e != id);
                }
            }
        }
    }
}
