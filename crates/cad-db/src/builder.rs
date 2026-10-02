//! The sanctioned construction path for a [`DrawingDatabase`].

use std::collections::BTreeMap;

use cad_domain::*;

use crate::drawing::DrawingDatabase;
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::tables::{BlockDefinition, Layer, Layout, PlotSettingsRecord, Style};

/// The only sanctioned way to construct a [`DrawingDatabase`].
///
/// Validation is deferred to [`Self::finish`], which rejects dangling layer,
/// block and layout references so a partially-built database never escapes.
pub struct DrawingDatabaseBuilder {
    database: DrawingDatabase,
    errors: Vec<String>,
}

impl DrawingDatabaseBuilder {
    pub fn new(id: DatabaseId) -> Self {
        Self {
            database: DrawingDatabase {
                id,
                revision: Revision(0),
                entities: BTreeMap::new(),
                layers: BTreeMap::new(),
                blocks: BTreeMap::new(),
                layouts: BTreeMap::new(),
                styles: BTreeMap::new(),
                plot_settings: BTreeMap::new(),
                render_attributes: BTreeMap::new(),
            },
            errors: Vec::new(),
        }
    }

    pub fn insert_entity(&mut self, entity: DbEntity) -> CadResult<()> {
        if self.database.entities.contains_key(&entity.id) {
            return Err(CadError::Invariant(format!(
                "duplicate entity id {:?} during import",
                entity.id
            )));
        }
        self.database.entities.insert(entity.id, entity);
        Ok(())
    }

    /// Record the display attributes the importer resolved for an entity.
    ///
    /// The entity must already have been inserted; recording attributes for an
    /// unknown id is a caller bug and is rejected rather than silently lost.
    pub fn set_entity_render_attributes(
        &mut self,
        id: EntityId,
        attributes: EntityRenderAttributes,
    ) -> CadResult<()> {
        if !self.database.entities.contains_key(&id) {
            return Err(CadError::Invariant(format!(
                "render attributes for unknown entity {id:?}"
            )));
        }
        self.database.render_attributes.insert(id, attributes);
        Ok(())
    }

    pub fn insert_layer(&mut self, layer: Layer) -> CadResult<()> {
        self.database.layers.insert(layer.id, layer);
        Ok(())
    }

    pub fn insert_block(&mut self, block: BlockDefinition) -> CadResult<()> {
        self.database.blocks.insert(block.id, block);
        Ok(())
    }

    pub fn insert_layout(&mut self, layout: Layout) -> CadResult<()> {
        self.database.layouts.insert(layout.id, layout);
        Ok(())
    }

    pub fn insert_style(&mut self, style: Style) -> CadResult<()> {
        self.database.styles.insert(style.id, style);
        Ok(())
    }

    /// Record the plot configuration read for a layout.
    ///
    /// The layout must already exist; plot settings for an unknown layout would
    /// be silently unreachable, so that is rejected as a caller bug.
    pub fn set_plot_settings(&mut self, settings: PlotSettingsRecord) -> CadResult<()> {
        if !self.database.layouts.contains_key(&settings.layout) {
            return Err(CadError::Invariant(format!(
                "plot settings for unknown layout {:?}",
                settings.layout
            )));
        }
        self.database
            .plot_settings
            .insert(settings.layout, settings);
        Ok(())
    }

    /// Validate references and produce the immutable database.
    pub fn finish(mut self) -> CadResult<DrawingDatabase> {
        // Every entity's layer must exist.
        for entity in self.database.entities.values() {
            if !self.database.layers.contains_key(&entity.layer) {
                self.errors.push(format!(
                    "entity {:?} references missing layer {:?}",
                    entity.id, entity.layer
                ));
            }
            if let SpaceId::Paper(layout) = &entity.space {
                if !self.database.layouts.contains_key(layout) {
                    self.errors.push(format!(
                        "entity {:?} references missing layout {:?}",
                        entity.id, layout
                    ));
                }
            }
        }
        // Block definitions must not reference unknown entities.
        for block in self.database.blocks.values() {
            for e in &block.entities {
                if !self.database.entities.contains_key(e) {
                    self.errors.push(format!(
                        "block {:?} references missing entity {:?}",
                        block.id, e
                    ));
                }
            }
        }
        if !self.errors.is_empty() {
            let joined = self.errors.join("; ");
            return Err(CadError::Invariant(format!(
                "import validation failed: {joined}"
            )));
        }
        Ok(self.database)
    }
}
