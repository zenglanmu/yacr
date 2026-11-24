//! Building a snapshot from a database, and hydrating a database from one.
//!
//! This is the only place that knows both sides: element rows and table rows in,
//! [`super::DocumentSnapshot`] out (and back). The store never sees a database
//! and the database never sees an archive, per ADR-0037 D1.
//!
//! # What the header may not say
//!
//! [`DrawingDatabase::snapshot`] takes the unit context, the document identity,
//! the resource keys and the revision from its caller, but it takes the database
//! id and the nine identifier counters from `self`. A header that disagreed with
//! the rows it describes would make a later load either refuse (a counter below
//! the row high-water mark) or mint an id that aliases a row — so the caller is
//! never allowed to supply them.
//!
//! # Models (D7)
//!
//! Models are derived from the in-memory spaces, not allocated: model space, one
//! `PaperSpace` per stored layout (never `LayoutId(0)`, which is synthetic model
//! space), one `BlockModel` per stored block. v1 emits **no** definition model:
//! style/layer/linetype/scale/layout/block rows are table rows (D7's explicit
//! storage deviation), so a `Definitions` model would be empty.
//!
//! # Hydration order
//!
//! Rows go in dependency order (layers before layer attributes, blocks before
//! their metadata, layouts before plot settings) and every refusal is surfaced:
//! a row that does not decode, or a reference that dangles, fails the load with
//! [`SnapshotError::invalid`] instead of dropping the row. `finish()` is the last
//! word on referential integrity; the header variables are applied after it
//! because they are document scalars, not rows.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::ymodel::{shard_of, DocumentHeader, ModelKey, PersistentIdOf, Shard};
use cad_domain::{CadResult, DocumentIdentity, Revision, SpaceId, UnitContext};

use super::bim;
use super::configurations;
use super::elements;
use super::features;
use super::groups;
use super::parameters;
use super::relations;
use super::relations_in_context;
use super::resources;
use super::tables;
use super::variables;
use super::views;
use super::{
    table_entry, DocumentSnapshot, ElementSnapshot, ModelKind, ModelRecord, ShardSnapshot,
    SnapshotError,
};
use crate::drawing::DrawingDatabase;
use crate::scope::TableKind;
use crate::DrawingDatabaseBuilder;

impl DrawingDatabase {
    /// Encode this database as a [`DocumentSnapshot`].
    ///
    /// `units`, `identity`, `resource_keys` and `revision` are host/session state
    /// the database does not hold; the database id, the identifier counters and
    /// the resource policy are **not** parameters, because they must describe the
    /// document (the policy is document state, ADR-0040 D3).
    pub fn snapshot(
        &self,
        units: UnitContext,
        identity: DocumentIdentity,
        resource_keys: Vec<String>,
        revision: Revision,
    ) -> Result<DocumentSnapshot, SnapshotError> {
        let header = DocumentHeader {
            database: self.id,
            units,
            identity,
            resource_keys,
            embed_resources: self.embed_resources,
            application_domain: self.application_domain,
            next: self.id_counters(),
            variables: variables::encode(self)?,
        };
        // `tables::encode` writes the row tables; the resource mapping lives in
        // its own codec (`snapshot/resources.rs`) for the file-size gate and is
        // still one `tables/resources.json` row array here.
        //
        // ADR-0040 D12: that table exists **only** when the document really
        // carries resource bytes. A document that packs nothing writes no table
        // at all, so a pre-0040 reader sees the minor-4 archive it expects
        // instead of refusing a table it cannot model (an empty table is still a
        // `tables/resources.json` entry). `embed_resources` alone is a
        // `document.json` scalar and does not earn a table: the flag is written
        // there, so the archive is minor 5 without a mapping nothing maps.
        let mut tables = tables::encode(self)?;
        if !self.resources.is_empty() {
            tables.set(TableKind::Resources, resources::encode(self)?);
        }
        // ADR-0042 D3: the definition table exists **only** when it holds rows.
        // An unconditional table would make every document a minor-6 archive
        // that an older build refuses, for no data. Per-element BIM blocks are
        // written by the element codec, in the shard rows themselves (D2).
        if !self.bim_definitions.is_empty() {
            tables.set(TableKind::BimDefinitions, bim::encode_definitions(self)?);
        }
        // ADR-0042 D3 tail/D12: the relation table exists **only** when it holds
        // rows. Same rule as the definition table: no data, no table, no minor
        // bump. When it is written the document is minor 7 (`has_bim`).
        if !self.bim_relations.is_empty() {
            tables.set(TableKind::BimRelations, relations::encode_relations(self)?);
        }
        // ADR-0043 D11: the parameter table exists **only** when it holds rows,
        // the same rule as the BIM tables. An unconditional table would make
        // every document a minor-8 archive an older build refuses, for no data.
        // Per-element bindings are written by the element codec, in the shard
        // rows themselves (D7).
        if !self.parameters.is_empty() {
            tables.set(TableKind::Parameters, parameters::encode_parameters(self)?);
        }
        // ADR-0043 P1: configuration overlays share the expression tier and are
        // written only when they hold rows. The base parameter table above stays
        // the authority; a configuration is derived and never written back.
        if !self.param_configs.is_empty() {
            tables.set(
                TableKind::Configurations,
                configurations::encode_configurations(self)?,
            );
        }
        // ADR-0043 P2/P3: feature definitions and self-authored relations are
        // written only when they hold rows; each table owns its own minor tier.
        if !self.features.is_empty() {
            tables.set(TableKind::Features, features::encode_features(self)?);
        }
        if !self.relations.is_empty() {
            tables.set(
                TableKind::Relations,
                relations_in_context::encode_relations_in_context(self)?,
            );
        }
        // Persisted entity groups are written only when they hold rows, the same
        // rule as the other side tables. A document without groups keeps its
        // lower minor; a document with them is minor 17.
        if !self.groups.is_empty() {
            tables.set(TableKind::Groups, groups::encode_groups(self)?);
        }
        // Named views are document content and get the same "no rows, no table"
        // rule as the other side tables: a document without views stays at the
        // minor its other features imply.
        if !self.named_views.is_empty() {
            tables.set(TableKind::Views, views::encode_views(self));
        }
        Ok(DocumentSnapshot {
            header,
            revision,
            models: self.model_records(),
            shards: self.shard_snapshots()?,
            tables,
            // Cloned `Arc`s, not copied bytes: one save shares the document's
            // texture buffers with the snapshot it hands to the store.
            textures: self.textures.clone(),
            resources: self.resources.clone(),
            resource_names: self.resource_names.clone(),
            resource_sources: self.resource_sources.clone(),
        })
    }

    /// Hydrate a new database from a snapshot.
    ///
    /// The revision is seeded from the snapshot (ADR-0037 D2) and the identifier
    /// counters are adopted verbatim (D5), so a delete-then-reopen-then-insert can
    /// never hand out a deleted id.
    pub fn from_snapshot(snapshot: &DocumentSnapshot) -> Result<DrawingDatabase, SnapshotError> {
        let mut builder = DrawingDatabaseBuilder::new(snapshot.header.database);
        builder.set_revision(snapshot.revision);
        builder.seed_identifiers(snapshot.header.next);
        builder.set_embed_resources(snapshot.header.embed_resources);
        builder.set_application_domain(snapshot.header.application_domain);

        let tables = tables::decode(&snapshot.tables)?;
        let resource_tables = resources::decode(&snapshot.tables)?;
        // The definition table is decoded and installed before the element loop,
        // so `set_entity_bim` never has to defer a reference. Its own strictness
        // (duplicate ids, budgets, kind prefixes) belongs to the codec.
        let bim_definitions = bim::decode_definitions(&snapshot.tables)?;
        if !bim_definitions.is_empty() {
            checked(
                builder.set_bim_definitions(bim_definitions),
                "tables/bim-definitions.json",
            )?;
        }
        // Relation rows have no dangling-reference rule (an endpoint may name a
        // product with no geometry row), so they are decoded and installed
        // before the element loop as well; the codec owns shape and budget.
        let bim_relations = relations::decode_relations(&snapshot.tables)?;
        if !bim_relations.is_empty() {
            checked(
                builder.set_bim_relations(bim_relations),
                table_entry(TableKind::BimRelations),
            )?;
        }
        // Parameter rows are decoded and installed before the element loop, so
        // `set_param_bindings` never has to defer a reference. The codec owns
        // row shape, duplicates, dangling references/cycles and the budgets; the
        // builder's `finish()` owns slot validity against the element's type.
        let parameters = parameters::decode_parameters(&snapshot.tables)?;
        // Configuration rows are validated against the decoded base table, so
        // they are decoded before the builder consumes it.
        let configs = configurations::decode_configurations(&snapshot.tables, &parameters)?;
        let features = features::decode_features(&snapshot.tables)?;
        let relations = relations_in_context::decode_relations_in_context(&snapshot.tables)?;
        // Group rows are decoded before the element loop; member existence is
        // checked by the builder's `finish()` gate, where every entity exists.
        let group_rows = groups::decode_groups(&snapshot.tables)?;
        let named_views = views::decode_views(&snapshot.tables)?;
        if !parameters.is_empty() {
            checked(
                builder.set_parameters(parameters),
                table_entry(TableKind::Parameters),
            )?;
        }
        if !configs.is_empty() {
            checked(
                builder.set_param_configs(configs),
                table_entry(TableKind::Configurations),
            )?;
        }
        if !features.is_empty() {
            checked(
                builder.set_features(features),
                table_entry(TableKind::Features),
            )?;
        }
        if !relations.is_empty() {
            checked(
                builder.set_relations(relations),
                table_entry(TableKind::Relations),
            )?;
        }
        if !group_rows.is_empty() {
            checked(
                builder.set_groups(group_rows),
                table_entry(TableKind::Groups),
            )?;
        }
        if !named_views.is_empty() {
            checked(
                builder.set_named_views(named_views),
                table_entry(TableKind::Views),
            )?;
        }

        for layer in tables.layers.values() {
            checked(
                builder.insert_layer(layer.clone()),
                format!("tables/layers.json layer {}", layer.id.0),
            )?;
        }
        for (id, attributes) in &tables.layer_attributes {
            checked(
                builder.set_layer_attributes(*id, attributes.clone()),
                format!("tables/layers.json layer {}", id.0),
            )?;
        }

        for block in tables.blocks.values() {
            checked(
                builder.insert_block(block.clone()),
                format!("tables/blocks.json block {}", block.id.0),
            )?;
        }
        // The registration order is *state* (`block_by_name` takes the last
        // registered duplicate), so it is replayed in the persisted order.
        for (name, id) in &tables.block_name_order {
            let metadata = tables.block_metadata.get(id).ok_or_else(|| {
                SnapshotError::invalid(format!(
                    "tables/blocks.json: name registration {name:?} has no metadata row for \
                     block {}",
                    id.0
                ))
            })?;
            checked(
                builder.set_block_metadata(*id, &metadata.name, metadata.base_point),
                format!("tables/blocks.json block {}", id.0),
            )?;
        }
        for (handle, id) in &tables.block_source_handles {
            if *handle == 0 {
                return Err(SnapshotError::invalid(format!(
                    "tables/blocks.json: block {} carries the reserved source handle 0",
                    id.0
                )));
            }
            if !tables.blocks.contains_key(id) {
                return Err(SnapshotError::invalid(format!(
                    "tables/blocks.json: source handle {handle} points at unknown block {}",
                    id.0
                )));
            }
            builder.set_block_source_handle(*id, *handle);
        }

        for layout in tables.layouts.values() {
            checked(
                builder.insert_layout(layout.clone()),
                format!("tables/layouts.json layout {}", layout.id.0),
            )?;
        }
        for settings in tables.plot_settings.values() {
            checked(
                builder.set_plot_settings(settings.clone()),
                format!("tables/plot-settings.json layout {}", settings.layout.0),
            )?;
        }
        for style in tables.styles.values() {
            checked(
                builder.insert_style(style.clone()),
                format!("tables/styles.json style {}", style.id.0),
            )?;
        }
        for style in tables.dim_styles.values() {
            checked(
                builder.insert_dim_style(style.clone()),
                format!("tables/dimstyles.json style {}", style.id.0),
            )?;
        }
        for style in tables.mleader_styles.values() {
            checked(
                builder.insert_mleader_style(style.clone()),
                format!("tables/mleaderstyles.json style {}", style.id.0),
            )?;
        }
        for linetype in tables.linetypes.values() {
            checked(
                builder.insert_linetype(linetype.clone()),
                format!("tables/linetypes.json linetype {}", linetype.id.0),
            )?;
        }
        for scale in tables.scales.values() {
            checked(
                builder.insert_scale(scale.clone()),
                format!("tables/scales.json scale {}", scale.id.0),
            )?;
        }
        for reference in tables.external_references.values() {
            checked(
                builder.insert_external_reference(reference.clone()),
                format!("tables/xrefs.json reference {:?}", reference.name),
            )?;
        }
        // Document-owned textures (ADR-0038 D3): the snapshot is the authority, so
        // every entry here is restored verbatim. `insert_texture` re-checks the key
        // form and the non-empty payload, which makes a corrupt archive entry an
        // `invalid_document` refusal rather than an unusable asset in memory.
        for (key, bytes) in &snapshot.textures {
            checked(
                builder.insert_texture(key.clone(), std::sync::Arc::clone(bytes)),
                format!("textures/{key}"),
            )?;
        }
        // Bundled external resources (ADR-0040 D5). On load the **table** is the
        // mapping authority and the snapshot's byte map is the payload authority,
        // so the two refusals below are the whole integrity rule: a listed name
        // with no bytes is a dangling reference, and bytes with no listed name
        // would be an asset nothing can reach (and would be dropped by the next
        // save). Neither is repaired by guessing.
        for ((kind, key), name) in &resource_tables.0 {
            let Some(bytes) = snapshot.resources.get(&(*kind, key.clone())) else {
                return Err(SnapshotError::invalid(format!(
                    "{}: {kind}/{key} has no resources/{kind}/{key} payload entry",
                    table_entry(TableKind::Resources)
                )));
            };
            checked(
                builder.insert_resource(*kind, key, name, std::sync::Arc::clone(bytes)),
                format!("resources/{kind}/{key}"),
            )?;
            let source = resource_tables
                .1
                .get(&(*kind, key.clone()))
                .cloned()
                .flatten();
            checked(
                builder.set_resource_source(*kind, key, source.as_deref()),
                format!("resources/{kind}/{key}"),
            )?;
        }
        for (kind, key) in snapshot.resources.keys() {
            if !resource_tables.0.contains_key(&(*kind, key.clone())) {
                return Err(SnapshotError::invalid(format!(
                    "resources/{kind}/{key} is not listed in {}",
                    table_entry(TableKind::Resources)
                )));
            }
        }

        let mut hidden = BTreeSet::new();
        for shard in &snapshot.shards {
            for element in &shard.elements {
                let elements::DecodedElement {
                    entity,
                    attributes,
                    invisible,
                    bim,
                    params,
                } = elements::decode(&element.payload)?;
                // The row id is the shard key; `decode` reads the copy inside the
                // payload. They must agree or one of the two writers is lying.
                if entity.id != element.id {
                    return Err(SnapshotError::invalid(format!(
                        "elements: shard row id {} does not match its payload id {}",
                        element.id.0, entity.id.0
                    )));
                }
                // `finish` checks paper-space targets; a block-space target is not
                // validated there, and a dangling one would be unreachable.
                if let SpaceId::Block(block) = &entity.space {
                    if !tables.blocks.contains_key(block) {
                        return Err(SnapshotError::invalid(format!(
                            "elements: entity {} is in block-space of unknown block {}",
                            entity.id.0, block.0
                        )));
                    }
                }
                let id = entity.id;
                checked(
                    builder.insert_entity(entity),
                    format!("elements: entity {}", id.0),
                )?;
                if let Some(attributes) = attributes {
                    checked(
                        builder.set_entity_render_attributes(id, attributes),
                        format!("elements: entity {}", id.0),
                    )?;
                }
                if invisible {
                    hidden.insert(id);
                }
                if let Some(bim) = bim {
                    checked(
                        builder.set_entity_bim(id, bim),
                        format!("elements: entity {}", id.0),
                    )?;
                }
                if let Some(params) = params {
                    checked(
                        builder.set_param_bindings(id, params),
                        format!("elements: entity {}", id.0),
                    )?;
                }
            }
        }
        checked(
            builder.set_hidden_entities(hidden),
            "elements: the baked-hidden set".to_string(),
        )?;

        let mut database = checked(builder.finish(), "snapshot rows")?;
        variables::apply_to(&mut database, &snapshot.header.variables)?;
        // ADR-0043 D7: opening re-derives bound geometry from the table. A
        // failure here means the archive contradicted its own decoded rows; it
        // is surfaced as an invalid document, never as an empty success.
        crate::drawing::param::reconcile_loaded_parameters(&mut database)
            .map_err(|error| SnapshotError::invalid(format!("{error}")))?;
        Ok(database)
    }

    /// The model metadata list: model space, every stored layout, every block.
    fn model_records(&self) -> Vec<ModelRecord> {
        let mut models = vec![ModelRecord {
            key: ModelKey::ModelSpace,
            kind: ModelKind::ModelSpace,
            name: None,
            owner: None,
        }];
        for layout in self.layouts.values().filter(|layout| layout.id.0 != 0) {
            models.push(ModelRecord {
                key: ModelKey::Paper(layout.id),
                kind: ModelKind::PaperSpace,
                name: Some(layout.name.clone()),
                owner: Some(layout.id.persistent()),
            });
        }
        for block in self.blocks.values() {
            models.push(ModelRecord {
                key: ModelKey::Block(block.id),
                kind: ModelKind::Block,
                name: self
                    .block_metadata
                    .get(&block.id)
                    .map(|metadata| metadata.name.clone()),
                owner: Some(block.id.persistent()),
            });
        }
        models
    }

    /// Every entity, grouped by `(model, shard)` and sorted inside each group.
    ///
    /// The shard rule is `seq / 2048` inside one model (D11), and ids are never
    /// reused, so a group is stable across saves. `BTreeMap` iteration makes the
    /// shard list deterministic without a second sort.
    fn shard_snapshots(&self) -> Result<Vec<ShardSnapshot>, SnapshotError> {
        let mut grouped: BTreeMap<(ModelKey, Shard), Vec<ElementSnapshot>> = BTreeMap::new();
        for entity in self.entities.values() {
            let model = ModelKey::from_space(entity.space.clone());
            let payload = elements::encode(
                entity,
                self.render_attributes.get(&entity.id),
                self.hidden_entities.contains(&entity.id),
                self.bim_elements.get(&entity.id),
                self.param_bindings.get(&entity.id),
            )?;
            grouped
                .entry((model, shard_of(entity.id.0)))
                .or_default()
                .push(ElementSnapshot {
                    id: entity.id,
                    payload,
                });
        }
        let mut shards = Vec::with_capacity(grouped.len());
        for ((model, shard), mut elements) in grouped {
            elements.sort_by_key(|element| element.id.0);
            shards.push(ShardSnapshot {
                model,
                shard,
                elements,
            });
        }
        Ok(shards)
    }
}

/// Name a builder refusal after the row that caused it.
fn checked<T>(result: CadResult<T>, context: impl std::fmt::Display) -> Result<T, SnapshotError> {
    result.map_err(|error| SnapshotError::invalid(format!("{context}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dim_style::DimStyle;
    use crate::entity::{
        DbEntity, DbObject, EntityColor, EntityLineWeight, EntityRenderAttributes,
    };
    use crate::mleader_style::MLeaderStyle;
    use crate::tables::{
        BlockDefinition, Layer, LayerAttributes, LayerColor, LayerLineWeight, Layout, LineType,
        PaperViewport, PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation,
        PlotSettingsRecord, PlotType, Scale as ScaleRow, Style,
    };
    use crate::xref::{ExternalReference, ExternalReferenceStatus};
    use crate::LinetypePattern;
    use cad_domain::{
        BlockId, Completeness, DatabaseId, DimStyleId, EntityId, LayerId, LayoutId, LinetypeId,
        MLeaderStyleId, ObjectId, Point3, ScaleId, SemanticGeometry, StyleId, TransactionId,
        Transform3,
    };
    use serde_json::Value;

    fn point(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn units() -> UnitContext {
        UnitContext::drawing_units()
    }

    fn identity() -> DocumentIdentity {
        DocumentIdentity::Temporary(1)
    }

    fn resource_keys() -> Vec<String> {
        vec!["arial.ttf".into()]
    }

    fn line(id: u128, layer: u128, space: SpaceId) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: Some(format!("{id:X}")),
            },
            id: EntityId(id),
            layer: LayerId(layer),
            space,
            geometry: SemanticGeometry::Line {
                start: point(0.0, 0.0, 0.0),
                end: point(10.0, 0.0, 0.0),
            },
            draw_order: 0,
        }
    }

    /// A database that touches every persisted surface: all ten tables, both
    /// non-default layer attributes, a block registered out of id order, two
    /// viewports, and header variables including a non-finite `$PDSIZE`.
    fn fixture() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(7));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_layer(Layer {
            id: LayerId(1),
            name: "Walls".into(),
            visible: false,
        })
        .unwrap();
        b.set_layer_attributes(
            LayerId(1),
            LayerAttributes {
                off: true,
                frozen: true,
                color: LayerColor {
                    aci: Some(1),
                    rgb: [255, 0, 0],
                },
                linetype: "DASHED".into(),
                lineweight: LayerLineWeight::Hundredths(35),
                opacity: 0.5,
                plot_style: Some("Normal".into()),
                ..LayerAttributes::default()
            },
        )
        .unwrap();
        b.insert_layer(Layer {
            id: LayerId(2),
            name: "Scratch".into(),
            visible: true,
        })
        .unwrap();

        b.insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(3)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![],
            dynamic_visibility: None,
        })
        .unwrap();
        // Registered out of id order: last-duplicate-wins lookup is order-dependent.
        b.set_block_metadata(BlockId(2), "Window", point(0.0, 0.0, 0.0))
            .unwrap();
        b.set_block_metadata(BlockId(1), "Door", point(1.5, 2.5, 0.0))
            .unwrap();
        b.set_block_source_handle(BlockId(1), 42);

        b.insert_layout(Layout {
            id: LayoutId(0),
            name: "Model".into(),
            viewports: vec![],
        })
        .unwrap();
        let viewport = |overrides: BTreeMap<LayerId, bool>| PaperViewport {
            clip: vec![
                point(0.0, 0.0, 0.0),
                point(100.0, 0.0, 0.0),
                point(100.0, 50.0, 0.0),
                point(0.0, 50.0, 0.0),
            ],
            model_to_paper: Transform3::translation(point(5.0, 7.0, 0.0)),
            completeness: Completeness::Partial(vec!["viewport.clip_partial".into()]),
            layer_overrides: overrides,
        };
        b.insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet".into(),
            viewports: vec![
                viewport(BTreeMap::new()),
                viewport(BTreeMap::from([(LayerId(1), true)])),
            ],
        })
        .unwrap();
        b.set_plot_settings(PlotSettingsRecord {
            layout: LayoutId(1),
            paper_size_name: "ISO_A3".into(),
            paper_width: 297.0,
            paper_height: 420.0,
            margins: PlotMargins::uniform(12.0),
            rotation: PlotRotation::Degrees90,
            scale_numerator: 1.0,
            scale_denominator: 100.0,
            plot_type: PlotType::Layout,
            paper_units: PlotPaperUnits::Millimeters,
            provenance: PlotProvenance::DefaultPage {
                reason: "plot.default_page".into(),
            },
        })
        .unwrap();

        b.insert_style(Style {
            id: StyleId(1),
            name: "Notes".into(),
            resource_keys: vec!["arial.ttf".into()],
        })
        .unwrap();
        let mut dim = DimStyle::standard_metric(DimStyleId(3));
        dim.name = "Fancy".into();
        dim.text_height = 3.5;
        b.insert_dim_style(dim).unwrap();
        let mut mleader = MLeaderStyle::standard_metric(MLeaderStyleId(4));
        mleader.name = "Callout".into();
        mleader.left_attachment = 3;
        b.insert_mleader_style(mleader).unwrap();
        b.insert_linetype(LineType {
            id: LinetypeId(1),
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            complex: false,
        })
        .unwrap();
        b.insert_scale(ScaleRow {
            id: ScaleId(1),
            name: "1:100".into(),
            paper_units: 1.0,
            drawing_units: 100.0,
        })
        .unwrap();
        b.insert_external_reference(ExternalReference {
            name: "Site".into(),
            raw_path: "site.dwg".into(),
            resolved_path: Some("site.dwg".into()),
            block: Some(BlockId(2)),
            overlay: false,
            depth: 0,
            status: ExternalReferenceStatus::Missing,
            reason: Some("xref.missing".into()),
        })
        .unwrap();

        // One entity in each space; the block member is on the second layer.
        b.insert_entity(line(1, 0, SpaceId::Model)).unwrap();
        b.insert_entity(line(2, 0, SpaceId::Paper(LayoutId(1))))
            .unwrap();
        b.insert_entity(line(3, 1, SpaceId::Block(BlockId(1))))
            .unwrap();
        let mut attributes = EntityRenderAttributes {
            color: EntityColor::Explicit([10, 20, 30]),
            lineweight: EntityLineWeight::Explicit(0.25),
            ..EntityRenderAttributes::default()
        };
        attributes.linetype_scale = f64::NAN;
        b.set_entity_render_attributes(EntityId(1), attributes)
            .unwrap();
        b.set_hidden_entities([EntityId(2)]).unwrap();

        b.set_linetype_scale(2.5).unwrap();
        b.set_point_display_mode(3);
        b.set_point_display_size(f64::NAN);
        b.set_code_page(Some("ANSI_936"));
        b.set_active_annotation_scale("1:100", 0.01).unwrap();

        let mut db = b.finish().unwrap();
        db.set_insertion_base(point(5.0, 6.0, 0.0), TransactionId(1), "test")
            .unwrap();
        db.set_model_limits(
            Some([0.0, 0.0]),
            Some([420.0, 297.0]),
            TransactionId(2),
            "test",
        )
        .unwrap();
        db
    }

    fn snapshot_of(db: &DrawingDatabase, revision: Revision) -> DocumentSnapshot {
        db.snapshot(units(), identity(), resource_keys(), revision)
            .expect("snapshot")
    }

    #[test]
    fn snapshot_round_trip_is_a_fixed_point() {
        let db = fixture();
        let a = snapshot_of(&db, Revision(7));
        assert_eq!(
            a.header.next,
            db.id_counters(),
            "the header counters are the database's own"
        );

        let loaded = DrawingDatabase::from_snapshot(&a).expect("hydrate a");
        assert_eq!(loaded.revision(), Revision(7));
        let b = snapshot_of(&loaded, Revision(7));
        assert_eq!(a, b, "snapshot -> database -> snapshot is a fixed point");
        assert!(
            a.bytes_equal(&b),
            "the fixed point holds for the payload bytes too, not just their lengths"
        );

        // Spot-check the values a lenient codec could have flattened.
        assert!(loaded.point_display_size.is_nan());
        assert_eq!(loaded.linetype_scale, 2.5);
        assert_eq!(loaded.point_display_mode, 3);
        assert_eq!(loaded.insertion_base, Some(point(5.0, 6.0, 0.0)));
        assert_eq!(loaded.limits_min, Some([0.0, 0.0]));
        assert_eq!(loaded.limits_max, Some([420.0, 297.0]));
        assert_eq!(loaded.code_page.as_deref(), Some("ANSI_936"));
        let active = loaded
            .active_annotation_scale
            .as_ref()
            .expect("the active annotation scale survives");
        assert_eq!(active.name, "1:100");
        assert!(active.named);
        assert!(loaded.render_attributes[&EntityId(1)]
            .linetype_scale
            .is_nan());
        assert!(loaded.hidden_entities.contains(&EntityId(2)));
        assert!(loaded.block_source_handles.contains_key(&42));
    }

    #[test]
    fn a_deleted_high_water_id_is_never_reused_after_reopen() {
        let mut db = fixture();
        // `Scratch` is the highest layer id and no entity references it.
        db.delete_layer(LayerId(2), TransactionId(3), "test")
            .expect("delete the highest layer");
        let a = snapshot_of(&db, Revision(7));

        let mut loaded = DrawingDatabase::from_snapshot(&a).expect("hydrate");
        assert_eq!(loaded.id_counters(), a.header.next);
        let (id, _) = loaded
            .create_layer("After", TransactionId(4), "test")
            .expect("create a layer after reopen");
        assert_eq!(id, LayerId(3), "the counter kept its high-water mark");
        assert_ne!(id, LayerId(2), "the deleted id must not be reused");
    }

    #[test]
    fn block_name_order_survives_in_registration_order() {
        let loaded =
            DrawingDatabase::from_snapshot(&snapshot_of(&fixture(), Revision(7))).expect("hydrate");
        assert_eq!(
            loaded.block_name_order,
            vec![
                ("Window".to_string(), BlockId(2)),
                ("Door".to_string(), BlockId(1)),
            ],
            "the order is state: last-duplicate-wins depends on it"
        );
    }

    #[test]
    fn header_variables_are_strict_in_both_directions() {
        let a = snapshot_of(&fixture(), Revision(7));

        let mut unknown = a.clone();
        unknown.header.variables["future_variable"] = Value::from(1);
        let error = DrawingDatabase::from_snapshot(&unknown).expect_err("unknown variable");
        assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
        assert!(error.detail.contains("future_variable"), "{}", error.detail);

        let mut missing = a.clone();
        missing.header.variables = match &missing.header.variables {
            Value::Object(map) => {
                let mut map = map.clone();
                map.remove("code_page");
                Value::Object(map)
            }
            other => other.clone(),
        };
        let error = DrawingDatabase::from_snapshot(&missing).expect_err("missing variable");
        assert!(error.detail.contains("code_page"), "{}", error.detail);

        let mut wrong_type = a.clone();
        wrong_type.header.variables["linetype_scale"] = Value::from("2.5");
        let error = DrawingDatabase::from_snapshot(&wrong_type).expect_err("wrong type");
        assert!(error.detail.contains("linetype_scale"), "{}", error.detail);
    }

    #[test]
    fn a_bare_sequence_identifier_is_refused() {
        let mut a = snapshot_of(&fixture(), Revision(7));
        let rows = a
            .tables
            .rows
            .get_mut(&crate::scope::TableKind::Layers)
            .expect("the layer table is written");
        let row = rows
            .iter_mut()
            .find(|row| row["id"].as_i64() == Some(LayerId(1).persistent().raw()))
            .expect("the Walls row");
        // A bare sequence collides across families on disk; the codec refuses it.
        row["id"] = Value::from(1);
        let error = DrawingDatabase::from_snapshot(&a).expect_err("a bare id is not persistent");
        assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
        assert!(
            error.detail.contains("not a persistent id"),
            "{}",
            error.detail
        );
    }
}
