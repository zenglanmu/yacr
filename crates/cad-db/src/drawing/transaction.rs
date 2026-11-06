//! A staged, all-or-nothing drawing write transaction.

use std::collections::BTreeSet;

use cad_domain::*;

use super::block_edit::BlockRow;
use super::{DrawingDatabase, EntityDisplayState, VisibilityMembership};
use crate::change::{ChangeMask, ChangeSet, ObjectChange};
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::transform::transform_geometry;

/// A staged, all-or-nothing drawing write transaction.
///
/// The transaction only stages intent; the database is not touched until
/// [`Self::commit`], which delegates to
/// [`DrawingDatabase::apply_drawing_changes_with_blocks_and_display`], so the
/// entity batch, the block rows and the staged display states land in **one**
/// revision and one `ChangeSet`. A dropped or rolled-back transaction leaves no
/// state behind.
///
/// # Layer locking
///
/// Every **entity** stager funnels through [`Self::stage`], which applies the
/// drawing's layer-lock rule **before staging**: an entity on a locked layer
/// cannot be inserted, updated, moved or deleted, and the call returns
/// `CadError::InvalidInput` with the stable code `layer.locked`. Nothing is
/// staged by a refused call, so `staged_len()` stays unchanged, the transaction
/// stays empty, and the revision/history depth never move.
///
/// [`Self::stage_create_block`] stages a block **table** row, which has no
/// layer: the lock rule applies to the member entities that row will own, and
/// those are staged through the entity stagers above.
///
/// `update_entity_display` is the documented exception: it stages the stored
/// entity row **unchanged**, i.e. no effective change, so a display
/// representation rebuild still works on a locked layer. [`Self::update_entity_display_state`]
/// has exactly the same lock behaviour. See `super::lock` for the full rule (and
/// for why frozen layers are not a second rule here).
pub struct DrawingTransaction<'a> {
    database: &'a mut DrawingDatabase,
    reason: String,
    transaction: TransactionId,
    /// Staged changes in call order (so the published ChangeSet preserves the
    /// order in which the caller staged them, not id order).
    staged: Vec<(EntityId, Option<DbEntity>)>,
    /// Block rows staged in this transaction (creates only in v1), committed
    /// **with** the entity batch as one revision. See
    /// [`Self::stage_create_block`].
    staged_blocks: Vec<(BlockId, Option<BlockRow>)>,
    /// Ids whose staged change is an affine move; used to add
    /// [`ChangeMask::TRANSFORM`] to the resulting update. The shared apply path
    /// cannot infer "move" from the before/after pair alone.
    transformed: BTreeSet<EntityId>,
    /// Staged per-entity display-state replacements (render attributes and
    /// block/visibility membership), applied by the shared write path after it
    /// has validated and committed the entity rows, in the **same** revision and
    /// ChangeSet. They are keyed by id, so a second staging of the same id is
    /// refused like a duplicate entity update.
    display: Vec<(EntityId, EntityDisplayState)>,
    /// Ids staged so far, for early duplicate rejection.
    seen: BTreeSet<EntityId>,
}

impl<'a> DrawingTransaction<'a> {
    /// `pub(super)`: called only by [`DrawingDatabase::begin_drawing_transaction`]
    /// in the sibling `edit` module of this same directory.
    pub(super) fn new(
        database: &'a mut DrawingDatabase,
        reason: &str,
        transaction: TransactionId,
    ) -> CadResult<Self> {
        if reason.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "transaction reason is required".to_string(),
            ));
        }
        Ok(DrawingTransaction {
            database,
            reason: reason.to_string(),
            transaction,
            staged: Vec::new(),
            staged_blocks: Vec::new(),
            transformed: BTreeSet::new(),
            display: Vec::new(),
            seen: BTreeSet::new(),
        })
    }

    /// Number of staged changes (used by tests and the UI status).
    ///
    /// Block rows staged by [`Self::stage_create_block`] count too: they are
    /// part of the same commit.
    pub fn staged_len(&self) -> usize {
        self.staged.len() + self.staged_blocks.len()
    }

    /// Stage insertion of a new entity, returning its id.
    ///
    /// The id must not already exist in the database and must not have been
    /// staged in this transaction. `object.id.0` must equal `entity.id.0`; the
    /// full invariant check happens at commit through the shared apply path.
    pub fn insert_entity(&mut self, entity: DbEntity) -> CadResult<EntityId> {
        let id = entity.id;
        if self.database.entities.contains_key(&id) {
            return Err(CadError::Invariant(format!("entity {id:?} already exists")));
        }
        self.stage(id, Some(entity), false)?;
        Ok(id)
    }

    /// Stage replacement of an existing entity. The id is preserved.
    pub fn update_entity(&mut self, entity: DbEntity) -> CadResult<()> {
        let id = entity.id;
        if !self.database.entities.contains_key(&id) {
            return Err(CadError::Invariant(format!("entity {id:?} does not exist")));
        }
        self.stage(id, Some(entity), false)
    }

    /// Stage a replacement of the per-entity display attributes the
    /// representation/renderer reads (colour, linetype, lineweight, opacity).
    ///
    /// Those attributes live outside [`DbEntity`], so the shared apply path
    /// cannot see them. This stage is a **display-only** write: it stages an
    /// unchanged entity row when the caller has not already staged one (the
    /// ChangeSet needs an update entry for the id), replays the attributes after
    /// the validated entity write, and adds [`ChangeMask::STYLE`] to the id's
    /// update so dependency invalidation rebuilds its representation exactly like
    /// any other style edit. See [`Self::update_entity_display_state`] for the
    /// full-state variant that also restores visibility membership.
    ///
    /// At most one display replacement per id per transaction; staging twice is
    /// a caller bug and is refused. If the caller stages the entity row itself,
    /// do that **before** this call.
    ///
    /// # Layer locking
    ///
    /// This is a **display-only** write and stays allowed on a locked layer: it
    /// stages the stored entity row unchanged (see [`Self::stage`]) and only
    /// replaces the display attributes beside it, exactly like a representation
    /// rebuild. AutoCAD regenerates a locked layer rather than refusing to draw
    /// it, so the display path is not blocked. A command that also edits the
    /// entity row is refused by its own stager (`insert_entity` /
    /// `update_entity` / `transform_entity` / `delete_entity`).
    pub fn update_entity_display(
        &mut self,
        id: EntityId,
        attributes: EntityRenderAttributes,
    ) -> CadResult<()> {
        self.update_entity_display_state(
            id,
            &EntityDisplayState {
                attributes,
                visibility: VisibilityMembership::default(),
            },
        )
    }

    /// Stage a full [`EntityDisplayState`] (render attributes **and** block /
    /// dynamic-visibility membership) for `id`.
    ///
    /// The state is committed by the shared validated write path **after** the
    /// entity rows are written, so an id this same transaction inserts can carry
    /// a display state: the row exists by the time the state is applied. The
    /// commit raises the revision once and reports the id with
    /// [`ChangeMask::STYLE`], so no caller has to restore display state after the
    /// commit (and none can: `restore_entity_display_state` is crate-private).
    ///
    /// An id with no staged entity row is staged as an unchanged row, so the
    /// ChangeSet carries an `Update` for it. Deleting and displaying the same id
    /// in one transaction is refused, as is staging its display twice.
    pub fn update_entity_display_state(
        &mut self,
        id: EntityId,
        state: &EntityDisplayState,
    ) -> CadResult<()> {
        if self.display.iter().any(|(staged, _)| *staged == id) {
            return Err(CadError::Invariant(format!(
                "entity {id:?} has its display attributes modified more than once in one \
                 transaction"
            )));
        }
        if self
            .staged
            .iter()
            .any(|(staged, change)| *staged == id && change.is_none())
        {
            return Err(CadError::Invariant(format!(
                "entity {id:?} cannot be deleted and have its display attributes replaced in \
                 one transaction"
            )));
        }
        if !self.seen.contains(&id) {
            let unchanged = self
                .database
                .entities
                .get(&id)
                .cloned()
                .ok_or_else(|| CadError::Invariant(format!("entity {id:?} does not exist")))?;
            self.stage(id, Some(unchanged), false)?;
        }
        self.display.push((id, state.clone()));
        Ok(())
    }

    /// Stage creation of a block table row, returning its id.
    ///
    /// The row carries the definition (with its exact member list) and, for a
    /// runtime create, the name/base-point metadata. The id must not already
    /// exist and must not have been staged in this transaction; the id is
    /// normally taken from [`DrawingDatabase::allocate_block_id`]. All
    /// structural/name/membership validation runs at commit, together with the
    /// entity batch, so the block and its members land in **one** revision.
    ///
    /// Members are moved with the ordinary
    /// [`Self::update_entity`] stager and the reference is inserted with
    /// [`Self::insert_entity`]; the existing lock chokepoint then refuses a
    /// member on a locked layer before anything is staged.
    ///
    /// There is no public `stage_delete_block` in this version: block deletion
    /// exists only through
    /// [`DrawingDatabase::apply_drawing_changes_with_blocks`], for the future
    /// history replay path.
    pub fn stage_create_block(&mut self, row: BlockRow) -> CadResult<BlockId> {
        let id = row.definition.id;
        if self.database.blocks.contains_key(&id) {
            return Err(CadError::Invariant(format!("block {id:?} already exists")));
        }
        if self.staged_blocks.iter().any(|(staged, _)| *staged == id) {
            return Err(CadError::Invariant(format!(
                "block {id:?} is created more than once in one transaction"
            )));
        }
        self.staged_blocks.push((id, Some(row)));
        Ok(id)
    }

    /// Stage deletion of an existing entity.
    pub fn delete_entity(&mut self, id: EntityId) -> CadResult<()> {
        if !self.database.entities.contains_key(&id) {
            return Err(CadError::Invariant(format!("entity {id:?} does not exist")));
        }
        self.stage(id, None, false)
    }

    /// Stage a move: replace the entity's geometry with `transform` applied to
    /// it, preserving id/object/space/layer/draw order.
    ///
    /// The move is baked into the geometry for the kinds listed in
    /// [`crate::transform_geometry`]. An unsupported kind or a transform that
    /// cannot be represented exactly (for example a non-uniform scale on a
    /// circle) fails with [`CadError::Unsupported`] and stages nothing; a
    /// mirror is never silently dropped. The committed update carries
    /// [`ChangeMask::TRANSFORM`] **and** [`ChangeMask::GEOMETRY`]: the geometry
    /// genuinely changed, and the transform bit tells the renderer a rigid move
    /// produced it.
    pub fn transform_entity(&mut self, id: EntityId, transform: &Transform3) -> CadResult<()> {
        let Some(entity) = self.database.entities.get(&id) else {
            return Err(CadError::Invariant(format!("entity {id:?} does not exist")));
        };
        let mut moved = entity.clone();
        moved.geometry = transform_geometry(&entity.geometry, transform)?;
        self.stage(id, Some(moved), true)
    }

    /// The single staging gate: duplicate detection, the layer-lock rule and the
    /// staged-change bookkeeping every stager shares.
    ///
    /// The lock check runs **first**, before anything is recorded, so a refusal
    /// leaves the transaction exactly as it was (no staged row, no `seen` entry,
    /// no transform bit). `apply_drawing_changes` re-applies the same rule at
    /// commit time; that second check is what covers history replay and any
    /// direct caller, and it cannot half-apply because the apply path validates
    /// the whole batch before mutating.
    fn stage(
        &mut self,
        id: EntityId,
        change: Option<DbEntity>,
        is_transform: bool,
    ) -> CadResult<()> {
        self.database
            .ensure_change_allowed_on_locked_layer(id, change.as_ref())?;
        if !self.seen.insert(id) {
            return Err(CadError::Invariant(format!(
                "entity {id:?} is modified more than once in one transaction"
            )));
        }
        if is_transform {
            self.transformed.insert(id);
        }
        self.staged.push((id, change));
        Ok(())
    }

    /// Commit the staged changes through the shared validated write path.
    ///
    /// On success the database revision advances by one (unless nothing was
    /// staged) and the returned ChangeSet lists the changes in staging order.
    /// On failure the database is untouched.
    pub fn commit(mut self) -> CadResult<ChangeSet> {
        let staged = std::mem::take(&mut self.staged);
        let staged_blocks = std::mem::take(&mut self.staged_blocks);
        let transformed = std::mem::take(&mut self.transformed);
        let display = std::mem::take(&mut self.display);
        let mut changeset = self
            .database
            .apply_drawing_changes_with_blocks_and_display(
                &self.reason,
                self.transaction,
                &staged_blocks,
                staged,
                &display,
            )?;
        // The apply path sees only before/after, so it cannot know a change was
        // a deliberate move. Add the TRANSFORM bit here for the moved ids; the
        // geometry/validation/mutation all happened in the shared path.
        if !transformed.is_empty() {
            for change in &mut changeset.changes {
                if let ObjectChange::Update(object, mask) = change {
                    if transformed.contains(&EntityId(object.0)) {
                        *mask = mask.union(ChangeMask::TRANSFORM);
                    }
                }
            }
        }
        Ok(changeset)
    }

    /// Uncommitted staged changes are discarded on drop.
    pub fn rollback(self) {
        // Dropping `self` discards `staged`; the database was never touched.
    }
}
