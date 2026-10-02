//! The authoritative, read-only-after-import drawing database.

use std::collections::BTreeMap;

use cad_domain::*;

use crate::bounds::{BoundsAccumulator, MAX_INSTANCE_DEPTH};
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::tables::{
    BlockDefinition, Layer, Layout, PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation,
    PlotSettingsRecord, PlotType, Style,
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
    /// Plot configuration per paper-space layout. Absent entries are resolved
    /// to an explicit default page by [`DrawingDatabase::plot_settings_for`].
    pub(crate) plot_settings: BTreeMap<LayoutId, PlotSettingsRecord>,
    /// Per-entity display attributes that the importer resolved from the source
    /// (transparency, geometry source). Absent entries are fully opaque
    /// analytic geometry, so older/hand-built databases stay valid.
    pub(crate) render_attributes: BTreeMap<EntityId, EntityRenderAttributes>,
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
    pub fn block_entities(&self, id: BlockId) -> Vec<&DbEntity> {
        let Some(block) = self.blocks.get(&id) else {
            return Vec::new();
        };
        let mut v: Vec<&DbEntity> = block
            .entities
            .iter()
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
