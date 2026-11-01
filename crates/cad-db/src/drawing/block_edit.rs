//! Atomic block-table writes: the runtime create path behind a future `BLOCK`
//! command.
//!
//! A block is not a second drawing. It is a row in [`DrawingDatabase::blocks`]
//! plus name/base-point [`BlockMetadata`] beside it, while its geometry is
//! ordinary [`DbEntity`] rows whose [`SpaceId`] is [`SpaceId::Block`]. Creating
//! a block therefore always edits **both** tables: the definition row and the
//! selected entities that become its members. Doing that through two writes
//! would publish two revisions (and, later, two undo steps) and could leave a
//! row without its geometry if the second write failed.
//!
//! [`DrawingDatabase::apply_drawing_changes_with_blocks`] is the single
//! all-or-nothing path for such a combined write:
//!
//! 1. validate the block batch (ids, names, member lists, delete safety) with a
//!    **post-batch** entity view, so a member created/moved by the same batch is
//!    seen;
//! 2. validate the entity batch through the existing layer-lock +
//!    [`validate_entity_in`] chokepoint, with the batch's block ids staged as
//!    valid references;
//! 3. mutate block creates/updates first, then the entity batch (the existing
//!    membership attach/detach), then block deletes last;
//! 4. raise the revision **exactly once** and publish the entity-scoped
//!    [`ChangeSet`] in the caller's order.
//!
//! # Membership is a stored, ordered list
//!
//! [`BlockDefinition::entities`] is the authoritative member list (the block
//! geometry query, bounds expansion and dynamic visibility all read it). The
//! entity write path silently appends a member to its block when the row exists
//! — an INSERT that names a missing block would otherwise "work" while never
//! drawing anything. To make that silent no-op impossible, a block row in this
//! batch must list **exactly** the post-batch entities whose `space` is
//! `Block(id)`, ordered by `draw_order` (no phantom member, no orphan member).
//! The check runs before any mutation, so a mismatch is refused
//! all-or-nothing.
//!
//! # Runtime rules are stricter than the importer
//!
//! The importer allows duplicate block names (its own name map is last-wins,
//! and [`DrawingDatabase::block_by_name`] reproduces that). A **create** through
//! this path refuses a name that already resolves (exact, then ASCII
//! case-insensitive) because inventing a second block with an existing name is
//! ambiguous to every later `INSERT`/`BLOCK` command; the importer behaviour is
//! untouched. A runtime create also cannot carry a dynamic-visibility descriptor:
//! a descriptor mirrors a source parameter list, and nothing in a new block
//! defines one.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::*;

use super::DrawingDatabase;
use crate::change::{ChangeMask, ChangeSet, ObjectChange};
use crate::entity::DbEntity;
use crate::tables::{BlockDefinition, BlockMetadata};
use crate::validate::validate_entity_in;

/// One block table row as a writable unit.
///
/// [`BlockDefinition`] keeps its two-field shape; the optional name/base point
/// lives beside it exactly as it does in the database (see
/// [`BlockMetadata`]), so an unnamed block is representable and no zero base
/// point is invented.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockRow {
    pub definition: BlockDefinition,
    pub metadata: Option<BlockMetadata>,
}

/// A before/after record of one block row.
///
/// `None` means the row did not exist (`before`) / no longer exists (`after`).
/// Nothing in this build produces one yet — the atomic apply path is ready for
/// the future `BLOCK` history patch — but the type is public so the history
/// layer does not have to invent a second, divergent row shape.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockPatch {
    pub id: BlockId,
    pub before: Option<BlockRow>,
    pub after: Option<BlockRow>,
}

impl DrawingDatabase {
    /// The next id [`Self::allocate_block_id`] will return.
    pub fn next_block_id(&self) -> BlockId {
        BlockId(self.next_block_id)
    }

    /// Allocate a fresh, unused block id.
    ///
    /// Monotonic like [`Self::allocate_entity_id`]: a delete never rewinds it, so
    /// a deleted block id is never handed out again and a stale reference cannot
    /// alias a new definition. Allocation is not a content write, so it does not
    /// raise the revision.
    pub fn allocate_block_id(&mut self) -> BlockId {
        let id = BlockId(self.next_block_id);
        self.next_block_id = self.next_block_id.saturating_add(1);
        self.invalidate_bounds_cache();
        id
    }

    /// The single validated write path for a **combined** block + entity batch.
    ///
    /// `block_changes` maps a block id to its new row (`None` = delete) and
    /// `changes` is the ordinary entity batch accepted by
    /// [`Self::apply_drawing_changes`]; the two are committed as one revision.
    ///
    /// Validation happens before any mutation, so every refusal listed below
    /// leaves the tables, the entity rows, the allocators and the revision
    /// exactly as they were:
    ///
    /// * duplicate ids inside either batch;
    /// * a block create whose id already exists, whose name is empty or spans
    ///   several lines, whose base point is non-finite, whose name already
    ///   resolves ([`Self::block_by_name`] semantics), that carries a
    ///   dynamic-visibility descriptor, or that repeats a member id;
    /// * a block row whose member list is not *exactly* the post-batch entities
    ///   in its space, ordered by `draw_order` (phantom/orphan member);
    /// * a block delete with no stored row, or one still referenced by a
    ///   post-batch entity (space or `INSERT`);
    /// * any entity failure the plain path reports, including the layer-lock
    ///   rule (checked before the block batch is even inspected).
    ///
    /// Block creates/updates are applied before the entity batch and block
    /// deletes after it, so no intermediate state is observable: a created
    /// definition exists before its members arrive, and a deleted definition
    /// disappears after its last reference leaves. The revision advances by one
    /// for a non-empty combined batch and does not move for an empty one.
    ///
    /// # ChangeSet is entity-scoped
    ///
    /// There is no block object-change kind, so the published [`ChangeSet`]
    /// lists only entity changes (in caller order); a block-only batch bumps the
    /// revision with an empty change list, exactly like the layer-table writes.
    /// A block create that moves members and inserts a reference produces the
    /// member `Update`s and the reference `Insert`, which is what drives
    /// representation invalidation.
    pub fn apply_drawing_changes_with_blocks(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        block_changes: &[(BlockId, Option<BlockRow>)],
        changes: Vec<(EntityId, Option<DbEntity>)>,
    ) -> CadResult<ChangeSet> {
        let before = self.revision;
        if block_changes.is_empty() && changes.is_empty() {
            // Nothing staged: no revision bump, no fabricated change.
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
            });
        }

        // Phase 1a: entity identity + the layer-lock chokepoint. The lock rule
        // runs first so a locked member refuses before the block batch is
        // inspected (and nothing has mutated either way).
        let mut seen_entities = BTreeSet::new();
        for (id, change) in &changes {
            if !seen_entities.insert(*id) {
                return Err(CadError::Invariant(format!(
                    "transaction {transaction:?} modifies entity {id:?} twice"
                )));
            }
            self.ensure_change_allowed_on_locked_layer(*id, change.as_ref())?;
        }

        // Post-batch entity view: the batch overlay on top of the stored rows.
        let batch: BTreeMap<EntityId, Option<&DbEntity>> = changes
            .iter()
            .map(|(id, change)| (*id, change.as_ref()))
            .collect();
        let post_batch = post_batch_entities(self, &batch);

        // Phase 1b: the whole block batch (shape, names, membership, deletes).
        let staged_blocks = validate_block_batch(self, block_changes, &post_batch)?;

        // Phase 1c: entity rows, with the same batch's block ids staged as valid
        // references (`SpaceId::Block(new)` and `Insert { block: new }`).
        for (id, change) in &changes {
            match change {
                Some(entity) => validate_entity_in(self, *id, entity, &staged_blocks)?,
                None => {
                    if !self.entities.contains_key(id) {
                        return Err(CadError::Invariant(format!("entity {id:?} does not exist")));
                    }
                }
            }
        }

        // Phase 2: mutate. The batch is known-valid; keep the caller's order.
        let mut ordered = Vec::with_capacity(changes.len());

        // 2a: block creates/updates first, so a member can attach to its row.
        for (id, change) in block_changes {
            if let Some(row) = change {
                self.install_block_row(*id, row);
                self.bump_block_allocator(*id);
            }
        }

        // 2b: the entity batch (the existing membership attach/detach).
        for (id, change) in changes {
            match change {
                Some(entity) => {
                    let mask = match self.entities.get(&id) {
                        Some(previous) => ChangeMask::for_entity_update(previous, &entity),
                        None => ChangeMask::GEOMETRY.union(ChangeMask::STYLE),
                    };
                    let existed = self.entities.contains_key(&id);
                    if self
                        .entities
                        .get(&id)
                        .is_some_and(|previous| previous.space != entity.space)
                    {
                        self.detach_block_membership(id);
                    }
                    self.entities.insert(id, entity);
                    // Keep the allocator ahead of any explicitly supplied id so
                    // a later allocation can never collide.
                    self.next_entity_id = self.next_entity_id.max(id.0.saturating_add(1));
                    self.attach_block_membership(id);
                    if existed {
                        ordered.push(ObjectChange::Update(ObjectId(id.0), mask));
                    } else {
                        ordered.push(ObjectChange::Insert(ObjectId(id.0)));
                    }
                }
                None => {
                    self.remove_entity(id);
                    ordered.push(ObjectChange::Delete(ObjectId(id.0)));
                }
            }
        }

        // 2c: block deletes last, once nothing references them any more.
        for (id, change) in block_changes {
            if change.is_none() {
                self.blocks.remove(id);
                self.block_metadata.remove(id);
                self.block_name_order.retain(|(_, existing)| existing != id);
                self.bump_block_allocator(*id);
            }
        }

        let after = Revision(before.0 + 1);
        self.revision = after;
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: ordered,
        })
    }

    /// Store one validated block row, keeping the name index in sync.
    ///
    /// The name is trimmed on the way in (the builder does the same); the
    /// validation step already proved it is non-empty and single-line.
    fn install_block_row(&mut self, id: BlockId, row: &BlockRow) {
        self.blocks.insert(id, row.definition.clone());
        match &row.metadata {
            Some(metadata) => {
                let name = metadata.name.trim().to_string();
                self.block_metadata.insert(
                    id,
                    BlockMetadata {
                        name: name.clone(),
                        base_point: metadata.base_point,
                    },
                );
                // A re-registration replaces the previous spelling completely, so
                // no stale name can resolve to the new row.
                self.block_name_order
                    .retain(|(_, existing)| *existing != id);
                self.block_name_order.push((name, id));
            }
            None => {
                self.block_metadata.remove(&id);
                self.block_name_order
                    .retain(|(_, existing)| *existing != id);
            }
        }
    }

    /// Keep the block allocator ahead of every id this batch applied.
    fn bump_block_allocator(&mut self, id: BlockId) {
        self.next_block_id = self.next_block_id.max(id.0.saturating_add(1));
    }
}

/// Validate the whole block batch; returns the staged (created/updated) ids.
fn validate_block_batch(
    db: &DrawingDatabase,
    block_changes: &[(BlockId, Option<BlockRow>)],
    post_batch: &[&DbEntity],
) -> CadResult<BTreeSet<BlockId>> {
    let mut seen = BTreeSet::new();
    let mut staged = BTreeSet::new();
    let mut deleted = Vec::new();
    for (id, change) in block_changes {
        if !seen.insert(*id) {
            return Err(CadError::Invariant(format!(
                "transaction modifies block {id:?} twice"
            )));
        }
        match change {
            Some(row) => {
                let exists = db.blocks.contains_key(id);
                validate_block_row(*id, row, !exists)?;
                if !exists {
                    // Runtime create: a name that already resolves would make
                    // `block_by_name` ambiguous for every later command.
                    let name = row
                        .metadata
                        .as_ref()
                        .map(|metadata| metadata.name.trim())
                        .unwrap_or("");
                    if let Some(resolved) = db.block_id_by_name(name) {
                        return Err(CadError::InvalidInput(format!(
                            "block.duplicate_name: this drawing already has a block named '{}' \
                             (id {}); names are unique case-insensitively",
                            name, resolved.0
                        )));
                    }
                }
                // The member list is authoritative and stored: it must match the
                // post-batch entities exactly, or the block query/bounds would
                // silently see phantom or missing geometry.
                let expected = expected_members(post_batch, *id);
                if row.definition.entities != expected {
                    return Err(CadError::Invariant(format!(
                        "block.member_list_mismatch: block {id:?} lists {:?} but its post-batch \
                         members ordered by draw order are {expected:?}",
                        row.definition.entities
                    )));
                }
                staged.insert(*id);
            }
            None => {
                if !db.blocks.contains_key(id) {
                    return Err(CadError::Invariant(format!(
                        "block.unknown_id: cannot delete block {id:?}; it does not exist"
                    )));
                }
                deleted.push(*id);
            }
        }
    }
    // Delete safety: a surviving reference would be dangling the moment the row
    // disappears, so the delete is refused instead of rewriting the reference.
    for id in deleted {
        if let Some(entity) = post_batch
            .iter()
            .find(|entity| references_block(entity, id))
        {
            return Err(CadError::Invariant(format!(
                "block.in_use: block {id:?} is still referenced by entity {:?}; move or delete \
                 the reference first",
                entity.id
            )));
        }
    }
    Ok(staged)
}

/// The structural rules one block row must satisfy.
///
/// A **create** must name the block (a name that cannot be resolved is useless
/// to a later `INSERT`/`BLOCK`) and must not carry a dynamic-visibility
/// descriptor. An **update** may leave the row unnamed, mirroring imported
/// unnamed definitions; the member-list invariant still applies to both.
fn validate_block_row(id: BlockId, row: &BlockRow, is_create: bool) -> CadResult<()> {
    if row.definition.id != id {
        return Err(CadError::Invariant(format!(
            "change key {id:?} does not match block id {:?}",
            row.definition.id
        )));
    }
    let mut members = BTreeSet::new();
    for member in &row.definition.entities {
        if !members.insert(*member) {
            return Err(CadError::Invariant(format!(
                "block.member_duplicate: block {id:?} lists entity {member:?} twice"
            )));
        }
    }
    if is_create && row.definition.dynamic_visibility.is_some() {
        return Err(CadError::InvalidInput(
            "block.dynamic_visibility_unsupported: a runtime-created block cannot carry a \
             dynamic-visibility descriptor"
                .into(),
        ));
    }
    let Some(metadata) = row.metadata.as_ref() else {
        if is_create {
            return Err(CadError::InvalidInput(format!(
                "block.name_empty: block {id:?} has no name/base-point metadata"
            )));
        }
        return Ok(());
    };
    let name = metadata.name.trim();
    if name.is_empty() {
        return Err(CadError::InvalidInput(
            "block.name_empty: a block name must not be empty or whitespace-only".into(),
        ));
    }
    if name.contains(['\n', '\r']) {
        return Err(CadError::InvalidInput(format!(
            "block.name_multiline: block name '{name}' must be a single line"
        )));
    }
    if !metadata.base_point.x.is_finite()
        || !metadata.base_point.y.is_finite()
        || !metadata.base_point.z.is_finite()
    {
        return Err(CadError::InvalidInput(format!(
            "block.base_point_non_finite: block '{name}' has a non-finite base point"
        )));
    }
    Ok(())
}

/// The entities the batch leaves in `block`, ordered by draw order.
///
/// `post_batch` is already sorted that way by [`post_batch_entities`], so the
/// returned order is the stored member order a block row must reproduce.
fn expected_members(post_batch: &[&DbEntity], block: BlockId) -> Vec<EntityId> {
    post_batch
        .iter()
        .filter(|entity| entity.space == SpaceId::Block(block))
        .map(|entity| entity.id)
        .collect()
}

/// The stored rows overlaid with the batch, sorted by `draw_order`.
fn post_batch_entities<'a>(
    db: &'a DrawingDatabase,
    batch: &BTreeMap<EntityId, Option<&'a DbEntity>>,
) -> Vec<&'a DbEntity> {
    let mut entities: Vec<&DbEntity> = db
        .entities
        .iter()
        .filter(|(id, _)| !batch.contains_key(id))
        .map(|(_, entity)| entity)
        .chain(batch.values().filter_map(|change| *change))
        .collect();
    // A stable sort keeps the (deterministic) stored/BTreeMap order for ids that
    // share a `draw_order`, so "exactly this list" is reproducible.
    entities.sort_by_key(|entity| entity.draw_order);
    entities
}

/// Whether `entity` keeps `block` reachable (space ownership or an INSERT).
///
/// The INSERT arm checks the entity's own geometry, the same shape the bounds
/// expansion follows; a nested INSERT (for example inside a compound) is not a
/// block reference this database can expand, so it is not counted as one.
fn references_block(entity: &DbEntity, block: BlockId) -> bool {
    entity.space == SpaceId::Block(block)
        || matches!(
            &entity.geometry,
            SemanticGeometry::Insert { block: referenced, .. } if *referenced == block
        )
}
