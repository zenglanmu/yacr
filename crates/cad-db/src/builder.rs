//! The sanctioned construction path for a [`DrawingDatabase`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cad_domain::ymodel::IdCounters;
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
    /// Crate-visible so the BIM builder methods live in `crate::bim` (ADR-0042
    /// new code stays in a new module; `builder.rs` only wires the field and the
    /// `finish()` gate).
    pub(crate) database: DrawingDatabase,
    pub(crate) errors: Vec<String>,
    /// Persisted counters a load path supplied, or `None` on the import path
    /// (which has no counters yet and rebuilds them from row maxima).
    seeded_counters: Option<IdCounters>,
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
                hidden_entities: BTreeSet::new(),
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
                insertion_base: None,
                limits_min: None,
                limits_max: None,
                dim_styles: BTreeMap::new(),
                dim_style_ids_by_name: BTreeMap::new(),
                mleader_styles: BTreeMap::new(),
                mleader_style_ids_by_name: BTreeMap::new(),
                active_annotation_scale: None,
                code_page: None,
                next_entity_id: 1,
                next_block_id: 1,
                next_layer_id: 1,
                next_layout_id: 1,
                next_style_id: 1,
                next_dim_style_id: 1,
                next_mleader_style_id: 1,
                next_linetype_id: 1,
                next_scale_id: 1,
                external_references: BTreeMap::new(),
                textures: BTreeMap::new(),
                resources: BTreeMap::new(),
                resource_names: BTreeMap::new(),
                resource_sources: BTreeMap::new(),
                embed_resources: false,
                bim_elements: BTreeMap::new(),
                bim_definitions: crate::bim::BimDefinitions::new(),
                bim_relations: crate::bim::BimRelations::new(),
                bounds_cache: Default::default(),
            },
            errors: Vec::new(),
            seeded_counters: None,
        }
    }

    /// Set the revision the finished database starts at.
    ///
    /// The import path leaves it at `Revision(0)` (a fresh document, no saves
    /// yet). A load path seeds it from `manifest.saved_revision` (ADR-0037 D2):
    /// `manifest.open_revision` is the import baseline and never advances, so
    /// without this seed an edited-then-reopened document would still satisfy
    /// the L0 predicate `revision == open_revision` and could export divergent
    /// content as "the original bytes".
    pub fn set_revision(&mut self, revision: Revision) {
        self.database.revision = revision;
    }

    /// Adopt the persisted identifier high-water marks instead of rebuilding
    /// them from `max(row id) + 1`.
    ///
    /// A **load** path must call this: the counters are document state (ADR-0037
    /// D5), and a rebuild would hand a deleted id to the next created object.
    /// The import path must not call it — an imported drawing has no persisted
    /// counters, so [`Self::finish`] rebuilds them from row maxima.
    ///
    /// [`Self::finish`] validates the seeded counters verbatim: every one must
    /// be in `1 ..= MAX_SEQ + 1` and at least `max(existing row id) + 1`. A
    /// counter that would reuse an existing id refuses the build; it is never
    /// clamped.
    pub fn seed_identifiers(&mut self, counters: IdCounters) {
        self.seeded_counters = Some(counters);
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

    /// Record one texture's bytes under its document-owned key (ADR-0038 D2/D3).
    ///
    /// The key must be the canonical form [`cad_domain::is_valid_texture_key`]
    /// accepts (`<32 lowercase hex>.png|.jpg`): it is the `.ymodel` entry name and
    /// the value a mesh's `texture` field carries, so a path, a `..` or an
    /// uppercase spelling would make the document unrepresentable on disk and is
    /// refused here rather than at save time. An empty payload is refused too — a
    /// zero-byte image can never decode, so storing it would turn "the texture is
    /// missing" into a decode failure with no origin.
    ///
    /// There is **no budget policy in the database**: the importer owns the
    /// `import.texture.*` limits before it calls this. Re-inserting the same key is
    /// idempotent (the key is content-addressed); different bytes under one key are
    /// a caller bug and are refused instead of silently replacing the asset.
    ///
    /// Textures are document state: [`crate::DrawingDatabase::snapshot`] carries
    /// them and the loader re-inserts them. This method is the only writer, so
    /// there is no post-import mutation path.
    pub fn insert_texture(&mut self, key: impl Into<String>, bytes: Arc<[u8]>) -> CadResult<()> {
        let key = key.into();
        if !cad_domain::is_valid_texture_key(&key) {
            return Err(CadError::InvalidInput(format!(
                "cad:texture.key: {key:?} is not a valid texture key (expected \
                 <32 lowercase hex>.png|.jpg)"
            )));
        }
        if bytes.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "cad:texture.empty: texture {key:?} has no bytes"
            )));
        }
        if let Some(existing) = self.database.textures.get(&key) {
            if existing.as_ref() != bytes.as_ref() {
                return Err(CadError::Invariant(format!(
                    "cad:texture.conflict: different bytes were inserted under the \
                     content-addressed key {key:?}"
                )));
            }
            return Ok(());
        }
        self.database.textures.insert(key, bytes);
        Ok(())
    }

    /// Record one bundled external resource's bytes and its logical name
    /// (ADR-0040 D1/D4/D5).
    ///
    /// `key` must be the canonical form [`cad_domain::is_valid_bundled_key`]
    /// accepts for `kind`: it becomes the `resources/<kind>/<key>` archive entry
    /// name, so a path, a `..`, an uppercase extension or an extension outside
    /// the kind's allowlist is refused here, with the stable
    /// `resource.unsupported_kind` marker, rather than at save time.
    ///
    /// `name` is the **logical reference** the document uses (the spelling in
    /// `Style.resource_keys`/`Text.font`, `cad:Image.file`, or
    /// `ExternalReference.name`); it must be non-empty and is stored verbatim.
    /// `(kind, name)` is unique (ADR-0040 D4): a second distinct key claiming an
    /// already-used name is refused with `resource.name_conflict`, because the
    /// name would then resolve to two different payloads.
    ///
    /// Re-inserting the **same** key with the same bytes and the same name is
    /// idempotent (the key is content-addressed); different bytes under one key
    /// are a caller bug and are refused instead of silently replacing the asset
    /// — exactly [`Self::insert_texture`]'s rule. An empty payload is refused
    /// too: a zero-byte font or image can never resolve, so storing it would turn
    /// "the resource is missing" into a decode failure with no origin.
    ///
    /// There is **no budget policy in the database**: the collector owns
    /// `resource.budget.*` (ADR-0040 D10) before it calls this.
    pub fn insert_resource(
        &mut self,
        kind: BundledKind,
        key: &str,
        name: &str,
        bytes: Arc<[u8]>,
    ) -> CadResult<()> {
        if !cad_domain::is_valid_bundled_key(kind, key) {
            return Err(CadError::InvalidInput(format!(
                "{}: {kind} {key:?} is not a valid bundled-resource key (expected \
                 <32 lowercase hex>.<ext> with an extension this kind allows)",
                cad_domain::ymodel::markers::RESOURCE_UNSUPPORTED_KIND
            )));
        }
        if bytes.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "{}: {kind} {key:?} has no bytes",
                cad_domain::ymodel::markers::RESOURCE_UNAVAILABLE
            )));
        }
        if name.trim().is_empty() {
            return Err(CadError::InvalidInput(format!(
                "resource.name: {kind} {key:?} has an empty logical name"
            )));
        }
        let id = (kind, key.to_string());
        if let Some(existing) = self.database.resources.get(&id) {
            if existing.as_ref() != bytes.as_ref() {
                return Err(CadError::Invariant(format!(
                    "resource.key: different bytes were inserted under the \
                     content-addressed key {kind}/{key:?}"
                )));
            }
            // Same bytes: only the name may still disagree, which would make one
            // payload reachable under a different logical reference.
            if self.database.resource_names.get(&id).map(String::as_str) != Some(name) {
                return Err(CadError::InvalidInput(format!(
                    "{}: {kind} {key:?} is already bound to a different logical name",
                    cad_domain::ymodel::markers::RESOURCE_NAME_CONFLICT
                )));
            }
            return Ok(());
        }
        if let Some(other_key) = self
            .database
            .resource_names
            .iter()
            .find(|((existing_kind, _), existing_name)| {
                *existing_kind == kind && existing_name.as_str() == name
            })
            .map(|(id, _)| id.1.as_str())
        {
            return Err(CadError::InvalidInput(format!(
                "{}: {kind} {name:?} is already bound to key {other_key:?}; rename one of them",
                cad_domain::ymodel::markers::RESOURCE_NAME_CONFLICT,
            )));
        }
        self.database.resources.insert(id.clone(), bytes);
        self.database.resource_names.insert(id, name.to_string());
        Ok(())
    }

    /// Record where a bundled resource was collected from (ADR-0040 D4/D13).
    ///
    /// The path is **diagnostic only** — it is written to
    /// `tables/resources.json` as `source` and never used for runtime
    /// resolution. `None`, or an empty/whitespace-only path, records **no**
    /// path and removes any previous entry, so "absent" and "no path" are one
    /// state (matching [`Self::set_code_page`]'s rule and keeping a
    /// snapshot → load → snapshot round trip a fixed point). The key must
    /// already be inserted: provenance for an absent resource would be
    /// unreachable, so that is a caller bug.
    pub fn set_resource_source(
        &mut self,
        kind: BundledKind,
        key: &str,
        source: Option<&str>,
    ) -> CadResult<()> {
        let id = (kind, key.to_string());
        if !self.database.resources.contains_key(&id) {
            return Err(CadError::Invariant(format!(
                "resource source for unknown bundled resource {kind}/{key:?}"
            )));
        }
        match source.map(str::trim).filter(|value| !value.is_empty()) {
            Some(path) => {
                self.database
                    .resource_sources
                    .insert(id, Some(path.to_string()));
            }
            None => {
                self.database.resource_sources.remove(&id);
            }
        }
        Ok(())
    }

    /// Drop one bundled resource and its name/source mapping.
    ///
    /// Returns `true` when the document held it. A save that saw this call must
    /// rewrite `tables/resources.json` and must not copy the byte entry through
    /// (ADR-0040 D7/D8), which is why the host records
    /// [`TableKind::Resources`](crate::TableKind::Resources) when it drops one.
    /// Dropping an absent key is a no-op, not an error: a collector re-running
    /// over a changed policy must not fail on the second pass.
    pub fn drop_resource(&mut self, kind: BundledKind, key: &str) -> bool {
        let id = (kind, key.to_string());
        self.database.resource_sources.remove(&id);
        self.database.resource_names.remove(&id);
        self.database.resources.remove(&id).is_some()
    }

    /// Set whether this document embeds external resources (ADR-0040 D3).
    ///
    /// Document state, not a carrier preference: it is written into
    /// `document.json` and seeded back on load. `false` (the default) is not
    /// written at all, so a document that packs nothing keeps the bytes of a
    /// minor ≤ 4 archive.
    pub fn set_embed_resources(&mut self, embed: bool) {
        self.database.embed_resources = embed;
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

    /// Replace the set of entities the source file baked as hidden
    /// (`EntityCommon::invisible`).
    ///
    /// This is a **display** side table, not a re-evaluation: it covers
    /// top-level model/paper-space entities and block members alike. It is a
    /// whole-set replacement (not an accumulation), so a re-registration never
    /// leaves a stale hidden row; duplicates collapse and an empty iterator
    /// clears the set.
    ///
    /// Ids are not checked here because the importer calls this once after all
    /// entity rows are pushed; [`Self::finish`] rejects an id that no entity row
    /// exists for. A dangling id would be an unreachable hidden row.
    pub fn set_hidden_entities(
        &mut self,
        entities: impl IntoIterator<Item = EntityId>,
    ) -> CadResult<()> {
        self.database.hidden_entities = entities.into_iter().collect();
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
        // Every baked-hidden id must name an existing entity row: a dangling row
        // could never hide anything and would silently go stale.
        for id in &self.database.hidden_entities {
            if !self.database.entities.contains_key(id) {
                self.errors.push(format!(
                    "baked-hidden entity {id:?} does not exist in the drawing"
                ));
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
        // Bundled resources are two maps that must describe the same set: a
        // payload without a logical name could never be referenced, and the two
        // writers (`insert_resource`/`drop_resource`) are the only ones allowed
        // to touch either, so a mismatch here is a caller bug.
        for id in self.database.resources.keys() {
            if !self.database.resource_names.contains_key(id) {
                self.errors
                    .push(format!("bundled resource {id:?} has no logical name"));
            }
        }
        for id in self.database.resource_names.keys() {
            if !self.database.resources.contains_key(id) {
                self.errors
                    .push(format!("bundled resource name for {id:?} has no payload"));
            }
        }
        // BIM data is optional, but whatever an element references must exist
        // and every budget must hold (ADR-0042 D3/D5): a dangling level, a
        // wrong-kind reference or an over-budget table would make the stored
        // document lie. The check itself lives in `bim.rs` (new code, new
        // module); this line is the only wiring here.
        if let Err(error) = crate::bim::validate_all(&self.database) {
            self.errors.push(error.to_string());
        }
        if !self.errors.is_empty() {
            let joined = self.errors.join("; ");
            return Err(CadError::Invariant(format!(
                "import validation failed: {joined}"
            )));
        }
        // Identity counters are document state (ADR-0037 D5), not derived data.
        //
        // A load path seeds them from the document it opened; they are adopted
        // verbatim and only validated, because rebuilding them from row maxima
        // would hand a deleted id to the next created object. The import path
        // has no persisted counters yet, so it rebuilds them from the largest
        // id in the finished database.
        if let Some(seeded) = self.seeded_counters {
            self.database.adopt_id_counters(&seeded)?;
            return Ok(self.database);
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
        // Initialise the layout-id allocator the same way. `max(1)` keeps
        // `LayoutId(0)` — the reserved synthetic model row — out of the
        // allocator.
        let max_layout = self.database.layouts.keys().map(|l| l.0).max().unwrap_or(0);
        self.database.next_layout_id = max_layout.saturating_add(1).max(1);
        // The three style tables allocate the same way: from the largest imported
        // id, so a first post-import create cannot collide and a deleted id is
        // never handed out again.
        let max_style = self.database.styles.keys().map(|s| s.0).max().unwrap_or(0);
        self.database.next_style_id = max_style.saturating_add(1);
        let max_dim_style = self
            .database
            .dim_styles
            .keys()
            .map(|s| s.0)
            .max()
            .unwrap_or(0);
        self.database.next_dim_style_id = max_dim_style.saturating_add(1);
        let max_mleader_style = self
            .database
            .mleader_styles
            .keys()
            .map(|s| s.0)
            .max()
            .unwrap_or(0);
        self.database.next_mleader_style_id = max_mleader_style.saturating_add(1);
        // The annotation-scale table allocates the same way, so a post-import
        // `SCALELISTEDIT ADD` can never collide with an imported row.
        let max_scale = self.database.scales.keys().map(|s| s.0).max().unwrap_or(0);
        self.database.next_scale_id = max_scale.saturating_add(1);
        // Linetypes allocate the same way on the import path. A load never
        // reaches here: its persisted `next_linetype_id` is adopted above, so a
        // create → delete → create that was saved and reopened still gets a
        // fresh id instead of the deleted maximum.
        let max_linetype = self
            .database
            .linetypes
            .keys()
            .map(|s| s.0)
            .max()
            .unwrap_or(0);
        self.database.next_linetype_id = max_linetype.saturating_add(1);
        Ok(self.database)
    }
}
