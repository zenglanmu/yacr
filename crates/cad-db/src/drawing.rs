//! The authoritative, read-only-after-import drawing database.

use std::collections::BTreeMap;

use cad_domain::*;

use crate::bounds::{BoundsAccumulator, MAX_INSTANCE_DEPTH};
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::tables::{BlockDefinition, Layer, Layout, Style};

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
