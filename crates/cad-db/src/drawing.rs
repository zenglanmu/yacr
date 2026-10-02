//! The authoritative, read-only-after-import drawing database.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::*;

use crate::bounds::{BoundsAccumulator, MAX_INSTANCE_DEPTH};
use crate::change::{ChangeMask, ChangeSet, ObjectChange};
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::tables::{
    ActiveAnnotationScale, BlockDefinition, DynamicBlockVisibility, Layer, Layout, LineType,
    PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation, PlotSettingsRecord, PlotType, Scale,
    Style,
};
use crate::transform::transform_geometry;
use crate::validate::validate_entity;

/// The authoritative, read-only-after-import drawing database.
#[derive(Debug, Clone)]
pub struct DrawingDatabase {
    pub(crate) id: DatabaseId,
    pub(crate) revision: Revision,
    pub(crate) entities: BTreeMap<EntityId, DbEntity>,
    pub(crate) layers: BTreeMap<LayerId, Layer>,
    pub(crate) blocks: BTreeMap<BlockId, BlockDefinition>,
    pub(crate) layouts: BTreeMap<LayoutId, Layout>,
    pub(crate) styles: BTreeMap<StyleId, Style>,
    /// Named linetype table entries (spec §3.2). Empty for hand-built
    /// databases; the importer fills it from the source drawing.
    pub(crate) linetypes: BTreeMap<LinetypeId, LineType>,
    /// Drawing-global linetype scale (`$LTSCALE`). `1.0` is the DWG default and
    /// the value used when a database was not built by the importer.
    pub(crate) linetype_scale: f64,
    /// Plot configuration per paper-space layout. Absent entries are resolved
    /// to an explicit default page by [`DrawingDatabase::plot_settings_for`].
    pub(crate) plot_settings: BTreeMap<LayoutId, PlotSettingsRecord>,
    /// Per-entity display attributes that the importer resolved from the source
    /// (transparency, geometry source). Absent entries are fully opaque
    /// analytic geometry, so older/hand-built databases stay valid.
    pub(crate) render_attributes: BTreeMap<EntityId, EntityRenderAttributes>,
    /// Named annotation scales from the drawing's `ACAD_SCALELIST`. Empty for
    /// hand-built databases; the importer fills it from the source drawing.
    pub(crate) scales: BTreeMap<ScaleId, Scale>,
    /// Drawing-global active annotation scale (`CANNOSCALE`), when set.
    pub(crate) active_annotation_scale: Option<ActiveAnnotationScale>,
    /// Next entity id to hand out. Monotonic and never reset when entities are
    /// deleted, so a deleted id is never reused (its `ObjectId` would otherwise
    /// alias a new object). Initialised by the builder from the largest existing
    /// entity/block-member id.
    pub(crate) next_entity_id: u128,
}

impl DrawingDatabase {
    pub fn id(&self) -> DatabaseId {
        self.id
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Content identity used to decide whether a render/scene cache is stale.
    ///
    /// A bare `DatabaseId` is not enough: hosts may reuse the same id for a
    /// different drawing, which would leave old GPU batches on screen (audit
    /// B04). The identity therefore mixes the id, revision, structural counts
    /// and model-space bounds.
    pub fn scene_identity(&self) -> SceneIdentity {
        let (min, max) = self
            .bounds()
            .unwrap_or((Point3::default(), Point3::default()));
        SceneIdentity {
            database: self.id,
            revision: self.revision,
            entities: self.entities.len() as u64,
            layers: self.layers.len() as u64,
            bounds: [min, max],
        }
    }

    pub fn entity(&self, id: EntityId) -> Option<&DbEntity> {
        self.entities.get(&id)
    }

    /// Display attributes the importer resolved for an entity.
    ///
    /// Missing entries mean "fully opaque analytic geometry", so a database
    /// built without an importer (tests, hand-built fixtures) keeps working.
    pub fn entity_render_attributes(&self, id: EntityId) -> EntityRenderAttributes {
        self.render_attributes.get(&id).cloned().unwrap_or_default()
    }

    pub fn entities(&self) -> impl Iterator<Item = &DbEntity> {
        self.entities.values()
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.get(&id)
    }

    pub fn layers(&self) -> impl Iterator<Item = &Layer> {
        self.layers.values()
    }

    pub fn block(&self, id: BlockId) -> Option<&BlockDefinition> {
        self.blocks.get(&id)
    }

    /// The dynamic-block visibility descriptor of a block, if it has one.
    pub fn block_dynamic_visibility(&self, id: BlockId) -> Option<&DynamicBlockVisibility> {
        self.blocks.get(&id)?.dynamic_visibility.as_ref()
    }

    /// Member entities the block's active visibility state makes visible.
    ///
    /// `None` when the block carries no visibility descriptor. When a
    /// descriptor exists but its active state is unknown, this returns the full
    /// governed set so an unresolved state never silently hides geometry.
    pub fn block_visible_entities(&self, id: BlockId) -> Option<Vec<EntityId>> {
        let block = self.blocks.get(&id)?;
        let visibility = block.dynamic_visibility.as_ref()?;
        let visible: Vec<EntityId> = block
            .entities
            .iter()
            .copied()
            .filter(|e| visibility.is_visible(*e))
            .collect();
        Some(visible)
    }

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
        let members: Vec<EntityId> = current.member_entities.clone();
        let target_visible: Vec<EntityId> = match current.state(state) {
            Some(s) => members
                .iter()
                .copied()
                .filter(|e| s.entities.contains(e))
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

    pub fn blocks(&self) -> impl Iterator<Item = &BlockDefinition> {
        self.blocks.values()
    }

    pub fn layout(&self, id: LayoutId) -> Option<&Layout> {
        self.layouts.get(&id)
    }

    pub fn layouts(&self) -> impl Iterator<Item = &Layout> {
        self.layouts.values()
    }

    // -----------------------------------------------------------------------
    // Controlled drawing write path (docs/drawing-edit.md §1)
    //
    // The base drawing is read-only after import (spec §4.3); these methods are
    // the single sanctioned mutation path, mirroring
    // `AnnotationDatabase::apply_annotation_changes` and
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
    /// This is shared by [`DrawingTransaction`] and, later, by history/undo: both
    /// must go through here so validation and change-tracking cannot be bypassed.
    pub fn apply_drawing_changes(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        changes: Vec<(EntityId, Option<DbEntity>)>,
    ) -> CadResult<ChangeSet> {
        // Phase 1: validate everything. No mutation happens until this passes.
        let mut seen = BTreeSet::new();
        for (id, change) in &changes {
            if !seen.insert(*id) {
                return Err(CadError::Invariant(format!(
                    "transaction {transaction:?} modifies entity {id:?} twice"
                )));
            }
            match change {
                Some(entity) => validate_entity(self, *id, entity)?,
                None => {
                    if !self.entities.contains_key(id) {
                        return Err(CadError::Invariant(format!("entity {id:?} does not exist")));
                    }
                }
            }
        }

        let before = self.revision;
        if changes.is_empty() {
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
            });
        }

        // Phase 2: mutate. The batch is known-valid; keep the caller's order.
        let mut ordered = Vec::with_capacity(changes.len());
        for (id, change) in changes {
            match change {
                Some(entity) => {
                    let mask = match self.entities.get(&id) {
                        Some(previous) => ChangeMask::for_entity_update(previous, &entity),
                        None => ChangeMask::GEOMETRY.union(ChangeMask::STYLE),
                    };
                    let existed = self.entities.contains_key(&id);
                    self.entities.insert(id, entity);
                    // Keep the allocator ahead of any explicitly supplied id so
                    // a later allocation can never collide (history/undo replays
                    // insert with their recorded ids through this same path).
                    self.next_entity_id = self.next_entity_id.max(id.0.saturating_add(1));
                    // A new block-member entity must appear in its block's
                    // member list, or the geometry would exist but never be
                    // reachable through the INSERT.
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

    /// Ensure entity `id` is listed as a member of its block, if it is in one.
    fn attach_block_membership(&mut self, id: EntityId) {
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
    fn remove_entity(&mut self, id: EntityId) {
        self.entities.remove(&id);
        self.render_attributes.remove(&id);
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

    /// Plot settings stored for a layout, exactly as read.
    ///
    /// `None` means the drawing carried no PLOTSETTINGS data for that layout;
    /// callers that need to render must use [`Self::plot_settings_for`], which
    /// makes the fallback explicit instead of inventing values.
    pub fn plot_settings(&self, id: LayoutId) -> Option<&PlotSettingsRecord> {
        self.plot_settings.get(&id)
    }

    /// Plot settings for a layout, or an explicit documented default page.
    ///
    /// The default is the ISO A4 sheet (210 × 297 mm) with zero margins and no
    /// rotation, marked [`PlotProvenance::DefaultPage`]. A4 is an ISO standard,
    /// not a vendor value; it is chosen as a neutral canvas so a layout that
    /// carries no plot data is still plottable, and the provenance makes clear
    /// that nothing was read from the file.
    pub fn plot_settings_for(&self, id: LayoutId) -> PlotSettingsRecord {
        if let Some(record) = self.plot_settings.get(&id) {
            return record.clone();
        }
        PlotSettingsRecord {
            layout: id,
            paper_size_name: "ISO_A4_(210.00_x_297.00_MM)".to_string(),
            paper_width: 210.0,
            paper_height: 297.0,
            margins: PlotMargins::default(),
            rotation: PlotRotation::None,
            scale_numerator: 1.0,
            scale_denominator: 1.0,
            plot_type: PlotType::Layout,
            paper_units: PlotPaperUnits::Millimeters,
            provenance: PlotProvenance::DefaultPage {
                reason: "layout has no PLOTSETTINGS object or embedded plot data; \
                         using the ISO A4 (210x297 mm) default page"
                    .to_string(),
            },
        }
    }

    pub fn style(&self, id: StyleId) -> Option<&Style> {
        self.styles.get(&id)
    }

    /// Named linetype table entries.
    pub fn linetypes(&self) -> impl Iterator<Item = &LineType> {
        self.linetypes.values()
    }

    /// A linetype table entry by identity.
    pub fn linetype(&self, id: LinetypeId) -> Option<&LineType> {
        self.linetypes.get(&id)
    }

    /// Drawing-global linetype scale (`$LTSCALE`). Defaults to `1.0`.
    pub fn linetype_scale(&self) -> f64 {
        self.linetype_scale
    }

    /// Named annotation scales from the drawing's `ACAD_SCALELIST`.
    pub fn scales(&self) -> impl Iterator<Item = &Scale> {
        self.scales.values()
    }

    /// A named annotation scale by identity.
    pub fn scale(&self, id: ScaleId) -> Option<&Scale> {
        self.scales.get(&id)
    }

    /// A named annotation scale by name, matched exactly then case-insensitively.
    ///
    /// Drawing scale names are case-sensitive in principle but vendor exports
    /// vary; the case-insensitive pass is the documented fallback and never
    /// invents a factor.
    pub fn scale_by_name(&self, name: &str) -> Option<&Scale> {
        if let Some(scale) = self.scales.values().find(|s| s.name == name) {
            return Some(scale);
        }
        self.scales
            .values()
            .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The raw active annotation scale as set by the importer, when present.
    pub fn active_annotation_scale(&self) -> Option<&ActiveAnnotationScale> {
        self.active_annotation_scale.as_ref()
    }

    /// The active annotation scale name, when one is set.
    pub fn annotation_scale_name(&self) -> Option<&str> {
        self.active_annotation_scale
            .as_ref()
            .map(|s| s.name.as_str())
            .filter(|name| !name.is_empty())
    }

    /// The drawing's active annotation scale, resolved for rendering.
    ///
    /// Resolution order:
    /// 1. A named [`Scale`] in the table matching the active name — the table is
    ///    authoritative and its [`Scale::factor`] is used.
    /// 2. The header `CANNOSCALEVALUE`, when finite and positive.
    /// 3. `1.0` (no scaling), the safe fallback for a database with no scale
    ///    information at all.
    ///
    /// Returns `(name, factor, resolved)`. `resolved` is `false` only when the
    /// name was set but neither the table nor a valid header value could supply
    /// a factor; callers that render annotative entities must report that as an
    /// explicit `Partial` rather than silently scaling by `1.0`.
    pub fn annotation_scale(&self) -> (Option<String>, f64, bool) {
        let Some(active) = &self.active_annotation_scale else {
            return (None, 1.0, true);
        };
        let name = (!active.name.is_empty()).then(|| active.name.clone());
        if let Some(scale) = self.scale_by_name(&active.name) {
            return (name, scale.factor(), true);
        }
        if active.value.is_finite() && active.value > 0.0 {
            return (name, active.value, false);
        }
        (name, 1.0, false)
    }

    /// Entities in model space, in draw order.
    pub fn model_space(&self) -> Vec<&DbEntity> {
        let mut v: Vec<&DbEntity> = self
            .entities
            .values()
            .filter(|e| matches!(e.space, SpaceId::Model))
            .collect();
        v.sort_by_key(|e| e.draw_order);
        v
    }

    /// Entities of a block definition, in draw order.
    ///
    /// For a dynamic block with a resolved active visibility state, only the
    /// entities that state makes visible are returned (spec §3.2); ungoverned
    /// entities always remain. Without a descriptor (or with an unresolved
    /// active state) every member is returned, so a guess can never hide
    /// geometry.
    pub fn block_entities(&self, id: BlockId) -> Vec<&DbEntity> {
        let Some(block) = self.blocks.get(&id) else {
            return Vec::new();
        };
        let mut v: Vec<&DbEntity> = block
            .entities
            .iter()
            .filter(|e| {
                block
                    .dynamic_visibility
                    .as_ref()
                    .map(|vis| vis.is_visible(**e))
                    .unwrap_or(true)
            })
            .filter_map(|e| self.entities.get(e))
            .collect();
        v.sort_by_key(|e| e.draw_order);
        v
    }

    /// Entities belonging to a layout, in draw order.
    pub fn layout_entities(&self, id: LayoutId) -> Vec<&DbEntity> {
        let mut v: Vec<&DbEntity> = self
            .entities
            .values()
            .filter(|e| matches!(&e.space, SpaceId::Paper(l) if *l == id))
            .collect();
        v.sort_by_key(|e| e.draw_order);
        v
    }

    /// World bounds of model-space geometry, expanding INSERT instances.
    pub fn bounds(&self) -> Option<(Point3, Point3)> {
        let mut acc = BoundsAccumulator::new();
        let mut stack: Vec<BlockId> = Vec::new();
        for e in self.model_space() {
            self.accumulate_bounds(&mut acc, e, &Transform3::identity(), 0, &mut stack);
        }
        acc.finish()
    }

    /// Recursively fold one model-space entity (and any inserts) into `acc`.
    ///
    /// Nesting is bounded and cycles are cut so a malformed block graph cannot
    /// loop forever; a truncated/cyclic branch simply contributes no bounds.
    fn accumulate_bounds(
        &self,
        acc: &mut BoundsAccumulator,
        entity: &DbEntity,
        transform: &Transform3,
        depth: usize,
        stack: &mut Vec<BlockId>,
    ) {
        if let SemanticGeometry::Insert {
            block,
            transform: insert,
        } = &entity.geometry
        {
            if depth >= MAX_INSTANCE_DEPTH || stack.contains(block) {
                return;
            }
            let composed = transform.matrix_mul(insert);
            stack.push(*block);
            for child in self.block_entities(*block) {
                self.accumulate_bounds(acc, child, &composed, depth + 1, stack);
            }
            stack.pop();
        } else {
            acc.add_geometry_transformed(&entity.geometry, transform);
        }
    }
}

/// A staged, all-or-nothing drawing write transaction.
///
/// The transaction only stages intent; the database is not touched until
/// [`Self::commit`], which delegates to
/// [`DrawingDatabase::apply_drawing_changes`]. A dropped or rolled-back
/// transaction leaves no state behind.
pub struct DrawingTransaction<'a> {
    database: &'a mut DrawingDatabase,
    reason: String,
    transaction: TransactionId,
    /// Staged changes in call order (so the published ChangeSet preserves the
    /// order in which the caller staged them, not id order).
    staged: Vec<(EntityId, Option<DbEntity>)>,
    /// Ids whose staged change is an affine move; used to add
    /// [`ChangeMask::TRANSFORM`] to the resulting update. The shared apply path
    /// cannot infer "move" from the before/after pair alone.
    transformed: BTreeSet<EntityId>,
    /// Ids staged so far, for early duplicate rejection.
    seen: BTreeSet<EntityId>,
}

impl<'a> DrawingTransaction<'a> {
    fn new(
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
            transformed: BTreeSet::new(),
            seen: BTreeSet::new(),
        })
    }

    /// Number of staged changes (used by tests and the UI status).
    pub fn staged_len(&self) -> usize {
        self.staged.len()
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

    fn stage(
        &mut self,
        id: EntityId,
        change: Option<DbEntity>,
        is_transform: bool,
    ) -> CadResult<()> {
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
        let transformed = std::mem::take(&mut self.transformed);
        let mut changeset =
            self.database
                .apply_drawing_changes(&self.reason, self.transaction, staged)?;
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
