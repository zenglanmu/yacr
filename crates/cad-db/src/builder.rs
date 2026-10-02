//! The sanctioned construction path for a [`DrawingDatabase`].

use std::collections::BTreeMap;

use cad_domain::*;

use crate::drawing::DrawingDatabase;
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::tables::{
    ActiveAnnotationScale, BlockDefinition, DynamicBlockVisibility, Layer, Layout, LineType,
    PlotSettingsRecord, Scale, Style,
};

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
                linetypes: BTreeMap::new(),
                linetype_scale: 1.0,
                plot_settings: BTreeMap::new(),
                render_attributes: BTreeMap::new(),
                scales: BTreeMap::new(),
                active_annotation_scale: None,
                next_entity_id: 1,
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

    /// Attach (or replace) a block's dynamic-block visibility descriptor.
    ///
    /// The descriptor is validated against the block's own member set at
    /// [`Self::finish`] time, because block entities are filled in after the
    /// definition is created. Attaching a descriptor to an unknown block is a
    /// caller bug and is rejected immediately.
    pub fn set_block_dynamic_visibility(
        &mut self,
        id: BlockId,
        visibility: DynamicBlockVisibility,
    ) -> CadResult<()> {
        let Some(block) = self.database.blocks.get_mut(&id) else {
            return Err(CadError::Invariant(format!(
                "dynamic visibility for unknown block {id:?}"
            )));
        };
        block.dynamic_visibility = Some(visibility);
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

    pub fn insert_linetype(&mut self, linetype: LineType) -> CadResult<()> {
        self.database.linetypes.insert(linetype.id, linetype);
        Ok(())
    }

    /// Insert a named annotation scale.
    ///
    /// A scale with a non-finite ratio or an empty name is rejected: it could
    /// not produce a usable factor and would poison annotative rendering. The
    /// name is not required to be unique across ids (the importer assigns ids
    /// from the source order), but a lookup by name takes the first match.
    pub fn insert_scale(&mut self, scale: Scale) -> CadResult<()> {
        if scale.name.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "annotation scale name is empty".into(),
            ));
        }
        if !scale.is_well_formed() {
            return Err(CadError::InvalidInput(format!(
                "annotation scale '{}' has a non-finite ratio",
                scale.name
            )));
        }
        self.database.scales.insert(scale.id, scale);
        Ok(())
    }

    /// Set the drawing's active annotation scale (`CANNOSCALE`).
    ///
    /// `value` is the raw paper/drawing ratio from the header. An empty name is
    /// rejected: a scale with no name cannot be matched against the table, and
    /// a non-finite value would make the fallback factor unusable. The name is
    /// **not** required to exist in the Scale table here — an unknown name is
    /// exactly the `Partial` case the importer reports, and
    /// [`DrawingDatabase::annotation_scale`] then falls back to `value`.
    pub fn set_active_annotation_scale(&mut self, name: &str, value: f64) -> CadResult<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CadError::InvalidInput(
                "active annotation scale name is empty".into(),
            ));
        }
        if !value.is_finite() || value <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "active annotation scale value must be a positive finite number, got {value}"
            )));
        }
        self.database.active_annotation_scale = Some(ActiveAnnotationScale {
            name: name.to_string(),
            value,
            named: self.database.scale_by_name(name).is_some(),
        });
        Ok(())
    }

    /// Set the drawing-global linetype scale (`$LTSCALE`). A non-finite or
    /// non-positive value is rejected rather than poisoning the render path.
    pub fn set_linetype_scale(&mut self, scale: f64) -> CadResult<()> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "linetype scale must be a positive finite number, got {scale}"
            )));
        }
        self.database.linetype_scale = scale;
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
            // A dynamic visibility descriptor must describe this block exactly:
            // state names are unique and non-empty, every governed/visible
            // entity is a member of the block, and a resolved active state is
            // one of the defined states. A dangling state or entity would
            // otherwise silently hide or expose the wrong geometry.
            if let Some(vis) = &block.dynamic_visibility {
                let mut seen = std::collections::BTreeSet::new();
                for state in &vis.states {
                    if state.name.trim().is_empty() {
                        self.errors.push(format!(
                            "block {:?} has a dynamic visibility state with no name",
                            block.id
                        ));
                    } else if !seen.insert(state.name.as_str()) {
                        self.errors.push(format!(
                            "block {:?} defines visibility state '{}' twice",
                            block.id, state.name
                        ));
                    }
                    for e in &state.entities {
                        if !block.entities.contains(e) {
                            self.errors.push(format!(
                                "block {:?} visibility state '{}' references non-member entity {:?}",
                                block.id, state.name, e
                            ));
                        }
                    }
                }
                for e in &vis.member_entities {
                    if !block.entities.contains(e) {
                        self.errors.push(format!(
                            "block {:?} visibility parameter governs non-member entity {:?}",
                            block.id, e
                        ));
                    }
                }
                if let Some(active) = &vis.active_state {
                    if !vis.has_state(active) {
                        self.errors.push(format!(
                            "block {:?} active visibility state '{}' is not defined",
                            block.id, active
                        ));
                    }
                }
            }
        }
        if !self.errors.is_empty() {
            let joined = self.errors.join("; ");
            return Err(CadError::Invariant(format!(
                "import validation failed: {joined}"
            )));
        }
        // Initialise the id allocator from the largest id in the finished
        // database (entities plus block member lists, defensively), so the first
        // `allocate_entity_id` can never collide with an imported entity.
        let max_entity = self
            .database
            .entities
            .keys()
            .map(|e| e.0)
            .chain(
                self.database
                    .blocks
                    .values()
                    .flat_map(|b| b.entities.iter().map(|e| e.0)),
            )
            .max()
            .unwrap_or(0);
        self.database.next_entity_id = max_entity.saturating_add(1);
        Ok(self.database)
    }
}
