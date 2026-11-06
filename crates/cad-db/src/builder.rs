//! The sanctioned construction path for a [`DrawingDatabase`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use cad_domain::*;

use crate::dim_style::DimStyle;
use crate::drawing::DrawingDatabase;
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::mleader_style::MLeaderStyle;
use crate::tables::{
    ActiveAnnotationScale, BlockDefinition, BlockMetadata, DynamicBlockVisibility, Layer,
    LayerAttributes, Layout, LineType, PlotSettingsRecord, Scale, Style,
};
use crate::xref::ExternalReference;

/// Process-global source of [`DrawingDatabase::construction_nonce`].
///
/// Starts at 1 so `0` is never a real database identity. `Relaxed` is enough:
/// only uniqueness within one process is required, not ordering between
/// constructions. A `u64` wrap would require 2^64 constructions in one process.
static NEXT_CONSTRUCTION_NONCE: AtomicU64 = AtomicU64::new(1);

fn next_construction_nonce() -> u64 {
    NEXT_CONSTRUCTION_NONCE.fetch_add(1, Ordering::Relaxed)
}

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
                construction_nonce: next_construction_nonce(),
                entities: BTreeMap::new(),
                layers: BTreeMap::new(),
                blocks: BTreeMap::new(),
                block_metadata: BTreeMap::new(),
                block_name_order: Vec::new(),
                block_source_handles: BTreeMap::new(),
                layouts: BTreeMap::new(),
                styles: BTreeMap::new(),
                linetypes: BTreeMap::new(),
                linetype_scale: 1.0,
                point_display_mode: 0,
                point_display_size: 0.0,
                plot_settings: BTreeMap::new(),
                render_attributes: BTreeMap::new(),
                layer_attributes: BTreeMap::new(),
                scales: BTreeMap::new(),
                dim_styles: BTreeMap::new(),
                dim_style_ids_by_name: BTreeMap::new(),
                mleader_styles: BTreeMap::new(),
                mleader_style_ids_by_name: BTreeMap::new(),
                active_annotation_scale: None,
                code_page: None,
                next_entity_id: 1,
                next_block_id: 1,
                next_layer_id: 1,
                external_references: BTreeMap::new(),
                bounds_cache: Default::default(),
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

    /// Record the attributes the importer resolved for a layer.
    ///
    /// The layer must already have been inserted; recording attributes for an
    /// unknown id is a caller bug and is rejected rather than silently lost
    /// (mirroring [`Self::set_entity_render_attributes`]).
    ///
    /// This is storage only: nothing in this build reads the flags to change
    /// behaviour, and [`Layer::visible`] is **not** recomputed here — the
    /// importer remains the authority for it.
    pub fn set_layer_attributes(
        &mut self,
        id: LayerId,
        attributes: LayerAttributes,
    ) -> CadResult<()> {
        if !self.database.layers.contains_key(&id) {
            return Err(CadError::Invariant(format!(
                "layer attributes for unknown layer {id:?}"
            )));
        }
        self.database.layer_attributes.insert(id, attributes);
        Ok(())
    }

    pub fn insert_block(&mut self, block: BlockDefinition) -> CadResult<()> {
        self.database.blocks.insert(block.id, block);
        Ok(())
    }

    /// Record one external reference's metadata and provenance.
    ///
    /// The name is the xref block name and is the map key, so it must be
    /// unique: a duplicate is an import error rather than a silent replace,
    /// because replacing would make the earlier attachment's provenance
    /// unreachable. If `block` names an xref-owned block, that block is
    /// validated for existence at [`Self::finish`] (block definitions are often
    /// created after their table row).
    ///
    /// This is import-only storage: it raises no revision and produces no
    /// [`crate::ChangeSet`]. External references cannot be edited through a
    /// transaction in this build.
    pub fn insert_external_reference(&mut self, reference: ExternalReference) -> CadResult<()> {
        if self
            .database
            .external_references
            .contains_key(&reference.name)
        {
            return Err(CadError::InvalidInput(format!(
                "duplicate external reference name '{}'",
                reference.name
            )));
        }
        self.database
            .external_references
            .insert(reference.name.clone(), reference);
        Ok(())
    }

    /// Record a block's table name and insertion base point.
    ///
    /// The block must already have been inserted: metadata for an unknown block
    /// is a caller bug and is rejected at [`Self::finish`] (block entities are
    /// filled in after the definition is created, exactly like the dynamic
    /// visibility descriptor).
    ///
    /// The name is trimmed. An empty/whitespace-only name is **not registered**
    /// (the block stays unnamed and is not addressable by name) and leaves no
    /// metadata behind, so no zero base point is ever invented. A non-finite base
    /// point is refused. Duplicate names are allowed; [`DrawingDatabase::block_by_name`]
    /// resolves the **last registered** duplicate, matching the importer's own
    /// name map, so a duplicate is never an import abort.
    pub fn set_block_metadata(
        &mut self,
        id: BlockId,
        name: &str,
        base_point: Point3,
    ) -> CadResult<()> {
        let name = name.trim();
        if name.is_empty() {
            // An empty name means "leave the block unnamed": clear any previous
            // registration for this id rather than keeping a stale spelling alive.
            self.database.block_metadata.remove(&id);
            self.database
                .block_name_order
                .retain(|(_, existing)| *existing != id);
            return Ok(());
        }
        if !base_point.x.is_finite() || !base_point.y.is_finite() || !base_point.z.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "block '{}' has a non-finite base point",
                name
            )));
        }
        self.database.block_metadata.insert(
            id,
            BlockMetadata {
                name: name.to_string(),
                base_point,
            },
        );
        // A re-registration replaces the block's previous name completely, so no
        // stale spelling can resolve to the new metadata.
        self.database
            .block_name_order
            .retain(|(_, existing)| *existing != id);
        self.database.block_name_order.push((name.to_string(), id));
        Ok(())
    }

    /// Record the source DWG handle of an imported block definition.
    ///
    /// Vendor derived representations (for example a Tianzheng door symbol)
    /// carry a block handle, not a table name; this map is what turns that
    /// handle back into a [`BlockId`]. A zero handle is ignored so no "handle 0
    /// is the first block" shortcut can ever appear. Re-registering a handle
    /// replaces the previous owner (the last imported definition wins, exactly
    /// like duplicate block names).
    pub fn set_block_source_handle(&mut self, id: BlockId, handle: u64) {
        if handle == 0 {
            return;
        }
        self.database.block_source_handles.insert(handle, id);
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

    /// Insert a DIMSTYLE table entry (ADR 0017 B.1).
    ///
    /// A style with an empty name is rejected: it could never resolve group 3.
    /// A style whose name already exists replaces the earlier entry in place
    /// (keeping the first id), so a source `Standard` overrides the pinned
    /// default without leaving a duplicate name in the index.
    ///
    /// A style whose **id** is already owned by a *different* name is rejected
    /// (review N9): inserting it would overwrite `dim_styles[id]` and orphan the
    /// old name's index entry, so the old name would silently resolve to the new
    /// style. A caller that needs a fresh identity must allocate a new id.
    pub fn insert_dim_style(&mut self, style: DimStyle) -> CadResult<()> {
        if style.name.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "dimension style name is empty".into(),
            ));
        }
        if let Some(existing) = self.database.dim_style_ids_by_name.get(&style.name) {
            let mut replaced = style;
            replaced.id = *existing;
            self.database.dim_styles.insert(*existing, replaced);
            return Ok(());
        }
        if let Some(existing) = self.database.dim_styles.get(&style.id) {
            if existing.name != style.name {
                return Err(CadError::InvalidInput(format!(
                    "dimension style id {:?} is already owned by '{}'; it cannot also own '{}'",
                    style.id, existing.name, style.name
                )));
            }
        }
        self.database
            .dim_style_ids_by_name
            .insert(style.name.clone(), style.id);
        self.database.dim_styles.insert(style.id, style);
        Ok(())
    }

    /// Insert a MLEADERSTYLE table entry (ADR 0019 decision D, P7c).
    ///
    /// Same name/id rules as [`Self::insert_dim_style`]: an empty name is
    /// rejected (it could never resolve), a source row whose name already exists
    /// replaces the earlier entry in place (keeping the first id), and an id
    /// already owned by a **different** name is rejected rather than orphaning
    /// the old name's index entry.
    pub fn insert_mleader_style(&mut self, style: MLeaderStyle) -> CadResult<()> {
        if style.name.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "multileader style name is empty".into(),
            ));
        }
        if let Some(existing) = self.database.mleader_style_ids_by_name.get(&style.name) {
            let mut replaced = style;
            replaced.id = *existing;
            self.database.mleader_styles.insert(*existing, replaced);
            return Ok(());
        }
        if let Some(existing) = self.database.mleader_styles.get(&style.id) {
            if existing.name != style.name {
                return Err(CadError::InvalidInput(format!(
                    "multileader style id {:?} is already owned by '{}'; it cannot also own '{}'",
                    style.id, existing.name, style.name
                )));
            }
        }
        self.database
            .mleader_style_ids_by_name
            .insert(style.name.clone(), style.id);
        self.database.mleader_styles.insert(style.id, style);
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

    /// Record the drawing's declared text code page (`$DWGCODEPAGE`).
    ///
    /// An empty or whitespace-only string means "the source declared nothing"
    /// and is stored as `None`, never guessed. A non-empty value is stored
    /// verbatim (case preserved) so the platform locale mapping is the only
    /// place that interprets it.
    pub fn set_code_page(&mut self, code_page: Option<&str>) {
        self.database.code_page = code_page
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
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

    /// Record the drawing-global POINT display mode (`$PDMODE`).
    ///
    /// The value is stored **raw**, including bit combinations and values this
    /// project cannot express: the display layer is the single interpreter (it
    /// draws the expressible base marker and reports the rest), so rejecting an
    /// unknown mode here would turn a display nicety into an import failure. No
    /// migration is needed — the field defaults to the DWG default `0`.
    pub fn set_point_display_mode(&mut self, mode: i16) {
        self.database.point_display_mode = mode;
    }

    /// Record the drawing-global POINT display size (`$PDSIZE`).
    ///
    /// Stored **raw**, for the same reason as [`Self::set_point_display_mode`]:
    /// the sign (viewport-relative vs absolute) and the sanitisation rule are the
    /// display layer's contract, not the db's. Defaults to the DWG default `0.0`.
    pub fn set_point_display_size(&mut self, size: f64) {
        self.database.point_display_size = size;
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
        // A DIMSTYLE table always carries a `Standard` (ADR 0017 B.1). A
        // hand-built database, or a source drawing that had none, gets the
        // pinned metric default; an imported `Standard` is left as read.
        if !self.database.dim_style_ids_by_name.contains_key("Standard") {
            let next = self
                .database
                .dim_styles
                .keys()
                .map(|id| id.0)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let id = DimStyleId(next);
            let style = DimStyle::standard_metric(id);
            self.database
                .dim_style_ids_by_name
                .insert(style.name.clone(), id);
            self.database.dim_styles.insert(id, style);
        }
        // A MLEADERSTYLE table always carries a `Standard` (ADR 0019 D/P7c),
        // exactly like DIMSTYLE: a hand-built database, or a source drawing that
        // had none, gets the pinned constant row; an imported `Standard` is left
        // as read.
        if !self
            .database
            .mleader_style_ids_by_name
            .contains_key(MLeaderStyle::STANDARD_NAME)
        {
            let next = self
                .database
                .mleader_styles
                .keys()
                .map(|id| id.0)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let id = MLeaderStyleId(next);
            let style = MLeaderStyle::standard_metric(id);
            self.database
                .mleader_style_ids_by_name
                .insert(style.name.clone(), id);
            self.database.mleader_styles.insert(id, style);
        }
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
        // Block metadata must resolve to a real block and keep a finite base
        // point: a dangling entry would never be addressable, and a non-finite
        // one would poison every reference transform built from it.
        for (id, metadata) in &self.database.block_metadata {
            if !self.database.blocks.contains_key(id) {
                self.errors
                    .push(format!("block metadata for missing block {id:?}"));
            }
            if !metadata.base_point.x.is_finite()
                || !metadata.base_point.y.is_finite()
                || !metadata.base_point.z.is_finite()
            {
                self.errors.push(format!(
                    "block {id:?} ('{}') has a non-finite base point",
                    metadata.name
                ));
            }
        }
        // An external reference's provenance block must exist: a dangling id
        // would make `external_reference_for_block`/`is_xref_block` point at a
        // block that was never imported.
        for reference in self.database.external_references.values() {
            if let Some(block) = reference.block {
                if !self.database.blocks.contains_key(&block) {
                    self.errors.push(format!(
                        "external reference '{}' references missing block {block:?}",
                        reference.name
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
        // Initialise the block-id allocator from the largest imported block id,
        // so the first `allocate_block_id` can never collide with an imported
        // definition (and a later delete of the largest row cannot make its id
        // reusable).
        let max_block = self.database.blocks.keys().map(|b| b.0).max().unwrap_or(0);
        self.database.next_block_id = max_block.saturating_add(1);
        // Initialise the layer-id allocator the same way, from the largest
        // imported layer id, so the first `create_layer` can never collide and a
        // later delete of the largest row cannot make its id reusable.
        let max_layer = self.database.layers.keys().map(|l| l.0).max().unwrap_or(0);
        self.database.next_layer_id = max_layer.saturating_add(1);
        Ok(self.database)
    }
}
