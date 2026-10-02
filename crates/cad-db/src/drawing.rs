//! The authoritative, read-only-after-import drawing database.

use std::collections::BTreeMap;

use cad_domain::*;

use crate::bounds::{BoundsAccumulator, MAX_INSTANCE_DEPTH};
use crate::change::{ChangeMask, ChangeSet, ObjectChange};
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::tables::{
    ActiveAnnotationScale, BlockDefinition, DynamicBlockVisibility, Layer, Layout, LineType,
    PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation, PlotSettingsRecord, PlotType, Scale,
    Style,
};

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
