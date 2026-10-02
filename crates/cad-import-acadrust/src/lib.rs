//! Sole acadrust integration boundary. No third-party entity escapes.
//!
//! Spec v2.0 §7.1: read bytes → acadrust parse → walk model/paper space and
//! block definitions → proxy supplement → normalise into the database. The
//! library is never modified and nothing here assumes internal hooks.

use std::collections::{BTreeSet, HashMap};
use std::io::Cursor;
use std::sync::Arc;

use acadrust::entities::EntityCommon;
use acadrust::entities::{
    AttachmentPoint, BoundaryEdge, DimensionBase, Hatch, TextHorizontalAlignment,
    TextVerticalAlignment,
};
use acadrust::{DwgReadOptions, DwgReader, EntityType, ReadStats};
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, EntityColor,
    EntityLineWeight, EntityRenderAttributes, EntityTransparency, Layer, Layout, PaperViewport,
    Style,
};
use cad_domain::*;
use cad_geometry::{arbitrary_axis, tessellate_bspline, PatternLine, TessellationParams};
use cad_proxy::{DecodeLimits, ProxyPlayer, ProxySource};

mod solid;
pub use solid::{
    acis_exchange, acis_raw_payload, sab_to_brep, sat_to_brep, solid_exchange_from_entity,
};

/// Bounds applied to an untrusted drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportLimits {
    pub max_file_bytes: usize,
    pub max_entities: usize,
    pub max_block_depth: usize,
}

impl Default for ImportLimits {
    fn default() -> Self {
        ImportLimits {
            max_file_bytes: 512 * 1024 * 1024,
            max_entities: 2_000_000,
            max_block_depth: 32,
        }
    }
}

pub struct ImportRequest {
    pub document: DocumentId,
    pub database: DatabaseId,
    pub bytes: Arc<[u8]>,
    pub limits: ImportLimits,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    Reading,
    Parsing,
    Normalizing,
    Proxy,
    Indexing,
    Representations,
    Uploading,
}

#[derive(Debug, Clone)]
pub struct ImportReport {
    pub identity: DocumentIdentity,
    pub capabilities: Vec<EntityCapability>,
    pub completeness: Completeness,
    pub diagnostics: Vec<Diagnostic>,
    /// Wall-clock parse/normalise time in milliseconds, when measured.
    pub parse_ms: Option<f64>,
}

impl ImportReport {
    /// Short human-facing completeness label for the UI status line.
    pub fn completeness_label(&self) -> String {
        match &self.completeness {
            Completeness::Complete => "完整".to_string(),
            Completeness::Partial(items) => format!("部分（{} 项）", items.len()),
            Completeness::Missing(items) => format!("缺失（{} 项）", items.len()),
            Completeness::Unverified => "未验证".to_string(),
        }
    }
}

pub struct ImportedDrawing {
    pub database: DrawingDatabase,
    pub units: UnitContext,
    pub report: ImportReport,
}

pub trait Importer {
    fn import(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<ImportedDrawing>;
}

/// The acadrust-backed importer.
#[derive(Default)]
pub struct AcadrustImporter {
    pub proxy_limits: DecodeLimits,
}

impl AcadrustImporter {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Importer for AcadrustImporter {
    fn import(
        &self,
        request: &ImportRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> CadResult<ImportedDrawing> {
        if cancelled() {
            return Err(CadError::Cancelled);
        }
        if request.bytes.len() > request.limits.max_file_bytes {
            return Err(CadError::InvalidInput(format!(
                "file is {} bytes, over the {} byte import limit",
                request.bytes.len(),
                request.limits.max_file_bytes
            )));
        }
        // A DWG begins with an "AC10xx" version signature. Refuse anything else
        // instead of handing arbitrary bytes to a failsafe parser that would
        // return an empty document and look like success.
        if !looks_like_dwg(&request.bytes) {
            return Err(CadError::CorruptData(
                "input is not a DWG (missing AC10xx signature); DXF is not handled by this importer".into(),
            ));
        }
        let started = std::time::Instant::now();
        let identity = compute_identity(&request.bytes);

        let mut reader = DwgReader::from_stream(Cursor::new(request.bytes.to_vec()));
        reader.options = DwgReadOptions { failsafe: true };
        let outcome = reader
            .read_with_stats()
            .map_err(|e| CadError::CorruptData(format!("DWG read failed: {e}")))?;
        if cancelled() {
            return Err(CadError::Cancelled);
        }

        let builder = ImporterBuilder::new(request, &outcome.document, outcome.stats, identity);
        builder.run().map(|mut drawing| {
            drawing.report.parse_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            drawing
        })
    }
}

struct ImporterBuilder<'a> {
    request: &'a ImportRequest,
    acad: &'a acadrust::CadDocument,
    stats: ReadStats,
    identity: DocumentIdentity,
    builder: DrawingDatabaseBuilder,
    diagnostics: Vec<Diagnostic>,
    capabilities: HashMap<String, EntityCapability>,
    layer_ids: HashMap<String, LayerId>,
    /// Effective display opacity of each layer, resolved from its DWG
    /// transparency (0 = opaque, 255 = transparent).
    layer_transparency: HashMap<LayerId, f32>,
    /// Resolved sRGB colour of each layer, used to substitute `ByLayer` on an
    /// entity (an acadrust `Color::Index` is resolved through its ACI table).
    layer_colors: HashMap<LayerId, [u8; 3]>,
    /// Resolved lineweight of each layer in millimetres.
    layer_lineweights: HashMap<LayerId, f32>,
    style_ids: HashMap<String, StyleId>,
    /// Lower-cased style name -> primary font file name.
    style_fonts: HashMap<String, String>,
    block_ids: HashMap<String, BlockId>,
    /// Block name -> the block's insertion base point. An INSERT maps the
    /// block's base point onto its insertion point, so expansion subtracts it
    /// (`world = insert * (local - base)`, audit B31).
    block_base_points: HashMap<String, Point3>,
    layout_ids: HashMap<String, LayoutId>,
    next_entity: u128,
    next_object: u128,
    entity_total: usize,
    dropped: usize,
    proxy: ProxyPlayer,
    model_layout: LayoutId,
    /// Per block: the display status of each child, with the referenced block
    /// when the child is an INSERT (so nesting can be resolved).
    block_members: HashMap<BlockId, Vec<BlockMember>>,
    /// Resolved render status of each block definition (weakest child).
    block_status: HashMap<BlockId, SupportStatus>,
    /// Weakest render status and offending types over model-space entities.
    model_render: SupportStatus,
    model_drawable: bool,
    model_render_types: BTreeSet<String>,
}

/// A block child classified for display, before nesting is resolved.
#[derive(Debug, Clone)]
struct BlockMember {
    render: SupportStatus,
    /// Block definitions referenced by this child (one per array cell), so
    /// nesting resolves through MINSERT arrays too.
    insert_blocks: Vec<BlockId>,
}

impl<'a> ImporterBuilder<'a> {
    fn new(
        request: &'a ImportRequest,
        acad: &'a acadrust::CadDocument,
        stats: ReadStats,
        identity: DocumentIdentity,
    ) -> Self {
        let proxy = ProxyPlayer::new(DecodeLimits {
            max_bytes: request.limits.max_file_bytes,
            ..DecodeLimits::default()
        });
        ImporterBuilder {
            request,
            acad,
            stats,
            identity,
            builder: DrawingDatabaseBuilder::new(request.database),
            diagnostics: Vec::new(),
            capabilities: HashMap::new(),
            layer_ids: HashMap::new(),
            layer_transparency: HashMap::new(),
            layer_colors: HashMap::new(),
            layer_lineweights: HashMap::new(),
            style_ids: HashMap::new(),
            style_fonts: HashMap::new(),
            block_ids: HashMap::new(),
            block_base_points: HashMap::new(),
            layout_ids: HashMap::new(),
            next_entity: 1,
            next_object: 1,
            entity_total: 0,
            dropped: 0,
            proxy,
            model_layout: LayoutId(0),
            block_members: HashMap::new(),
            block_status: HashMap::new(),
            model_render: SupportStatus::Verified,
            model_drawable: false,
            model_render_types: BTreeSet::new(),
        }
    }

    fn run(mut self) -> CadResult<ImportedDrawing> {
        self.read_layers()?;
        self.read_styles()?;
        self.read_layouts()?;
        self.read_blocks()?;
        self.read_entities()?;

        // Capture everything we need before the builder is consumed.
        let units = self.read_units();
        let capabilities: Vec<EntityCapability> = {
            let mut v: Vec<_> = std::mem::take(&mut self.capabilities)
                .into_values()
                .collect();
            v.sort_by(|a, b| a.type_key.cmp(&b.type_key));
            v
        };
        let mut diagnostics = std::mem::take(&mut self.diagnostics);
        let dropped = self.dropped;
        let stream_completed = self.stats.stream_completed;
        let model_render = self.model_render;
        let model_drawable = self.model_drawable;
        let model_render_types: Vec<String> = std::mem::take(&mut self.model_render_types)
            .into_iter()
            .collect();
        let database = self.builder.finish()?;

        if dropped > 0 {
            diagnostics.push(Diagnostic {
                object: None,
                code: "import.entity_limit".into(),
                message: format!("{dropped} entities were skipped at the import limit"),
            });
        }
        if !stream_completed {
            diagnostics.push(Diagnostic {
                object: None,
                code: "import.incomplete_stream".into(),
                message: "source stream did not complete; recovery data may be partial".into(),
            });
        }
        // Completeness reflects what can actually be drawn, not merely whether
        // parsing succeeded (audit B20). A drawing whose content cannot be
        // rendered must not report `Complete`.
        let import_items: Vec<String> = diagnostics
            .iter()
            .filter(|d| d.code.starts_with("import."))
            .map(|d| d.message.clone())
            .collect();
        let completeness = aggregate_completeness(
            model_render,
            model_drawable,
            model_render_types,
            import_items,
        );
        Ok(ImportedDrawing {
            database,
            units,
            report: ImportReport {
                identity: self.identity.clone(),
                capabilities,
                completeness,
                diagnostics,
                parse_ms: None,
            },
        })
    }

    fn read_units(&self) -> UnitContext {
        match self.acad.header.insertion_units {
            1 => UnitContext {
                source: Unit::Inch,
                display: Unit::Inch,
                display_per_source: Some(1.0),
                decimal_places: 3,
            },
            2 => UnitContext {
                source: Unit::Foot,
                display: Unit::Foot,
                display_per_source: Some(1.0),
                decimal_places: 3,
            },
            4 => UnitContext {
                source: Unit::Millimeter,
                display: Unit::Millimeter,
                display_per_source: Some(1.0),
                decimal_places: 3,
            },
            5 => UnitContext {
                source: Unit::Meter,
                display: Unit::Meter,
                display_per_source: Some(0.01),
                decimal_places: 3,
            },
            6 => UnitContext {
                source: Unit::Meter,
                display: Unit::Meter,
                display_per_source: Some(1.0),
                decimal_places: 3,
            },
            _ => UnitContext::drawing_units(),
        }
    }

    fn read_layers(&mut self) -> CadResult<()> {
        for (index, layer) in self.acad.layers.iter().enumerate() {
            let id = LayerId(index as u128);
            self.layer_ids.insert(layer.name.clone(), id);
            self.layer_transparency
                .insert(id, layer_opacity(layer.transparency));
            self.layer_colors.insert(id, layer_rgb(layer.color));
            if let Some(mm) = lineweight_mm(layer.line_weight) {
                self.layer_lineweights.insert(id, mm);
            }
            let visible = !layer.flags.off && !layer.flags.frozen;
            self.builder.insert_layer(Layer {
                id,
                name: layer.name.clone(),
                visible,
            })?;
        }
        Ok(())
    }

    fn read_styles(&mut self) -> CadResult<()> {
        for (index, style) in self.acad.text_styles.iter().enumerate() {
            let id = StyleId(index as u128);
            self.style_ids.insert(style.name.clone(), id);
            let mut keys = Vec::new();
            if !style.font_file.is_empty() {
                keys.push(style.font_file.clone());
            }
            if !style.big_font_file.is_empty() {
                keys.push(style.big_font_file.clone());
            }
            if !style.true_type_font.is_empty() {
                keys.push(style.true_type_font.clone());
            }
            // Primary font for this style, used to shape text geometry. Prefer
            // the TrueType face when the drawing names one explicitly.
            let primary = if !style.true_type_font.is_empty() {
                Some(style.true_type_font.clone())
            } else if !style.font_file.is_empty() {
                Some(style.font_file.clone())
            } else {
                None
            };
            if let Some(font) = primary {
                self.style_fonts
                    .insert(style.name.to_ascii_lowercase(), font);
            }
            self.builder.insert_style(Style {
                id,
                name: style.name.clone(),
                resource_keys: keys,
            })?;
        }
        Ok(())
    }

    fn read_layouts(&mut self) -> CadResult<()> {
        let mut order = 0u128;
        for block in self.acad.block_records.iter() {
            if !is_paper_space_name(&block.name) {
                continue;
            }
            let id = LayoutId(order + 1);
            order += 1;
            self.layout_ids.insert(block.name.clone(), id);
            let mut viewports = Vec::new();
            for entity in self.acad.entities_in_block(&block.name) {
                if let EntityType::Viewport(v) = entity {
                    // The sheet viewport (`id == 1`) frames the paper itself, not
                    // a model-space window, and an off viewport draws nothing;
                    // neither is a content viewport (audit B22).
                    let Some(viewport) = paper_viewport(v) else {
                        continue;
                    };
                    if let Completeness::Partial(reasons) = &viewport.completeness {
                        let why = reasons
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "unsupported viewport state".to_string());
                        self.diagnostics.push(Diagnostic {
                            object: None,
                            code: "import.viewport_unsupported".into(),
                            message: format!("layout {} viewport {}: {why}", block.name, v.id),
                        });
                    }
                    viewports.push(viewport);
                }
            }
            self.builder.insert_layout(Layout {
                id,
                name: block.name.clone(),
                viewports,
            })?;
        }
        if self.layout_ids.is_empty() {
            let id = LayoutId(0);
            self.builder.insert_layout(Layout {
                id,
                name: "Model".into(),
                viewports: Vec::new(),
            })?;
            self.model_layout = id;
        }
        Ok(())
    }

    fn read_blocks(&mut self) -> CadResult<()> {
        let mut next = 0u128;
        for block in self.acad.block_records.iter() {
            if is_space_block_name(&block.name) {
                continue;
            }
            let id = BlockId(next);
            next += 1;
            self.block_ids.insert(block.name.clone(), id);
            self.block_base_points
                .insert(block.name.clone(), p3(block.base_point));
            self.builder.insert_block(BlockDefinition {
                id,
                entities: Vec::new(),
            })?;
        }
        Ok(())
    }

    fn read_entities(&mut self) -> CadResult<()> {
        // Block definitions first, so model-space INSERTs can resolve the
        // display status of the block they reference (audit B15/B20).
        for block in self.acad.block_records.iter() {
            if is_space_block_name(&block.name) {
                continue;
            }
            let Some(id) = self.block_ids.get(&block.name).copied() else {
                continue;
            };
            let mut entity_ids = Vec::new();
            for entity in self.acad.entities_in_block(&block.name) {
                if let Some(e) = self.push_entity(entity, SpaceId::Block(id), self.model_layout) {
                    entity_ids.push(e);
                }
            }
            self.builder.insert_block(BlockDefinition {
                id,
                entities: entity_ids,
            })?;
        }
        self.resolve_block_status();

        // Model space: the primary drawable set.
        let model_space = self.model_space_id();
        for entity in self.acad.model_space_entities() {
            self.push_entity(entity, SpaceId::Model, model_space);
        }
        // Paper space.
        for (name, layout) in self.layout_ids.clone() {
            for entity in self.acad.entities_in_block(&name) {
                self.push_entity(entity, SpaceId::Paper(layout), layout);
            }
        }
        Ok(())
    }

    /// Resolve each block's render status from its children, iterating to a
    /// fixpoint so nested INSERTs are accounted for. Cycles settle at
    /// `Unverified` instead of looping.
    fn resolve_block_status(&mut self) {
        let ids: Vec<BlockId> = self.block_members.keys().copied().collect();
        for _ in 0..=ids.len() {
            let mut changed = false;
            for id in &ids {
                let members = self.block_members.get(id).cloned().unwrap_or_default();
                let mut status = SupportStatus::Verified;
                for member in &members {
                    // A member that references blocks inherits the weakest of
                    // its dependencies; a plain member uses its own status.
                    let child = if member.insert_blocks.is_empty() {
                        member.render
                    } else {
                        member
                            .insert_blocks
                            .iter()
                            .map(|block| {
                                self.block_status
                                    .get(block)
                                    .copied()
                                    .unwrap_or(SupportStatus::Unverified)
                            })
                            .fold(SupportStatus::Verified, weaker)
                    };
                    status = weaker(status, child);
                }
                if self.block_status.get(id).copied() != Some(status) {
                    self.block_status.insert(*id, status);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn model_space_id(&self) -> LayoutId {
        self.layout_ids
            .get("*Model_Space")
            .copied()
            .unwrap_or(self.model_layout)
    }

    fn push_entity(
        &mut self,
        entity: &EntityType,
        space: SpaceId,
        _layout: LayoutId,
    ) -> Option<EntityId> {
        if matches!(entity, EntityType::Block(_) | EntityType::BlockEnd(_)) {
            return None;
        }
        if self.entity_total >= self.request.limits.max_entities {
            self.dropped += 1;
            return None;
        }
        let common = entity.common();
        let class_name = entity_class_name(entity);
        let layer = self
            .layer_ids
            .get(&common.layer)
            .copied()
            .unwrap_or_else(|| self.layer_ids.get("0").copied().unwrap_or(LayerId(0)));

        let (geometry, completeness) = self.convert(entity, common);

        // Render/pick are judged from the drawn result, not from parse success
        // (audit B20). An INSERT inherits the resolved status of its block; an
        // array INSERT is a Compound of Instances that all reference one block.
        let (mut render, mut pick) = display_support(&geometry);
        let insert_blocks = referenced_blocks(&geometry);
        for block in &insert_blocks {
            if let Some(status) = self.block_status.get(block) {
                render = weaker(render, *status);
                pick = weaker(pick, *status);
            } else {
                render = weaker(render, SupportStatus::Unverified);
                pick = weaker(pick, SupportStatus::Unverified);
            }
        }
        if let SpaceId::Block(block) = &space {
            self.block_members
                .entry(*block)
                .or_default()
                .push(BlockMember {
                    render,
                    insert_blocks,
                });
        }
        if matches!(&space, SpaceId::Model) {
            self.model_render = weaker(self.model_render, render);
            if render == SupportStatus::Verified {
                self.model_drawable = true;
            } else {
                self.model_render_types.insert(class_name.clone());
            }
        }

        let entity_id = EntityId(self.next_entity);
        let object_id = ObjectId(self.next_object);
        self.next_entity += 1;
        self.next_object += 1;
        self.entity_total += 1;

        self.note_capability(&class_name, &geometry, &completeness, render, pick);

        // Resolve the entity's effective display opacity, colour and lineweight
        // so the representation/scene layers can carry real style instead of
        // always drawing opaque grey (audit F14 / §2.1.3). ByObject wins over
        // ByLayer; ByBlock is kept symbolic for INSERT expansion to resolve.
        //
        // LINETYPE is deliberately NOT resolved this round: acadrust exposes
        // `common.linetype` / `common.linetype_scale`, but dash generation is a
        // separate round (`docs/entity-style.md` "显式未实现"). No dash pattern is
        // fabricated here.
        let layer_alpha = self.layer_transparency.get(&layer).copied().unwrap_or(1.0);
        let layer_color = self.layer_colors.get(&layer).copied();
        let layer_lineweight = self.layer_lineweights.get(&layer).copied();
        let attributes = EntityRenderAttributes {
            transparency: resolve_entity_transparency(common.transparency, layer_alpha),
            color: resolve_entity_color(common.color, layer_color),
            lineweight: resolve_entity_lineweight(common.line_weight, layer_lineweight),
            geometry_source: if proxy_geometry_allowed(entity) {
                GeometrySource::ProxyCache
            } else {
                GeometrySource::Analytic
            },
        };

        let record = DbEntity {
            object: DbObject {
                id: object_id,
                type_key: class_name.clone(),
                revision: Revision(0),
                source_handle: Some(format!("{:X}", common.handle.value())),
            },
            id: entity_id,
            layer,
            space,
            geometry,
            draw_order: self.entity_total as i64,
        };
        if let Err(e) = self.builder.insert_entity(record) {
            self.diagnostics.push(Diagnostic {
                object: Some(object_id),
                code: "import.insert_failed".into(),
                message: e.to_string(),
            });
            return None;
        }
        if let Err(e) = self
            .builder
            .set_entity_render_attributes(entity_id, attributes)
        {
            // The entity was just inserted, so this cannot fail today; report
            // rather than pretend the transparency was recorded.
            self.diagnostics.push(Diagnostic {
                object: Some(object_id),
                code: "import.render_attributes_failed".into(),
                message: e.to_string(),
            });
        }
        Some(entity_id)
    }

    fn convert(
        &mut self,
        entity: &EntityType,
        common: &EntityCommon,
    ) -> (SemanticGeometry, Completeness) {
        match entity {
            EntityType::Line(l) => (
                SemanticGeometry::Line {
                    start: p3(l.start),
                    end: p3(l.end),
                },
                Completeness::Complete,
            ),
            EntityType::Circle(c) => (
                SemanticGeometry::Circle {
                    // A CIRCLE stores its centre in OCS; the extrusion normal
                    // defines the plane. `center_wcs` runs the AutoCAD
                    // arbitrary-axis frame so a tilted circle is placed in its
                    // own plane, not flattened onto world XY (audit B23/B31).
                    center: p3(c.center_wcs()),
                    normal: p3(c.normal),
                    radius: c.radius,
                },
                Completeness::Complete,
            ),
            EntityType::Arc(a) => (
                SemanticGeometry::Arc {
                    center: p3(a.center_wcs()),
                    normal: p3(a.normal),
                    radius: a.radius,
                    start: a.start_angle,
                    // Angles are measured in the OCS frame `arbitrary_axis`
                    // rebuilds from the same normal, so the arc keeps its
                    // sweep and side.
                    sweep: normalize_sweep(a.end_angle - a.start_angle),
                },
                Completeness::Complete,
            ),
            EntityType::Ellipse(e) => ellipse_semantics(e),
            EntityType::Point(p) => (
                SemanticGeometry::Point(p3(p.location)),
                Completeness::Complete,
            ),
            EntityType::LwPolyline(pl) => {
                let normal = p3(pl.normal);
                let points = polyline_ocs_points(
                    normal,
                    pl.elevation,
                    pl.vertices.iter().map(|v| (v.location.x, v.location.y)),
                );
                let bulges: Vec<f64> = pl.vertices.iter().map(|v| v.bulge).collect();
                let completeness = polyline_completeness(normal, pl.vertices.len(), &bulges);
                (
                    SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed: pl.is_closed,
                    },
                    completeness,
                )
            }
            EntityType::Polyline2D(pl) => {
                let normal = p3(pl.normal);
                let points = polyline_ocs_points(
                    normal,
                    pl.elevation,
                    pl.vertices.iter().map(|v| (v.location.x, v.location.y)),
                );
                let bulges: Vec<f64> = pl.vertices.iter().map(|v| v.bulge).collect();
                let closed = pl.flags.bits() & 1 != 0;
                let completeness = polyline_completeness(normal, pl.vertices.len(), &bulges);
                (
                    SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed,
                    },
                    completeness,
                )
            }
            EntityType::Polyline3D(pl) => {
                let points: Vec<Point3> = pl.vertices.iter().map(|v| p3(v.position)).collect();
                (
                    SemanticGeometry::Polyline {
                        points,
                        bulges: Vec::new(),
                        closed: pl.flags.closed,
                    },
                    Completeness::Complete,
                )
            }
            EntityType::Text(t) => {
                let (h_align, v_align) = (
                    map_h_align(t.horizontal_alignment),
                    map_v_align(t.vertical_alignment),
                );
                // DXF places aligned text at the alignment point when one is set.
                let position = if t.alignment_point.is_some()
                    && (h_align != TextAlignH::Left || v_align != TextAlignV::Baseline)
                {
                    t.alignment_point.unwrap_or(t.insertion_point)
                } else {
                    t.insertion_point
                };
                (
                    SemanticGeometry::Text {
                        text: t.value.clone(),
                        position: p3(position),
                        style: self.style_id(&t.style),
                        height: t.height,
                        rotation: t.rotation,
                        font: self.style_font(&t.style),
                        h_align,
                        v_align,
                    },
                    Completeness::Complete,
                )
            }
            EntityType::MText(t) => {
                let (h_align, v_align) = attach_align(t.attachment_point);
                (
                    SemanticGeometry::Text {
                        text: t.value.clone(),
                        position: p3(t.insertion_point),
                        style: self.style_id(&t.style),
                        height: t.height,
                        rotation: t.rotation,
                        font: self.style_font(&t.style),
                        h_align,
                        v_align,
                    },
                    Completeness::Complete,
                )
            }
            EntityType::Spline(s) => spline_semantics(s),
            EntityType::Solid(s) => solid_mesh_semantics(s),
            EntityType::Face3D(f) => {
                // A 3DFACE stores its corners in WCS and in boundary order
                // (first, second, third, fourth), unlike SOLID/TRACE
                // (docs/layouts.md §4, audit B31).
                let m = quad_mesh([
                    p3(f.first_corner),
                    p3(f.second_corner),
                    p3(f.third_corner),
                    p3(f.fourth_corner),
                ]);
                (SemanticGeometry::Mesh(m), Completeness::Complete)
            }
            EntityType::Insert(i) => self.insert_semantics(i),
            EntityType::Unknown(u) => {
                self.proxy_geometry(&u.dxf_name, common, u.raw_dwg_data.as_deref())
            }
            EntityType::Extended(x) if x.class_name() == "ACAD_PROXY_ENTITY" => {
                self.proxy_geometry(x.class_name(), common, None)
            }
            EntityType::Hatch(h) => Self::hatch_geometry(h),
            EntityType::Dimension(d) => {
                // A dimension's visible geometry lives in an anonymous block
                // (`*D...`); expand it like an insert instead of dropping it.
                let base = d.base();
                if base.block_name.is_empty() {
                    (
                        SemanticGeometry::Opaque {
                            type_key: "AcDbDimension".into(),
                            version: 1,
                            payload: Vec::new(),
                        },
                        Completeness::Partial(vec!["dimension has no geometry block".into()]),
                    )
                } else {
                    (
                        SemanticGeometry::Insert {
                            block: self.block_id(&base.block_name),
                            transform: dimension_transform(base),
                        },
                        Completeness::Complete,
                    )
                }
            }
            EntityType::Solid3D(s) => acis_semantics(entity, &s.acis_data),
            EntityType::Region(r) => acis_semantics(entity, &r.acis_data),
            EntityType::Body(b) => acis_semantics(entity, &b.acis_data),
            EntityType::Surface(s) => acis_semantics(entity, &s.acis_data),
            other => (
                SemanticGeometry::Opaque {
                    type_key: entity_class_name(other),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Unverified,
            ),
        }
    }

    fn proxy_geometry(
        &mut self,
        class_name: &str,
        common: &EntityCommon,
        raw: Option<&[u8]>,
    ) -> (SemanticGeometry, Completeness) {
        let source = ProxySource {
            handle: format!("{:X}", common.handle.value()),
            class_name: class_name.to_string(),
            application: None,
            dwg_version: format!("{:?}", self.stats.source_version),
        };
        match common.graphic_data.as_deref() {
            Some(data) => match self.proxy.replay(&source, data) {
                Ok(out) if !out.geometry.is_empty() => {
                    self.diagnostics.extend(out.diagnostics);
                    let completeness = out.completeness.clone();
                    // Keep *every* decoded fragment (audit B21): `.next()` used
                    // to silently drop all but the first proxy record. A single
                    // item stays itself; several become a `Compound`, which the
                    // representation layer draws one primitive at a time.
                    (proxy_geometry_compound(out.geometry), completeness)
                }
                Ok(out) => {
                    self.diagnostics.extend(out.diagnostics);
                    (
                        SemanticGeometry::Opaque {
                            type_key: class_name.to_string(),
                            version: 1,
                            payload: Vec::new(),
                        },
                        out.completeness,
                    )
                }
                Err(e) => {
                    self.diagnostics.push(Diagnostic {
                        object: None,
                        code: "import.proxy_error".into(),
                        message: format!("{class_name}: {e}"),
                    });
                    (
                        SemanticGeometry::Opaque {
                            type_key: class_name.to_string(),
                            version: 1,
                            payload: Vec::new(),
                        },
                        Completeness::Partial(vec!["proxy decode failed".into()]),
                    )
                }
            },
            None => {
                if let Some(raw) = raw {
                    let _ = self.proxy.inspect_raw_dwg(raw);
                }
                self.diagnostics.push(Diagnostic {
                    object: None,
                    code: "import.proxy_no_cache".into(),
                    message: format!(
                        "{class_name} has no proxy graphics cache; shown as a placeholder"
                    ),
                });
                (
                    SemanticGeometry::Opaque {
                        type_key: class_name.to_string(),
                        version: 1,
                        payload: Vec::new(),
                    },
                    Completeness::Missing(vec!["no proxy cache in the source drawing".into()]),
                )
            }
        }
    }

    fn style_id(&self, name: &str) -> StyleId {
        self.style_ids.get(name).copied().unwrap_or(StyleId(0))
    }

    /// Primary font file declared by a named text style, if any.
    fn style_font(&self, name: &str) -> Option<String> {
        self.style_fonts.get(&name.to_ascii_lowercase()).cloned()
    }

    fn block_id(&self, name: &str) -> BlockId {
        self.block_ids
            .get(name)
            .copied()
            .unwrap_or(BlockId(u128::MAX))
    }

    /// The block's insertion base point, defaulting to the origin.
    fn block_base(&self, name: &str) -> Point3 {
        self.block_base_points
            .get(name)
            .copied()
            .unwrap_or_default()
    }

    /// Convert an INSERT (including an array / MINSERT) into expandable
    /// geometry.
    ///
    /// One cell yields a single [`SemanticGeometry::Insert`]; `rows × columns`
    /// cells yield a [`SemanticGeometry::Compound`] of them, one per cell (audit
    /// B31). The block definition itself is never emitted directly, so it cannot
    /// be double-drawn. Unknown block names are reported `Missing` rather than
    /// expanded as an empty success.
    fn insert_semantics(&self, i: &acadrust::entities::Insert) -> (SemanticGeometry, Completeness) {
        let name = i.block_name.clone();
        let base = self.block_base(&name);
        let mut completeness = Completeness::Complete;
        if !self.block_ids.contains_key(&name) {
            completeness =
                Completeness::Missing(vec![format!("insert references unknown block '{name}'")]);
        }
        let columns = i.column_count.max(1) as usize;
        let rows = i.row_count.max(1) as usize;
        // A non-finite spacing cannot produce distinct cells; drawing them all on
        // top of each other would hide the state, so report it.
        let spacing_bad = |s: f64| !s.is_finite();
        if columns * rows > 1 && (spacing_bad(i.column_spacing) || spacing_bad(i.row_spacing)) {
            completeness = Completeness::Partial(vec![
                "insert array has a non-finite row/column spacing".into(),
            ]);
        }
        let mut instances = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            for column in 0..columns {
                instances.push(SemanticGeometry::Insert {
                    block: self.block_id(&name),
                    transform: insert_array_transform(
                        i,
                        base,
                        column as f64 * i.column_spacing,
                        row as f64 * i.row_spacing,
                    ),
                });
            }
        }
        let geometry = match instances.len() {
            1 => instances.pop().expect("length checked"),
            _ => SemanticGeometry::Compound(instances),
        };
        (geometry, completeness)
    }

    /// Convert a HATCH into a compound of boundary loops plus a solid fill or
    /// pattern lines. Boundaries are always emitted; fills that cannot be
    /// generated are reported as Partial rather than faked.
    fn hatch_geometry(h: &Hatch) -> (SemanticGeometry, Completeness) {
        let normal = p3(h.normal);
        // A degenerate normal cannot define a hatch plane; the boundary is still
        // emitted, but any fill is honestly reported as boundary-only.
        let plane_ok = cad_geometry::is_finite(normal) && cad_geometry::length(normal) > 1e-12;
        let (ux, uy, un) = arbitrary_axis(normal);
        let origin = cad_geometry::scale(un, h.elevation);
        let to_world = |p: [f64; 2]| {
            cad_geometry::add(
                origin,
                cad_geometry::add(cad_geometry::scale(ux, p[0]), cad_geometry::scale(uy, p[1])),
            )
        };
        let params = TessellationParams {
            tolerance: hatch_tolerance(h),
            ..TessellationParams::default()
        };
        let mut loops: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut children: Vec<SemanticGeometry> = Vec::new();
        for path in &h.paths {
            let mut points: Vec<[f64; 2]> = Vec::new();
            for edge in &path.edges {
                append_boundary_edge(edge, &mut points, &params);
            }
            dedup_loop(&mut points);
            if points.len() < 3 {
                continue;
            }
            children.push(SemanticGeometry::Polyline {
                points: points.iter().map(|p| to_world(*p)).collect(),
                bulges: Vec::new(),
                closed: true,
            });
            loops.push(points);
        }
        if loops.is_empty() {
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbHatch".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(vec!["hatch has no usable boundary".into()]),
            );
        }
        let mut completeness = Completeness::Complete;
        let solid = h.is_solid || h.pattern.name.eq_ignore_ascii_case("SOLID");
        if solid {
            if !plane_ok {
                completeness = Completeness::Partial(vec![
                    "hatch plane normal is degenerate; boundary only".into(),
                ]);
            } else {
                // Simplify every ring with a tolerance scaled to the hatch's own
                // extent. This keeps the even-odd fill bounded (spec §3.2).
                let (min, max) = loops.iter().flatten().fold(
                    ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
                    |(mut lo, mut hi), p| {
                        lo[0] = lo[0].min(p[0]);
                        lo[1] = lo[1].min(p[1]);
                        hi[0] = hi[0].max(p[0]);
                        hi[1] = hi[1].max(p[1]);
                        (lo, hi)
                    },
                );
                let diagonal = ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2)).sqrt();
                let tolerance = (diagonal * 1e-3).max(1e-9);
                let simplified: Vec<Vec<[f64; 2]>> = loops
                    .iter()
                    .filter_map(|loop2| {
                        let mut closed = loop2.clone();
                        closed.push(closed[0]);
                        let mut s = cad_geometry::simplify(&closed, tolerance);
                        if s.len() > 1 && s.last() == s.first() {
                            s.pop();
                        }
                        (s.len() >= 3).then_some(s)
                    })
                    .collect();
                // Multi-ring holes/islands go through the even-odd fill; a
                // failure is reported as Partial (boundary only), never faked.
                match cad_geometry::fill_rings(&simplified) {
                    Ok(fill) => {
                        let vertices: Vec<Point3> =
                            fill.vertices.iter().map(|p| to_world(*p)).collect();
                        let normals = vec![un; vertices.len()];
                        children.push(SemanticGeometry::Mesh(Mesh {
                            vertices,
                            triangles: fill.triangles,
                            normals,
                            face_sources: Vec::new(),
                        }));
                    }
                    Err(error) => {
                        completeness = Completeness::Partial(vec![format!(
                            "solid hatch could not be filled: {}",
                            error.reason()
                        )]);
                    }
                }
            }
        } else if h.gradient_color.enabled {
            completeness =
                Completeness::Partial(vec!["gradient hatch renders its boundary only".into()]);
        } else {
            let families = pattern_families(h);
            if families.is_empty() {
                completeness =
                    Completeness::Partial(vec!["hatch pattern has no line families".into()]);
            } else {
                for line in cad_geometry::pattern_polylines(&loops, &families) {
                    if line.len() >= 2 {
                        children.push(SemanticGeometry::Polyline {
                            points: line.iter().map(|p| to_world(*p)).collect(),
                            bulges: Vec::new(),
                            closed: false,
                        });
                    }
                }
            }
        }
        (SemanticGeometry::Compound(children), completeness)
    }

    fn note_capability(
        &mut self,
        class_name: &str,
        geometry: &SemanticGeometry,
        completeness: &Completeness,
        render: SupportStatus,
        pick: SupportStatus,
    ) {
        let semantic = match completeness {
            Completeness::Complete => SupportStatus::Verified,
            Completeness::Partial(_) => SupportStatus::Partial,
            Completeness::Missing(_) => SupportStatus::Unsupported,
            Completeness::Unverified => SupportStatus::Unverified,
        };
        let measure = match geometry {
            SemanticGeometry::Mesh(_)
            | SemanticGeometry::Opaque { .. }
            | SemanticGeometry::Text { .. } => SupportStatus::Unsupported,
            _ => semantic,
        };
        let entry = self
            .capabilities
            .entry(class_name.to_string())
            .or_insert(EntityCapability {
                type_key: class_name.to_string(),
                read: SupportStatus::Verified,
                semantic,
                render,
                pick,
                measure,
            });
        // Keep the weakest status observed for the class.
        entry.semantic = weaker(entry.semantic, semantic);
        entry.render = weaker(entry.render, render);
        entry.pick = weaker(entry.pick, pick);
        entry.measure = weaker(entry.measure, measure);
    }
}

/// Retain a solid/surface entity's ACIS payload as opaque bytes.
///
/// The neutral lift for tessellation is [`solid_exchange_from_entity`]; this
/// opaque form keeps the raw SAT/SAB for provenance and for a later kernel
/// provider, and deliberately claims no display support of its own.
fn acis_semantics(
    entity: &EntityType,
    acis: &acadrust::entities::AcisData,
) -> (SemanticGeometry, Completeness) {
    let (version, payload) = acis_raw_payload(acis);
    (
        SemanticGeometry::Opaque {
            type_key: entity_class_name(entity),
            version,
            payload,
        },
        Completeness::Unverified,
    )
}

/// Whether the display pipeline can actually draw this geometry (audit B20).
///
/// Text has no scene batching yet and opaque/ACIS geometry has no
/// representation, so neither is `Verified`; an INSERT depends on its block
/// contents and is resolved by the caller.
fn display_support(geometry: &SemanticGeometry) -> (SupportStatus, SupportStatus) {
    match geometry {
        SemanticGeometry::Opaque { .. } => (SupportStatus::Unsupported, SupportStatus::Unsupported),
        // Outline fonts (TTF/OTF/WOFF) and SHX shape fonts can be shaped once
        // the host supplies them; an unknown/absent font has no decoder.
        SemanticGeometry::Text { font, .. } => match font.as_deref().map(font_extension) {
            Some(ext) if matches!(ext.as_str(), "ttf" | "otf" | "woff" | "shx") => {
                (SupportStatus::Unverified, SupportStatus::Unverified)
            }
            _ => (SupportStatus::Unsupported, SupportStatus::Unsupported),
        },
        SemanticGeometry::Insert { .. } => (SupportStatus::Unverified, SupportStatus::Unverified),
        SemanticGeometry::Compound(children) => {
            let mut render = SupportStatus::Verified;
            let mut pick = SupportStatus::Verified;
            for child in children {
                let (r, p) = display_support(child);
                render = weaker(render, r);
                pick = weaker(pick, p);
            }
            (render, pick)
        }
        _ => (SupportStatus::Verified, SupportStatus::Verified),
    }
}

/// Convert acadrust's DWG transparency byte (0 opaque .. 255 transparent) to an
/// opacity in `[0, 1]` (1 opaque .. 0 transparent).
fn dwg_transparency_to_opacity(alpha: u8) -> f32 {
    (1.0 - alpha as f32 / 255.0).clamp(0.0, 1.0)
}

/// Effective opacity of a layer entry.
///
/// A layer normally carries an explicit transparency; the `ByLayer`/`ByBlock`
/// variants are degenerate on a layer and fall back to opaque rather than
/// fabricating a value.
fn layer_opacity(transparency: acadrust::Transparency) -> f32 {
    match transparency {
        acadrust::Transparency::Explicit(alpha) => dwg_transparency_to_opacity(alpha),
        acadrust::Transparency::ByLayer | acadrust::Transparency::ByBlock => 1.0,
    }
}

/// Resolve an entity's own DWG transparency into the effective display value.
///
/// `ByObject` (`Explicit`) overrides the layer; `ByLayer` uses the pre-resolved
/// `layer_alpha`; `ByBlock` is kept symbolic so INSERT expansion substitutes the
/// containing reference's opacity.
fn resolve_entity_transparency(
    transparency: acadrust::Transparency,
    layer_alpha: f32,
) -> EntityTransparency {
    match transparency {
        acadrust::Transparency::Explicit(alpha) => {
            EntityTransparency::Explicit(dwg_transparency_to_opacity(alpha))
        }
        acadrust::Transparency::ByLayer => {
            EntityTransparency::Explicit(layer_alpha.clamp(0.0, 1.0))
        }
        acadrust::Transparency::ByBlock => EntityTransparency::ByBlock,
    }
}

/// sRGB value of a concrete acadrust colour, or `None` when symbolic.
fn concrete_rgb(color: acadrust::Color) -> Option<[u8; 3]> {
    match color {
        acadrust::Color::Rgb { r, g, b } => Some([r, g, b]),
        // ACI indices resolve through acadrust's own canonical table, so an
        // `Index(1)` becomes true red rather than a guessed constant.
        acadrust::Color::Index(_) => color.rgb().map(|(r, g, b)| [r, g, b]),
        acadrust::Color::ByLayer | acadrust::Color::ByBlock | acadrust::Color::None => None,
    }
}

/// Resolve a layer's colour to sRGB bytes.
///
/// A layer carries a concrete colour in practice; the symbolic variants are
/// degenerate on a layer and fall back to white (AutoCAD's nominal default
/// entity colour) rather than fabricating a value.
fn layer_rgb(color: acadrust::Color) -> [u8; 3] {
    concrete_rgb(color).unwrap_or([255, 255, 255])
}

/// Resolve an entity's own colour into the effective display value.
///
/// An explicit entity colour (`ByObject`: true colour or an ACI index) wins over
/// the layer. `ByLayer` uses the layer's pre-resolved colour. `ByBlock` is kept
/// symbolic so INSERT expansion substitutes the containing reference's colour;
/// `None` (no colour) is treated as unresolved `ByLayer` rather than drawn black.
fn resolve_entity_color(color: acadrust::Color, layer_color: Option<[u8; 3]>) -> EntityColor {
    match concrete_rgb(color) {
        Some(rgb) => EntityColor::Explicit(rgb),
        None => match color {
            acadrust::Color::ByBlock => EntityColor::ByBlock,
            _ => match layer_color {
                Some(rgb) => EntityColor::Explicit(rgb),
                None => EntityColor::ByLayer,
            },
        },
    }
}

/// A lineweight in millimetres, or `None` for the symbolic/`Default` variants.
///
/// acadrust stores concrete weights as 1/100 mm; `LineWeight::millimeters`
/// performs that conversion, so no scale factor is invented here.
fn lineweight_mm(weight: acadrust::LineWeight) -> Option<f32> {
    match weight {
        acadrust::LineWeight::Value(_) => weight.millimeters().map(|mm| mm as f32),
        acadrust::LineWeight::ByLayer
        | acadrust::LineWeight::ByBlock
        | acadrust::LineWeight::Default => None,
    }
}

/// Resolve an entity's own lineweight into the effective display value.
///
/// A concrete entity weight wins over the layer; `ByLayer` uses the layer's
/// pre-resolved weight; `ByBlock` stays symbolic for INSERT expansion;
/// `Default` keeps acadrust's explicit "default" meaning.
fn resolve_entity_lineweight(
    weight: acadrust::LineWeight,
    layer_weight: Option<f32>,
) -> EntityLineWeight {
    match weight {
        acadrust::LineWeight::Value(_) => {
            EntityLineWeight::Explicit(lineweight_mm(weight).unwrap_or(0.0))
        }
        acadrust::LineWeight::ByBlock => EntityLineWeight::ByBlock,
        acadrust::LineWeight::Default => EntityLineWeight::Default,
        acadrust::LineWeight::ByLayer => match layer_weight {
            Some(mm) => EntityLineWeight::Explicit(mm),
            None => EntityLineWeight::ByLayer,
        },
    }
}

/// Whether a proxy cache may contribute geometry for this entity.
///
/// Only entity types with no semantic representation of their own are expanded
/// from `graphic_data`. A known entity (LINE, HATCH, ...) is drawn from its
/// semantic geometry, so overlaying its proxy cache would draw it twice
/// (audit B21; `docs/proxy-support.md` §4).
fn proxy_geometry_allowed(entity: &EntityType) -> bool {
    matches!(entity, EntityType::Unknown(_))
        || matches!(entity, EntityType::Extended(x) if x.class_name() == "ACAD_PROXY_ENTITY")
}

/// Fold every geometry item decoded from one proxy cache into a single
/// semantic geometry without dropping any.
///
/// One item stays itself; two or more become a [`SemanticGeometry::Compound`]
/// so the representation layer emits one drawable per item. This replaces the
/// previous `.into_iter().next()`, which silently discarded all but the first
/// fragment (audit B21).
fn proxy_geometry_compound(mut geometry: Vec<SemanticGeometry>) -> SemanticGeometry {
    match geometry.len() {
        1 => geometry.pop().expect("length checked"),
        _ => SemanticGeometry::Compound(geometry),
    }
}

/// Lower-cased extension of a font reference, without the dot.
fn font_extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

fn map_h_align(align: TextHorizontalAlignment) -> TextAlignH {
    match align {
        TextHorizontalAlignment::Center | TextHorizontalAlignment::Middle => TextAlignH::Center,
        TextHorizontalAlignment::Right => TextAlignH::Right,
        // Aligned/Fit still start at the left point; we do not stretch.
        TextHorizontalAlignment::Left
        | TextHorizontalAlignment::Aligned
        | TextHorizontalAlignment::Fit => TextAlignH::Left,
    }
}

fn map_v_align(align: TextVerticalAlignment) -> TextAlignV {
    match align {
        TextVerticalAlignment::Baseline => TextAlignV::Baseline,
        TextVerticalAlignment::Bottom => TextAlignV::Bottom,
        TextVerticalAlignment::Middle => TextAlignV::Middle,
        TextVerticalAlignment::Top => TextAlignV::Top,
    }
}

fn attach_align(attachment: AttachmentPoint) -> (TextAlignH, TextAlignV) {
    use AttachmentPoint::*;
    let h = match attachment {
        TopLeft | MiddleLeft | BottomLeft => TextAlignH::Left,
        TopCenter | MiddleCenter | BottomCenter => TextAlignH::Center,
        TopRight | MiddleRight | BottomRight => TextAlignH::Right,
    };
    let v = match attachment {
        TopLeft | TopCenter | TopRight => TextAlignV::Top,
        MiddleLeft | MiddleCenter | MiddleRight => TextAlignV::Middle,
        BottomLeft | BottomCenter | BottomRight => TextAlignV::Bottom,
    };
    (h, v)
}

/// Combine import-stage problems with the render support of model content.
///
/// `Complete` only when everything in model space can be drawn and the stream
/// was whole; otherwise `Partial`, or `Missing` when nothing at all is
/// drawable and there was no separate import fault to report.
fn aggregate_completeness(
    model_render: SupportStatus,
    model_drawable: bool,
    mut model_render_types: Vec<String>,
    import_items: Vec<String>,
) -> Completeness {
    let mut items = import_items;
    let has_import_item = !items.is_empty();
    if model_render != SupportStatus::Verified {
        model_render_types.sort();
        items.push(format!(
            "no display representation for: {}",
            model_render_types.join(", ")
        ));
    }
    if items.is_empty() {
        Completeness::Complete
    } else if model_drawable || has_import_item {
        Completeness::Partial(items)
    } else {
        Completeness::Missing(items)
    }
}

// ---- small helpers ----

fn p3(v: acadrust::Vector3) -> Point3 {
    Point3 {
        x: v.x,
        y: v.y,
        z: v.z,
    }
}

/// The world-Z direction, for the common `(0, 0, 1)` extrusion.
#[cfg(test)]
fn world_z() -> Point3 {
    Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    }
}

/// Convert a paper-space VIEWPORT entity into a database [`PaperViewport`].
///
/// Returns `None` for the sheet viewport (`id == 1`), which frames the paper
/// sheet itself rather than a model-space window, and for a viewport switched
/// off: neither has drawable content.
///
/// The clip is written as four axis-aligned paper corners
/// (`center ± (width/2, height/2)`), and `model_to_paper` is the real
/// paper→model map
/// `model = (paper - paper_center) * model_per_paper + view_target`,
/// so the representation layer recovers the view centre from the stored
/// transform (audit B22).
///
/// Exact only for a top/plan view: a tilted view direction, a view twist, a
/// perspective projection or a non-rectangular clip is reported `Partial` with
/// the specific reason. The representation layer then refuses it with a stable
/// code instead of drawing model space wrong.
fn paper_viewport(v: &acadrust::entities::Viewport) -> Option<PaperViewport> {
    if v.id == 1 || !v.status.is_on {
        return None;
    }
    let mut reasons: Vec<String> = Vec::new();
    // A non-rectangular (object) clip cannot be represented by four corners.
    if !v.clip_boundary_handle.null_or_value_zero() {
        reasons.push("non-rectangular viewport clip is not applied".into());
    }
    if !is_world_z(p3(v.view_direction)) {
        reasons.push("viewport view direction is not perpendicular to the paper plane".into());
    }
    if v.twist_angle.abs() > 1e-12 {
        reasons.push("viewport has a view twist".into());
    }
    if v.status.perspective {
        reasons.push("perspective viewport is not supported".into());
    }
    let view_height = v.view_height;
    let model_per_paper = if view_height.is_finite() && view_height.abs() > 1e-12 {
        view_height / v.height
    } else {
        reasons.push("viewport view_height is missing, zero or non-finite".into());
        0.0
    };
    let paper_center = p3(v.center);
    let view_target = p3(v.view_target);
    let finite = [paper_center, view_target]
        .iter()
        .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
        && v.width.is_finite()
        && v.height.is_finite();
    if !finite {
        reasons.push("viewport geometry is non-finite".into());
    }
    let half_w = v.width / 2.0;
    let half_h = v.height / 2.0;
    let clip = vec![
        Point3 {
            x: paper_center.x - half_w,
            y: paper_center.y - half_h,
            z: 0.0,
        },
        Point3 {
            x: paper_center.x + half_w,
            y: paper_center.y - half_h,
            z: 0.0,
        },
        Point3 {
            x: paper_center.x + half_w,
            y: paper_center.y + half_h,
            z: 0.0,
        },
        Point3 {
            x: paper_center.x - half_w,
            y: paper_center.y + half_h,
            z: 0.0,
        },
    ];
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = model_per_paper;
    matrix[1][1] = model_per_paper;
    matrix[0][3] = view_target.x - model_per_paper * paper_center.x;
    matrix[1][3] = view_target.y - model_per_paper * paper_center.y;
    matrix[2][3] = view_target.z;
    let completeness = if reasons.is_empty() {
        Completeness::Complete
    } else {
        Completeness::Partial(reasons)
    };
    Some(PaperViewport {
        clip,
        model_to_paper: Transform3 { matrix },
        completeness,
    })
}

/// True when an extrusion/direction is parallel to the world Z axis.
///
/// A zero vector is treated as world Z: acadrust defaults an absent normal to
/// `UNIT_Z`, and a degenerate normal has no other meaningful plane.
fn is_world_z(normal: Point3) -> bool {
    let n = cad_geometry::normalize(normal);
    n.x.abs() <= 1e-9 && n.y.abs() <= 1e-9
}

/// Map a 2D OCS (object coordinate system) point to WCS.
///
/// A 2D polyline's vertices live in the plane defined by its extrusion
/// (`normal`); `elevation` offsets the plane along that normal. The AutoCAD
/// arbitrary-axis algorithm supplies the in-plane axes. The previous importer
/// ignored the extrusion and used `(x, y, elevation)` for every polyline,
/// treating a tilted OCS entity as if it were flat on the WCS XY plane.
fn ocs_to_wcs(normal: Point3, elevation: f64, x: f64, y: f64) -> Point3 {
    let (ax, ay, az) = arbitrary_axis(normal);
    cad_geometry::add(
        cad_geometry::scale(az, elevation),
        cad_geometry::add(cad_geometry::scale(ax, x), cad_geometry::scale(ay, y)),
    )
}

/// Normalise a 2D polyline's OCS vertices to WCS.
fn polyline_ocs_points(
    normal: Point3,
    elevation: f64,
    vertices: impl IntoIterator<Item = (f64, f64)>,
) -> Vec<Point3> {
    if is_world_z(normal) {
        // Fast/identity path: the common drawing keeps `z = elevation` exactly.
        return vertices
            .into_iter()
            .map(|(x, y)| Point3 { x, y, z: elevation })
            .collect();
    }
    vertices
        .into_iter()
        .map(|(x, y)| ocs_to_wcs(normal, elevation, x, y))
        .collect()
}

/// Completeness of a 2D polyline whose vertices were mapped to WCS.
///
/// A tilted extrusion is exact for straight segments (the vertices carry the
/// plane). A bulge arc is exact too once the polyline has at least three
/// points to fix its plane; a two-point tilted bulge has no unique plane and is
/// reported Partial rather than silently drawn flat.
fn polyline_completeness(normal: Point3, point_count: usize, bulges: &[f64]) -> Completeness {
    let has_bulge = bulges.iter().any(|b| b.abs() > 1e-12);
    if !is_world_z(normal) && has_bulge && point_count < 3 {
        Completeness::Partial(vec![
            "tilted extrusion with a two-vertex bulge: the arc plane is ambiguous".into(),
        ])
    } else {
        Completeness::Complete
    }
}

/// Convert an ELLIPSE, preserving its extrusion normal.
///
/// An ELLIPSE stores its centre and major axis in world coordinates, but the
/// minor axis direction is defined by the extrusion normal
/// (`minor = cross(normal, major)`). Carrying the normal lets an ellipse on an
/// arbitrary OCS plane stay in that plane instead of being folded onto world XY
/// (audit B23/B31).
fn ellipse_semantics(e: &acadrust::entities::Ellipse) -> (SemanticGeometry, Completeness) {
    let normal = p3(e.normal);
    let normal = if is_world_z(normal) || cad_geometry::length(normal) < 1e-24 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        cad_geometry::normalize(normal)
    };
    let geometry = SemanticGeometry::Ellipse {
        center: p3(e.center),
        normal,
        major_axis: p3(e.major_axis),
        ratio: e.minor_axis_ratio,
        start: e.start_parameter,
        sweep: normalize_sweep(e.end_parameter - e.start_parameter),
    };
    let complete = is_finite_point(e.center)
        && is_finite_point(e.major_axis)
        && cad_geometry::length(p3(e.major_axis)) >= 1e-12
        && e.minor_axis_ratio.is_finite();
    let completeness = if complete {
        Completeness::Complete
    } else {
        Completeness::Partial(vec!["ellipse has a zero or non-finite axis/ratio".into()])
    };
    (geometry, completeness)
}

fn is_finite_point(v: acadrust::types::Vector3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// Convert a SPLINE, preserving the source's own degree, knots and weights.
///
/// A rational spline is no longer downgraded: the geometry engine evaluates the
/// source knot vector and weights exactly (audit B23). The status is `Partial`
/// only when the source record cannot be represented faithfully — missing
/// control points (a fit-point-only spline), or a knot/weight vector that does
/// not match the control polygon length.
fn spline_semantics(s: &acadrust::entities::Spline) -> (SemanticGeometry, Completeness) {
    let degree = s.degree.max(1) as u32;
    let control_points: Vec<Point3> = s.control_points.iter().map(|p| p3(*p)).collect();
    let geometry = SemanticGeometry::Spline {
        degree,
        knots: s.knots.clone(),
        control_points: control_points.clone(),
        weights: s.weights.clone(),
    };
    let n = control_points.len();
    let mut reasons: Vec<String> = Vec::new();
    if n == 0 && !s.fit_points.is_empty() {
        reasons.push("fit-point spline is not interpolated to a NURBS control polygon".into());
    }
    if n > 0 {
        let expected = n + degree as usize + 1;
        if s.knots.len() != expected {
            reasons.push(format!(
                "knot vector has {} entries, expected {expected}",
                s.knots.len()
            ));
        }
        if !s.weights.is_empty() && s.weights.len() != n {
            reasons.push(format!(
                "weight vector has {} entries, expected {n}",
                s.weights.len()
            ));
        }
    }
    let completeness = if reasons.is_empty() {
        Completeness::Complete
    } else {
        Completeness::Partial(reasons)
    };
    (geometry, completeness)
}

fn normalize_sweep(sweep: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut s = sweep % tau;
    if s <= 0.0 {
        s += tau;
    }
    s
}

fn quad_mesh(corners: [Point3; 4]) -> Mesh {
    let mut vertices = corners.to_vec();
    // A SOLID/TRACE/FACE with repeated corners is a triangle.
    let degenerate = corners[2] == corners[3];
    let triangles = if degenerate {
        vec![[0, 1, 2]]
    } else {
        vec![[0, 1, 2], [0, 2, 3]]
    };
    let normals = cad_geometry::compute_vertex_normals(&Mesh {
        vertices: vertices.clone(),
        triangles: triangles.clone(),
        normals: Vec::new(),
        face_sources: Vec::new(),
    });
    Mesh {
        vertices: std::mem::take(&mut vertices),
        triangles,
        normals,
        face_sources: Vec::new(),
    }
}

/// A LWPOLYLINE/POLYLINE's OCS plane normal, defaulting a degenerate or absent
/// normal to world Z.
fn polyline_normal(normal: Point3) -> Point3 {
    if cad_geometry::length(normal) < 1e-24 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        cad_geometry::normalize(normal)
    }
}

/// Build the block-local shift that moves the block's insertion base point to
/// the local origin, so the INSERT's insertion point lands on it.
fn block_placement(base_point: Point3) -> Transform3 {
    Transform3::translation(Point3 {
        x: -base_point.x,
        y: -base_point.y,
        z: -base_point.z,
    })
}

/// OCS (extrusion-direction) to WCS transform, the AutoCAD arbitrary-axis one.
fn ocs_to_wcs_transform(normal: Point3) -> Transform3 {
    let (ax, ay, az) = arbitrary_axis(polyline_normal(normal));
    Transform3 {
        matrix: [
            [ax.x, ay.x, az.x, 0.0],
            [ax.y, ay.y, az.y, 0.0],
            [ax.z, ay.z, az.z, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

/// Build one INSERT array cell's placement matrix, including the block's base
/// point and rotation.
///
/// The block's base point and rotation are folded in first (the same order as
/// the stored `INSERT`): a block-space point *p* lands at
/// `insert(OCS · scale · R · (p - base))`. `offset_x`/`offset_y` are the cell's
/// pre-scale displacement, so the array spacing is not scaled by the INSERT's
/// own scale factors.
fn insert_array_transform(
    i: &acadrust::entities::Insert,
    base_point: Point3,
    offset_x: f64,
    offset_y: f64,
) -> Transform3 {
    let ocs = ocs_to_wcs_transform(p3(i.normal));
    let (s, c) = i.rotation.sin_cos();
    let scale = Transform3 {
        matrix: [
            [i.x_scale(), 0.0, 0.0, offset_x],
            [0.0, i.y_scale(), 0.0, offset_y],
            [0.0, 0.0, i.z_scale(), 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let rotate = Transform3 {
        matrix: [
            [c, -s, 0.0, 0.0],
            [s, c, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let translate = Transform3::translation(p3(i.insert_point));
    ocs.matrix_mul(&translate)
        .matrix_mul(&rotate)
        .matrix_mul(&scale)
        .matrix_mul(&block_placement(base_point))
}

/// Every block referenced by expanded geometry (one per array cell), so render
/// status nesting resolves through MINSERTs as well.
fn referenced_blocks(geometry: &SemanticGeometry) -> Vec<BlockId> {
    match geometry {
        SemanticGeometry::Insert { block, .. } => vec![*block],
        SemanticGeometry::Compound(children) => {
            children.iter().flat_map(referenced_blocks).collect()
        }
        _ => Vec::new(),
    }
}

/// Expand a SOLID/TRACE into a mesh.
///
/// Unlike 3DFACE, a SOLID/TRACE stores its corners out of boundary order: the
/// visible quadrilateral runs first, second, fourth, third. The previous
/// importer emitted the stored order, which crossed the quad. Corners are also
/// lifted from the entity's OCS to WCS, so a tilted SOLID is not flattened onto
/// world XY (audit B31).
///
/// Returns `Partial` for the states this build cannot draw exactly: a
/// non-finite corner, or a thickness extrusion (a prism, not a flat fill) — the
/// flat face is still emitted.
fn solid_mesh_semantics(s: &acadrust::entities::Solid) -> (SemanticGeometry, Completeness) {
    let normal = polyline_normal(p3(s.normal));
    let finite = [
        s.first_corner,
        s.second_corner,
        s.third_corner,
        s.fourth_corner,
    ]
    .iter()
    .all(|v| is_finite_point(*v));
    if !finite {
        return (
            SemanticGeometry::Opaque {
                type_key: "AcDbTrace".into(),
                version: 1,
                payload: Vec::new(),
            },
            Completeness::Partial(vec!["SOLID/TRACE has a non-finite corner".into()]),
        );
    }
    let (ax, ay, az) = arbitrary_axis(normal);
    let w = |v: acadrust::types::Vector3| {
        cad_geometry::add(
            cad_geometry::add(cad_geometry::scale(ax, v.x), cad_geometry::scale(ay, v.y)),
            cad_geometry::scale(az, v.z),
        )
    };
    let first = w(s.first_corner);
    let second = w(s.second_corner);
    let third = w(s.third_corner);
    let fourth = w(s.fourth_corner);
    // Stored order is first, second, third, fourth; the visible boundary is
    // first, second, fourth, third.
    let boundary = [first, second, fourth, third];
    let mut completeness = Completeness::Complete;
    if s.thickness.abs() > 1e-12 {
        completeness = Completeness::Partial(vec![
            "thickness extrusion is not drawn (flat face only)".into(),
        ]);
    }
    (SemanticGeometry::Mesh(quad_mesh(boundary)), completeness)
}

fn dimension_transform(base: &DimensionBase) -> Transform3 {
    placement_transform(
        base.insertion_point,
        base.insertion_rotation,
        base.insertion_scale,
    )
}

/// Scale, then rotate about Z, then translate. Used by dimension blocks.
fn placement_transform(
    origin: acadrust::types::Vector3,
    rotation: f64,
    scale: acadrust::types::Vector3,
) -> Transform3 {
    let (s, c) = rotation.sin_cos();
    let mut m = [[0.0f64; 4]; 4];
    m[0][0] = c * scale.x;
    m[0][1] = -s * scale.y;
    m[1][0] = s * scale.x;
    m[1][1] = c * scale.y;
    m[2][2] = scale.z;
    m[3][3] = 1.0;
    m[0][3] = origin.x;
    m[1][3] = origin.y;
    m[2][3] = origin.z;
    Transform3 { matrix: m }
}

const HATCH_EPS: f64 = 1e-9;

/// A tessellation tolerance scaled to the hatch's own coordinate magnitude.
///
/// Hatch spline edges are tessellated eagerly (the pattern needs 2D loops), so
/// a fixed world tolerance would explode for drawings in the tens of thousands
/// of units. A relative tolerance keeps the segment count bounded.
fn hatch_tolerance(h: &Hatch) -> f64 {
    let mut magnitude = 0.0f64;
    let mut bump = |x: f64, y: f64| {
        magnitude = magnitude.max(x.abs()).max(y.abs());
    };
    for path in &h.paths {
        for edge in &path.edges {
            match edge {
                BoundaryEdge::Line(l) => {
                    bump(l.start.x, l.start.y);
                    bump(l.end.x, l.end.y);
                }
                BoundaryEdge::CircularArc(a) => {
                    bump(a.center.x, a.center.y);
                    bump(a.center.x + a.radius, a.center.y + a.radius);
                }
                BoundaryEdge::EllipticArc(a) => {
                    bump(a.center.x, a.center.y);
                    bump(
                        a.center.x + a.major_axis_endpoint.x,
                        a.center.y + a.major_axis_endpoint.y,
                    );
                }
                BoundaryEdge::Spline(s) => {
                    for p in &s.control_points {
                        bump(p.x, p.y);
                    }
                }
                BoundaryEdge::Polyline(pl) => {
                    for v in &pl.vertices {
                        bump(v.x, v.y);
                    }
                }
            }
        }
    }
    (magnitude * 1e-4).max(1e-6)
}

/// Append one HATCH boundary edge, sampled into the hatch plane (2D).
fn append_boundary_edge(edge: &BoundaryEdge, out: &mut Vec<[f64; 2]>, params: &TessellationParams) {
    match edge {
        BoundaryEdge::Line(l) => {
            push_hatch_point(out, [l.start.x, l.start.y]);
            push_hatch_point(out, [l.end.x, l.end.y]);
        }
        BoundaryEdge::CircularArc(a) => {
            let sweep = directed_sweep(a.start_angle, a.end_angle, a.counter_clockwise);
            let n = arc_steps(sweep);
            for i in 0..=n {
                let t = a.start_angle + sweep * (i as f64) / (n as f64);
                push_hatch_point(
                    out,
                    [
                        a.center.x + a.radius * t.cos(),
                        a.center.y + a.radius * t.sin(),
                    ],
                );
            }
        }
        BoundaryEdge::EllipticArc(a) => {
            let major = [a.major_axis_endpoint.x, a.major_axis_endpoint.y];
            let major_len = (major[0] * major[0] + major[1] * major[1]).sqrt();
            if major_len < HATCH_EPS {
                return;
            }
            let u = [major[0] / major_len, major[1] / major_len];
            let minor = [
                -u[1] * major_len * a.minor_axis_ratio,
                u[0] * major_len * a.minor_axis_ratio,
            ];
            let sweep = directed_sweep(a.start_angle, a.end_angle, a.counter_clockwise);
            let n = arc_steps(sweep);
            for i in 0..=n {
                let t = a.start_angle + sweep * (i as f64) / (n as f64);
                let (sin, cos) = t.sin_cos();
                push_hatch_point(
                    out,
                    [
                        a.center.x + u[0] * major_len * cos + minor[0] * sin,
                        a.center.y + u[1] * major_len * cos + minor[1] * sin,
                    ],
                );
            }
        }
        BoundaryEdge::Spline(s) => {
            if s.control_points.len() < 2 {
                return;
            }
            let control: Vec<Point3> = s.control_points.iter().map(|p| p3(*p)).collect();
            let degree = s.degree.max(1) as u32;
            for p in tessellate_bspline(&control, degree, *params) {
                push_hatch_point(out, [p.x, p.y]);
            }
        }
        BoundaryEdge::Polyline(pl) => {
            for v in &pl.vertices {
                push_hatch_point(out, [v.x, v.y]);
            }
            if pl.is_closed {
                if let Some(first) = pl.vertices.first() {
                    push_hatch_point(out, [first.x, first.y]);
                }
            }
        }
    }
}

fn push_hatch_point(out: &mut Vec<[f64; 2]>, p: [f64; 2]) {
    if let Some(last) = out.last() {
        if (last[0] - p[0]).abs() <= HATCH_EPS && (last[1] - p[1]).abs() <= HATCH_EPS {
            return;
        }
    }
    out.push(p);
}

/// Drop a duplicated closing vertex (loops are closed implicitly).
fn dedup_loop(points: &mut Vec<[f64; 2]>) {
    if points.len() >= 2 {
        let first = points[0];
        let last = points[points.len() - 1];
        if (first[0] - last[0]).abs() <= HATCH_EPS && (first[1] - last[1]).abs() <= HATCH_EPS {
            points.pop();
        }
    }
}

fn directed_sweep(start: f64, end: f64, counter_clockwise: bool) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut sweep = end - start;
    if counter_clockwise {
        while sweep <= 0.0 {
            sweep += tau;
        }
    } else {
        while sweep >= 0.0 {
            sweep -= tau;
        }
    }
    sweep
}

fn arc_steps(sweep: f64) -> usize {
    ((sweep.abs() / 0.05).ceil() as usize).clamp(2, 4096)
}

/// Build the (possibly doubled) pattern line families, applying the hatch's
/// pattern angle and scale.
fn pattern_families(h: &Hatch) -> Vec<PatternLine> {
    let (sin, cos) = h.pattern_angle.sin_cos();
    let mut families = Vec::new();
    for line in &h.pattern.lines {
        let base_raw = [line.base_point.x, line.base_point.y];
        let base = [
            cos * base_raw[0] - sin * base_raw[1],
            sin * base_raw[0] + cos * base_raw[1],
        ];
        let offset = [
            line.offset.x * h.pattern_scale,
            line.offset.y * h.pattern_scale,
        ];
        let dashes = line
            .dash_lengths
            .iter()
            .map(|d| d * h.pattern_scale)
            .collect::<Vec<_>>();
        let angle = line.angle + h.pattern_angle;
        families.push(PatternLine {
            angle,
            base,
            offset,
            dashes: dashes.clone(),
        });
        if h.is_double {
            families.push(PatternLine {
                angle: angle + std::f64::consts::FRAC_PI_2,
                base,
                offset: [-offset[1], offset[0]],
                dashes,
            });
        }
    }
    families
}

fn weaker(a: SupportStatus, b: SupportStatus) -> SupportStatus {
    let rank = |s: &SupportStatus| match s {
        SupportStatus::Verified => 4,
        SupportStatus::Partial => 3,
        SupportStatus::Unverified => 2,
        SupportStatus::Unsupported => 1,
        SupportStatus::NotImplemented => 0,
    };
    match (rank(&a), rank(&b)) {
        (x, y) if x <= y => a,
        _ => b,
    }
}

fn entity_class_name(e: &EntityType) -> String {
    match e {
        EntityType::Line(_) => "AcDbLine",
        EntityType::Circle(_) => "AcDbCircle",
        EntityType::Arc(_) => "AcDbArc",
        EntityType::Ellipse(_) => "AcDbEllipse",
        EntityType::Point(_) => "AcDbPoint",
        EntityType::LwPolyline(_) => "AcDbPolyline",
        EntityType::Polyline2D(_) => "AcDb2dPolyline",
        EntityType::Polyline3D(_) => "AcDb3dPolyline",
        EntityType::Text(_) => "AcDbText",
        EntityType::MText(_) => "AcDbMText",
        EntityType::Spline(_) => "AcDbSpline",
        EntityType::Solid(_) => "AcDbTrace",
        EntityType::Face3D(_) => "AcDbFace",
        EntityType::Insert(_) => "AcDbBlockReference",
        EntityType::Solid3D(_) => "AcDb3dSolid",
        EntityType::Region(_) => "AcDbRegion",
        EntityType::Body(_) => "AcDbBody",
        EntityType::Surface(_) => "AcDbSurface",
        EntityType::Hatch(_) => "AcDbHatch",
        EntityType::Dimension(_) => "AcDbDimension",
        EntityType::Unknown(u) => return u.dxf_name.clone(),
        EntityType::Extended(x) => return x.class_name().to_string(),
        other => {
            return format!("{other:?}")
                .split('(')
                .next()
                .unwrap_or("Unknown")
                .to_string()
        }
    }
    .to_string()
}

/// A DWG file starts with an `AC10xx` version signature.
fn looks_like_dwg(bytes: &[u8]) -> bool {
    bytes.len() >= 6 && &bytes[0..2] == b"AC" && bytes[2..6].iter().all(|b| b.is_ascii_digit())
}

/// True for the model/paper space records rather than a user block.
///
/// DWG version differences use mixed case (`*Model_Space`) and upper case
/// (`*MODEL_SPACE`, `*PAPER_SPACE`); matching only one casing imports the model
/// space twice (audit: R14 files).
fn is_space_block_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper == "*MODEL_SPACE" || upper.starts_with("*PAPER_SPACE")
}

fn is_paper_space_name(name: &str) -> bool {
    name.to_ascii_uppercase().starts_with("*PAPER_SPACE")
}

fn compute_identity(bytes: &[u8]) -> DocumentIdentity {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    DocumentIdentity::Sha256(out)
}

trait HandleExt {
    fn null_or_value_zero(&self) -> bool;
}

impl HandleExt for acadrust::Handle {
    fn null_or_value_zero(&self) -> bool {
        self.value() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::entities::BoundaryPath;

    fn request(bytes: Vec<u8>) -> ImportRequest {
        ImportRequest {
            document: DocumentId(1),
            database: DatabaseId(1),
            bytes: Arc::from(bytes.into_boxed_slice()),
            limits: ImportLimits::default(),
            generation: 0,
        }
    }

    #[test]
    fn garbage_input_fails_without_panicking() {
        let importer = AcadrustImporter::new();
        let result = importer.import(&request(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]), &|| {
            false
        });
        assert!(result.is_err(), "random bytes must not import as a drawing");
    }

    #[test]
    fn oversize_input_is_rejected_before_parsing() {
        let importer = AcadrustImporter::new();
        let mut req = request(vec![0u8; 64]);
        req.limits.max_file_bytes = 8;
        assert!(matches!(
            importer.import(&req, &|| false),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn cancellation_is_honoured() {
        let importer = AcadrustImporter::new();
        let result = importer.import(&request(vec![0u8; 64]), &|| true);
        assert!(matches!(result, Err(CadError::Cancelled)));
    }

    #[test]
    fn space_block_names_are_case_insensitive() {
        for name in ["*Model_Space", "*MODEL_SPACE", "*model_space"] {
            assert!(is_space_block_name(name), "{name}");
        }
        for name in ["*Paper_Space", "*PAPER_SPACE", "*Paper_Space0"] {
            assert!(is_space_block_name(name), "{name}");
            assert!(is_paper_space_name(name), "{name}");
        }
        assert!(!is_space_block_name("MyBlock"));
        assert!(!is_paper_space_name("*Model_Space"));
    }

    #[test]
    fn display_support_separates_drawn_from_unrendered() {
        let line = SemanticGeometry::Line {
            start: Point3::default(),
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        };
        assert_eq!(display_support(&line).0, SupportStatus::Verified);
        let text = SemanticGeometry::Text {
            text: "x".into(),
            position: Point3::default(),
            style: StyleId(0),
            height: 1.0,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        };
        assert_eq!(display_support(&text).0, SupportStatus::Unsupported);
        let opaque = SemanticGeometry::Opaque {
            type_key: "ACIS".into(),
            version: 1,
            payload: Vec::new(),
        };
        assert_eq!(display_support(&opaque).0, SupportStatus::Unsupported);
        let insert = SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::identity(),
        };
        assert_eq!(display_support(&insert).0, SupportStatus::Unverified);
    }

    #[test]
    fn completeness_never_reports_complete_for_unrendered_content() {
        // Text-only drawing: parsed but not drawable -> Missing, never Complete.
        let c = aggregate_completeness(
            SupportStatus::Unsupported,
            false,
            vec!["AcDbText".into()],
            vec![],
        );
        assert!(matches!(c, Completeness::Missing(_)), "{c:?}");
        // Mixed drawable + unrendered -> Partial.
        let c = aggregate_completeness(
            SupportStatus::Unsupported,
            true,
            vec!["AcDbText".into()],
            vec![],
        );
        assert!(matches!(c, Completeness::Partial(_)), "{c:?}");
        // Fully drawable -> Complete.
        assert_eq!(
            aggregate_completeness(SupportStatus::Verified, true, vec![], vec![]),
            Completeness::Complete
        );
        // Empty drawing with no fault -> Complete.
        assert_eq!(
            aggregate_completeness(SupportStatus::Verified, false, vec![], vec![]),
            Completeness::Complete
        );
        // A separate import fault is still Partial, not Complete.
        let c = aggregate_completeness(
            SupportStatus::Verified,
            false,
            vec![],
            vec!["stream".into()],
        );
        assert!(matches!(c, Completeness::Partial(_)), "{c:?}");
    }

    #[test]
    fn dimension_placement_scales_rotates_then_translates() {
        // Scale (2,2,1), rotate +90 deg about Z, translate (1,2,3).
        let t = placement_transform(
            acadrust::types::Vector3::new(1.0, 2.0, 3.0),
            std::f64::consts::FRAC_PI_2,
            acadrust::types::Vector3::new(2.0, 2.0, 1.0),
        );
        // (1,0,0) -> scale (2,0,0) -> rotate (0,2,0) -> translate (1,4,3).
        let p = t.apply_point(Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        });
        assert!((p.x - 1.0).abs() < 1e-9, "{p:?}");
        assert!((p.y - 4.0).abs() < 1e-9, "{p:?}");
        assert!((p.z - 3.0).abs() < 1e-9, "{p:?}");
    }

    #[test]
    fn hatch_sweeps_follow_their_winding() {
        let tau = std::f64::consts::TAU;
        let half = std::f64::consts::PI / 2.0;
        assert!((directed_sweep(half, 0.0, false) + half).abs() < 1e-9);
        assert!((directed_sweep(0.0, half, true) - half).abs() < 1e-9);
        // A full turn is kept, not collapsed to zero.
        assert!((directed_sweep(0.0, 0.0, true) - tau).abs() < 1e-9);
    }

    fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> BoundaryPath {
        let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        let mut path = BoundaryPath::new();
        for i in 0..4 {
            let a = corners[i];
            let b = corners[(i + 1) % 4];
            path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
                start: acadrust::types::Vector2::new(a.0, a.1),
                end: acadrust::types::Vector2::new(b.0, b.1),
            }));
        }
        path
    }

    fn mesh_area(geometry: &SemanticGeometry) -> Option<f64> {
        let SemanticGeometry::Compound(children) = geometry else {
            return None;
        };
        children.iter().find_map(|child| match child {
            SemanticGeometry::Mesh(m) => Some(
                m.triangles
                    .iter()
                    .map(|t| {
                        let a = m.vertices[t[0] as usize];
                        let b = m.vertices[t[1] as usize];
                        let c = m.vertices[t[2] as usize];
                        let ab = Point3 {
                            x: b.x - a.x,
                            y: b.y - a.y,
                            z: b.z - a.z,
                        };
                        let ac = Point3 {
                            x: c.x - a.x,
                            y: c.y - a.y,
                            z: c.z - a.z,
                        };
                        let cx = ab.y * ac.z - ab.z * ac.y;
                        let cy = ab.z * ac.x - ab.x * ac.z;
                        let cz = ab.x * ac.y - ab.y * ac.x;
                        (cx * cx + cy * cy + cz * cz).sqrt() / 2.0
                    })
                    .sum(),
            ),
            _ => None,
        })
    }

    #[test]
    fn solid_hatch_with_a_hole_fills_the_solid_band_only() {
        let mut hatch = acadrust::entities::Hatch::new();
        hatch.is_solid = true;
        hatch.paths = vec![
            rect_path(0.0, 0.0, 10.0, 10.0),
            rect_path(3.0, 3.0, 7.0, 7.0),
        ];
        let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
        assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
        let area = mesh_area(&geometry).expect("solid fill mesh");
        assert!((area - 84.0).abs() < 1e-6, "hole area not excluded: {area}");
    }

    #[test]
    fn over_budget_multi_ring_hatch_stays_partial_boundary_only() {
        // A zig-zag star defeats Douglas-Peucker, so the loop stays over
        // MAX_FILL_POINTS and the fill must be refused, never approximated.
        let points = 2200usize;
        let mut path = BoundaryPath::new();
        for i in 0..points {
            let t = i as f64 / points as f64 * std::f64::consts::TAU;
            let r = if i % 2 == 0 { 10.0 } else { 1.0 };
            let a = (r * t.cos(), r * t.sin());
            let t2 = (i + 1) as f64 / points as f64 * std::f64::consts::TAU;
            let r2 = if (i + 1) % 2 == 0 { 10.0 } else { 1.0 };
            let b = (r2 * t2.cos(), r2 * t2.sin());
            path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
                start: acadrust::types::Vector2::new(a.0, a.1),
                end: acadrust::types::Vector2::new(b.0, b.1),
            }));
        }
        let mut hatch = acadrust::entities::Hatch::new();
        hatch.is_solid = true;
        hatch.paths = vec![path, rect_path(0.0, 0.0, 20.0, 20.0)];
        let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
        match completeness {
            Completeness::Partial(reasons) => {
                assert!(reasons.iter().any(|r| r.contains("budget")), "{reasons:?}");
            }
            other => panic!("expected Partial, got {other:?}"),
        }
        // The boundary loops are still present; no fill mesh was fabricated.
        assert!(mesh_area(&geometry).is_none(), "{geometry:?}");
    }

    #[test]
    fn degenerate_hatch_normal_reports_partial_boundary_only() {
        let mut hatch = acadrust::entities::Hatch::new();
        hatch.is_solid = true;
        hatch.normal = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
        hatch.paths = vec![rect_path(0.0, 0.0, 4.0, 4.0)];
        let (_geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
        match completeness {
            Completeness::Partial(reasons) => {
                assert!(reasons.iter().any(|r| r.contains("normal")), "{reasons:?}");
            }
            other => panic!("expected Partial, got {other:?}"),
        }
    }

    // ---- B23/B31: OCS normalisation of 2D polylines (importer side) ----

    fn x_axis() -> Point3 {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    }

    #[test]
    fn world_z_extrusion_is_left_untouched() {
        // The common case must stay exactly `(x, y, elevation)`.
        let pts = polyline_ocs_points(world_z(), 7.0, [(1.0, 2.0), (3.0, 4.0)]);
        assert_eq!(
            pts,
            vec![
                Point3 {
                    x: 1.0,
                    y: 2.0,
                    z: 7.0
                },
                Point3 {
                    x: 3.0,
                    y: 4.0,
                    z: 7.0
                },
            ]
        );
    }

    #[test]
    fn non_z_extrusion_is_transformed_to_wcs_not_treated_as_flat() {
        // Extrusion +X: the AutoCAD arbitrary axis gives ax=+Y, ay=+Z, so an
        // OCS point (x, y) at elevation e maps to (e, x, y). The old code
        // returned (x, y, e) and treated the tilted entity as flat.
        let p = ocs_to_wcs(x_axis(), 5.0, 2.0, 3.0);
        assert!((p.x - 5.0).abs() < 1e-9, "{p:?}");
        assert!((p.y - 2.0).abs() < 1e-9, "{p:?}");
        assert!((p.z - 3.0).abs() < 1e-9, "{p:?}");
        // The mapped point must lie on the extrusion plane through elevation.
        assert!(!is_world_z(x_axis()));
    }

    #[test]
    fn tilted_lwpolyline_vertices_carry_the_ocs_plane() {
        // A real acadrust entity, no DWG needed: a two-vertex LWPOLYLINE with
        // an +X extrusion and elevation 4.
        let mut pl = acadrust::entities::LwPolyline::from_points(vec![
            acadrust::types::Vector2::new(1.0, 0.0),
            acadrust::types::Vector2::new(0.0, 2.0),
        ]);
        pl.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
        pl.elevation = 4.0;

        let points = polyline_ocs_points(
            p3(pl.normal),
            pl.elevation,
            pl.vertices.iter().map(|v| (v.location.x, v.location.y)),
        );
        // (1,0) -> (4,1,0); (0,2) -> (4,0,2).
        assert_eq!(
            points,
            vec![
                Point3 {
                    x: 4.0,
                    y: 1.0,
                    z: 0.0
                },
                Point3 {
                    x: 4.0,
                    y: 0.0,
                    z: 2.0
                },
            ]
        );
        // Both vertices share the plane x = elevation; not flat on XY.
        assert!(points.iter().all(|p| (p.x - 4.0).abs() < 1e-9));
        assert!(points.iter().any(|p| p.z.abs() > 1e-9));
    }

    #[test]
    fn tilted_bulge_completeness_tracks_plane_representability() {
        // A tilted polyline with >=3 vertices fixes its own plane, so a bulge
        // is exact and the import is Complete. A two-vertex tilted bulge has no
        // unique plane and must not claim Complete.
        assert_eq!(
            polyline_completeness(x_axis(), 4, &[0.0, 0.0, 0.0, 0.0]),
            Completeness::Complete
        );
        assert_eq!(
            polyline_completeness(x_axis(), 4, &[0.5, 0.0, 0.0, 0.0]),
            Completeness::Complete
        );
        assert!(matches!(
            polyline_completeness(x_axis(), 2, &[0.5, 0.0]),
            Completeness::Partial(_)
        ));
        // A world-Z bulge polyline is always fully supported.
        assert_eq!(
            polyline_completeness(world_z(), 2, &[0.5, 0.0]),
            Completeness::Complete
        );
    }

    #[test]
    fn tilted_ellipse_keeps_its_normal_and_is_complete() {
        // A +X extrusion is now carried exactly (the domain Ellipse has a
        // normal), so the ellipse is no longer flattened onto world XY.
        let mut e = acadrust::entities::Ellipse::new();
        e.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
        e.center = acadrust::types::Vector3::new(2.0, 3.0, 4.0);
        e.major_axis = acadrust::types::Vector3::new(0.0, 5.0, 0.0);
        let (geom, completeness) = ellipse_semantics(&e);
        match geom {
            SemanticGeometry::Ellipse {
                normal,
                major_axis,
                ratio,
                ..
            } => {
                assert!((normal.x - 1.0).abs() < 1e-12, "normal {normal:?}");
                assert!(
                    cad_geometry::length(major_axis) >= 4.0,
                    "major axis lost: {major_axis:?}"
                );
                assert!(ratio > 0.0);
            }
            other => panic!("expected ellipse, got {other:?}"),
        }
        assert_eq!(completeness, Completeness::Complete);

        // A degenerate extrusion defaults to world Z rather than collapsing.
        e.normal = acadrust::types::Vector3::ZERO;
        match ellipse_semantics(&e).0 {
            SemanticGeometry::Ellipse { normal, .. } => {
                assert!((normal.z - 1.0).abs() < 1e-12, "normal {normal:?}");
            }
            other => panic!("expected ellipse, got {other:?}"),
        }
    }

    #[test]
    fn tilted_circle_centre_is_mapped_from_ocs_to_wcs() {
        // A CIRCLE stores its centre in OCS. With a +X extrusion the arbitrary
        // axis frame maps OCS (x, y) at elevation z to WCS (z, x, y); reading
        // the centre verbatim would put the circle in the wrong plane.
        let mut c = acadrust::entities::Circle::new();
        c.center = acadrust::types::Vector3::new(2.0, 3.0, 5.0);
        c.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
        let wcs = c.center_wcs();
        let expected = ocs_to_wcs(
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            5.0,
            2.0,
            3.0,
        );
        assert!((wcs.x - expected.x).abs() < 1e-9);
        assert!((wcs.y - expected.y).abs() < 1e-9);
        assert!((wcs.z - expected.z).abs() < 1e-9);
    }

    // ---- B22: paper-space viewport → four corners + full transform ----

    /// Build a top/plan VIEWPORT entity with the given paper rectangle, scale,
    /// and model view target.
    fn top_viewport(
        center: (f64, f64),
        w: f64,
        h: f64,
        view_height: f64,
        view_target: (f64, f64),
    ) -> acadrust::entities::Viewport {
        let mut v = acadrust::entities::Viewport::new();
        v.id = 2;
        v.center = acadrust::types::Vector3::new(center.0, center.1, 0.0);
        v.width = w;
        v.height = h;
        v.view_height = view_height;
        v.view_direction = acadrust::types::Vector3::UNIT_Z;
        v.view_target = acadrust::types::Vector3::new(view_target.0, view_target.1, 0.0);
        v
    }

    #[test]
    fn one_to_one_hundred_viewport_has_four_corners_and_the_real_transform() {
        // 1:100: a 10000×5000 model region fits the 100×50 paper window, view
        // centre (10, 20).
        let vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
        let pv = paper_viewport(&vp).expect("id 2 is a content viewport");
        assert_eq!(pv.completeness, Completeness::Complete);
        assert_eq!(
            pv.clip,
            vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0
                },
                Point3 {
                    x: 100.0,
                    y: 0.0,
                    z: 0.0
                },
                Point3 {
                    x: 100.0,
                    y: 50.0,
                    z: 0.0
                },
                Point3 {
                    x: 0.0,
                    y: 50.0,
                    z: 0.0
                },
            ]
        );
        // The stored paper→model transform maps the paper centre to the view
        // target; 100 model units per paper unit.
        let m = &pv.model_to_paper.matrix;
        assert!((m[0][0] - 100.0).abs() < 1e-9, "{m:?}");
        assert!((m[1][1] - 100.0).abs() < 1e-9, "{m:?}");
        let model_centre = pv.model_to_paper.apply_point(Point3 {
            x: 50.0,
            y: 25.0,
            z: 0.0,
        });
        assert!((model_centre.x - 10.0).abs() < 1e-9, "{model_centre:?}");
        assert!((model_centre.y - 20.0).abs() < 1e-9, "{model_centre:?}");
        // One paper unit right of centre is 100 model units.
        let right = pv.model_to_paper.apply_point(Point3 {
            x: 51.0,
            y: 25.0,
            z: 0.0,
        });
        assert!((right.x - 110.0).abs() < 1e-9, "{right:?}");
    }

    #[test]
    fn sheet_viewport_and_off_viewport_are_not_content_viewports() {
        let mut sheet = top_viewport((0.0, 0.0), 100.0, 100.0, 100.0, (0.0, 0.0));
        sheet.id = 1;
        assert!(paper_viewport(&sheet).is_none(), "id 1 is the sheet");
        let mut off = top_viewport((0.0, 0.0), 100.0, 100.0, 100.0, (0.0, 0.0));
        off.status.is_on = false;
        assert!(
            paper_viewport(&off).is_none(),
            "an off viewport draws nothing"
        );
    }

    #[test]
    fn rotated_viewport_is_partial_with_a_twist_reason() {
        let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
        vp.twist_angle = std::f64::consts::FRAC_PI_4;
        let pv = paper_viewport(&vp).expect("still a content viewport");
        match &pv.completeness {
            Completeness::Partial(reasons) => {
                assert!(reasons.iter().any(|r| r.contains("twist")), "{reasons:?}");
            }
            other => panic!("expected Partial, got {other:?}"),
        }
        // The four-corner rectangle is still present (the representation layer
        // will refuse it with `viewport.twisted_transform`, never square it off).
        assert_eq!(pv.clip.len(), 4);
    }

    #[test]
    fn non_perpendicular_view_is_partial_with_an_off_plane_reason() {
        let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
        vp.view_direction = acadrust::types::Vector3::new(1.0, 0.0, 1.0);
        let pv = paper_viewport(&vp).unwrap();
        match &pv.completeness {
            Completeness::Partial(reasons) => {
                assert!(
                    reasons.iter().any(|r| r.contains("perpendicular")),
                    "{reasons:?}"
                );
            }
            other => panic!("expected Partial, got {other:?}"),
        }
    }

    #[test]
    fn complex_clip_viewport_is_partial() {
        let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
        vp.clip_boundary_handle = acadrust::types::Handle::new(0x1A);
        let pv = paper_viewport(&vp).unwrap();
        match &pv.completeness {
            Completeness::Partial(reasons) => {
                assert!(reasons.iter().any(|r| r.contains("clip")), "{reasons:?}");
            }
            other => panic!("expected Partial, got {other:?}"),
        }
    }

    // ---- B31: INSERT base point, OCS normal and array semantics ----

    #[test]
    fn insert_subtracts_the_block_base_point() {
        // A block whose base point is (1, 1) and an INSERT at (10, 10): the
        // block point (1, 1) must land on (10, 10), not (11, 11).
        let mut i = acadrust::entities::Insert::new(
            "BLOCK",
            acadrust::types::Vector3::new(10.0, 10.0, 0.0),
        );
        i.rotation = 0.0;
        let t = insert_array_transform(
            &i,
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
            0.0,
            0.0,
        );
        let placed = t.apply_point(Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        });
        assert!((placed.x - 10.0).abs() < 1e-9, "{placed:?}");
        assert!((placed.y - 10.0).abs() < 1e-9, "{placed:?}");
        // The origin with base (1,1) lands one block unit left/below the insert.
        let origin = t.apply_point(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        assert!((origin.x - 9.0).abs() < 1e-9, "{origin:?}");
        assert!((origin.y - 9.0).abs() < 1e-9, "{origin:?}");
    }

    #[test]
    fn insert_positive_rotation_turns_counter_clockwise() {
        let mut i = acadrust::entities::Insert::new("BLOCK", acadrust::types::Vector3::ZERO);
        i.rotation = std::f64::consts::FRAC_PI_2;
        // Base point (1,0). A block point (2,0) is (1,0) after base subtraction;
        // +90° CCW turns it to (0,1).
        let t = insert_array_transform(
            &i,
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            0.0,
            0.0,
        );
        let placed = t.apply_point(Point3 {
            x: 2.0,
            y: 0.0,
            z: 0.0,
        });
        assert!(placed.x.abs() < 1e-9, "{placed:?}");
        assert!((placed.y - 1.0).abs() < 1e-9, "{placed:?}");
        // The base point itself lands on the insert point (the origin).
        let base = t.apply_point(Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        });
        assert!(base.x.abs() < 1e-9 && base.y.abs() < 1e-9, "{base:?}");
    }

    #[test]
    fn insert_ocs_normal_lifts_the_block_off_the_xy_plane() {
        // A +X extrusion maps the OCS X/Y axes into world Y/Z, so a block
        // point on its local X lands off the world XY plane (audit B31).
        let mut i = acadrust::entities::Insert::new("BLOCK", acadrust::types::Vector3::ZERO);
        i.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
        let t = insert_array_transform(&i, Point3::default(), 0.0, 0.0);
        let w = t.apply_point(Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        });
        // arbitrary_axis(+X) = (ax=+Y, ay=+Z); OCS (1,0) -> world (0,1).
        assert!(w.x.abs() < 1e-9, "{w:?}");
        assert!((w.y - 1.0).abs() < 1e-9, "{w:?}");
        assert!(w.z.abs() < 1e-9, "{w:?}");
    }

    #[test]
    fn insert_array_offsets_are_pre_scale_and_row_major() {
        let mut i = acadrust::entities::Insert::new("BLOCK", acadrust::types::Vector3::ZERO);
        // Non-uniform scale: the 10-unit column spacing must not be scaled by
        // the 2× x-scale.
        i.set_x_scale(2.0);
        i.set_y_scale(3.0);
        i.column_count = 2;
        i.row_count = 2;
        i.column_spacing = 10.0;
        i.row_spacing = 20.0;

        let cell = |col: usize, row: usize| {
            insert_array_transform(
                &i,
                Point3::default(),
                col as f64 * i.column_spacing,
                row as f64 * i.row_spacing,
            )
            .apply_point(Point3::default())
        };
        // Row-major: cells are (col 0,row 0), (col 1,row 0), ...
        assert_eq!(cell(0, 0).x, 0.0);
        assert_eq!(cell(1, 0).x, 10.0);
        assert_eq!(cell(0, 1).x, 0.0);
        assert_eq!(cell(0, 1).y, 20.0);
    }

    #[test]
    fn array_insert_is_a_compound_of_cell_instances() {
        // referenced_blocks must see every cell so status nesting resolves
        // through MINSERTs; insert_semantics builds one Instance per cell.
        let geometry = SemanticGeometry::Compound(vec![
            SemanticGeometry::Insert {
                block: BlockId(7),
                transform: Transform3::identity(),
            },
            SemanticGeometry::Insert {
                block: BlockId(7),
                transform: Transform3::identity(),
            },
        ]);
        assert_eq!(referenced_blocks(&geometry), vec![BlockId(7), BlockId(7)]);
        assert!(referenced_blocks(&SemanticGeometry::Line {
            start: Point3::default(),
            end: Point3::default(),
        })
        .is_empty());
    }

    #[test]
    fn missing_block_is_reported_missing_not_an_empty_success() {
        // A bare builder (no block table) resolves the insert block to the
        // sentinel id; the record must be `Missing`, never a silent success.
        let bytes = vec![0u8; 0];
        let req = request(bytes);
        let acad = acadrust::CadDocument::new();
        let stats = ReadStats::default();
        let builder = ImporterBuilder::new(&req, &acad, stats, compute_identity(&[]));
        let mut i = acadrust::entities::Insert::new("NOPE", acadrust::types::Vector3::ZERO);
        i.column_count = 1;
        i.row_count = 1;
        let (geometry, completeness) = builder.insert_semantics(&i);
        assert!(matches!(geometry, SemanticGeometry::Insert { .. }));
        match completeness {
            Completeness::Missing(reasons) => {
                assert!(reasons.iter().any(|r| r.contains("NOPE")), "{reasons:?}");
            }
            other => panic!("expected Missing, got {other:?}"),
        }
    }

    // ---- B31: SOLID/TRACE boundary order and OCS lift ----

    #[test]
    fn solid_corners_use_the_visible_boundary_order() {
        // Stored corners 1,2,3,4 with the visible quad 1,2,4,3. The extruded
        // triangles must follow the boundary, not the stored order.
        let mut s = acadrust::entities::Solid::new(
            acadrust::types::Vector3::new(0.0, 0.0, 0.0),
            acadrust::types::Vector3::new(2.0, 0.0, 0.0),
            acadrust::types::Vector3::new(0.0, 2.0, 0.0),
            acadrust::types::Vector3::new(2.0, 2.0, 0.0),
        );
        s.normal = acadrust::types::Vector3::UNIT_Z;
        let (geometry, completeness) = solid_mesh_semantics(&s);
        assert_eq!(completeness, Completeness::Complete);
        let SemanticGeometry::Mesh(mesh) = geometry else {
            panic!("expected a mesh");
        };
        // Boundary order: (0,0), (2,0), (2,2), (0,2). Triangles (0,1,2) and
        // (0,2,3) must be two triangles of a unit square (area 4 total).
        let area: f64 = mesh
            .triangles
            .iter()
            .map(|t| {
                let a = mesh.vertices[t[0] as usize];
                let b = mesh.vertices[t[1] as usize];
                let c = mesh.vertices[t[2] as usize];
                let ab = Point3 {
                    x: b.x - a.x,
                    y: b.y - a.y,
                    z: 0.0,
                };
                let ac = Point3 {
                    x: c.x - a.x,
                    y: c.y - a.y,
                    z: 0.0,
                };
                (ab.x * ac.y - ab.y * ac.x).abs() / 2.0
            })
            .sum();
        assert!((area - 4.0).abs() < 1e-9, "crossed quad area {area}");
    }

    #[test]
    fn solid_with_a_non_z_extrusion_is_lifted_to_wcs() {
        let mut s = acadrust::entities::Solid::new(
            acadrust::types::Vector3::new(1.0, 0.0, 0.0),
            acadrust::types::Vector3::new(0.0, 1.0, 0.0),
            acadrust::types::Vector3::new(0.0, 0.0, 1.0),
            acadrust::types::Vector3::new(1.0, 1.0, 1.0),
        );
        s.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
        let (geometry, completeness) = solid_mesh_semantics(&s);
        assert_eq!(completeness, Completeness::Complete);
        let SemanticGeometry::Mesh(mesh) = geometry else {
            panic!("expected a mesh");
        };
        // With +X extrusion the arbitrary-axis frame maps OCS (x, y, z) to
        // world (z, x, y). The old flat treatment would have left the first
        // corner at (1,0,0); the lift must move it to (0,1,0).
        assert!(mesh
            .vertices
            .iter()
            .any(|p| { (p.x).abs() < 1e-9 && (p.y - 1.0).abs() < 1e-9 && p.z.abs() < 1e-9 }));
        // The fourth OCS corner (1,1,1) -> world (1,1,1).
        assert!(mesh.vertices.iter().any(|p| {
            (p.x - 1.0).abs() < 1e-9 && (p.y - 1.0).abs() < 1e-9 && (p.z - 1.0).abs() < 1e-9
        }));
    }

    #[test]
    fn solid_thickness_is_partial_flat_face_only() {
        let mut s = acadrust::entities::Solid::new(
            acadrust::types::Vector3::new(0.0, 0.0, 0.0),
            acadrust::types::Vector3::new(1.0, 0.0, 0.0),
            acadrust::types::Vector3::new(0.0, 1.0, 0.0),
            acadrust::types::Vector3::new(1.0, 1.0, 0.0),
        );
        s.thickness = 5.0;
        let (_geometry, completeness) = solid_mesh_semantics(&s);
        match completeness {
            Completeness::Partial(reasons) => {
                assert!(
                    reasons.iter().any(|r| r.contains("thickness")),
                    "{reasons:?}"
                );
            }
            other => panic!("expected Partial, got {other:?}"),
        }
    }

    // ---- B23/B31: tilted OCS polyline length ----

    #[test]
    fn tilted_ocs_polyline_preserves_its_segment_length() {
        // A 3-4-5 triangle drawn flat on the OCS XY plane at an +X extrusion.
        // Lifting it to WCS must preserve each segment's length exactly, while
        // the old flat treatment would have collapsed it onto the XY plane.
        let mut pl = acadrust::entities::LwPolyline::from_points(vec![
            acadrust::types::Vector2::new(0.0, 0.0),
            acadrust::types::Vector2::new(3.0, 0.0),
            acadrust::types::Vector2::new(3.0, 4.0),
        ]);
        pl.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
        pl.elevation = 7.0;
        let points = polyline_ocs_points(
            p3(pl.normal),
            pl.elevation,
            pl.vertices.iter().map(|v| (v.location.x, v.location.y)),
        );
        let seg = |a: Point3, b: Point3| {
            let d = Point3 {
                x: b.x - a.x,
                y: b.y - a.y,
                z: b.z - a.z,
            };
            (d.x * d.x + d.y * d.y + d.z * d.z).sqrt()
        };
        assert!((seg(points[0], points[1]) - 3.0).abs() < 1e-9, "{points:?}");
        assert!((seg(points[1], points[2]) - 4.0).abs() < 1e-9, "{points:?}");
        // Both vertices lie on the x = elevation plane.
        assert!(points.iter().all(|p| (p.x - 7.0).abs() < 1e-9));
    }

    // ---- B23: source spline knots and weights survive the importer ----

    #[test]
    fn source_spline_knots_and_weights_survive_import() {
        let mut s = acadrust::entities::Spline::new();
        s.degree = 2;
        // Explicit non-uniform clamped knots, not the uniform ones acadrust
        // would fabricate.
        s.knots = vec![0.0, 0.0, 0.0, 0.3, 0.7, 1.0, 1.0, 1.0];
        s.control_points = vec![
            acadrust::types::Vector3::new(0.0, 0.0, 0.0),
            acadrust::types::Vector3::new(1.0, 2.0, 0.0),
            acadrust::types::Vector3::new(2.0, 0.0, 0.0),
            acadrust::types::Vector3::new(3.0, 2.0, 0.0),
            acadrust::types::Vector3::new(4.0, 0.0, 0.0),
        ];
        s.weights = vec![1.0, 3.0, 1.0, 3.0, 1.0];

        let (geometry, completeness) = spline_semantics(&s);
        match geometry {
            SemanticGeometry::Spline {
                degree,
                knots,
                control_points,
                weights,
            } => {
                assert_eq!(degree, 2);
                assert_eq!(knots, s.knots, "source knots must not be uniformised");
                assert_eq!(weights, s.weights, "source weights must survive");
                assert_eq!(control_points.len(), 5);
            }
            other => panic!("expected spline, got {other:?}"),
        }
        // Rational splines are evaluated from the source knots and weights, so
        // the record is complete rather than downgraded.
        assert_eq!(completeness, Completeness::Complete);

        // A mismatched weight vector is the honest Partial case.
        s.weights = vec![1.0, 3.0];
        assert!(matches!(spline_semantics(&s).1, Completeness::Partial(_)));
    }

    // ---- F14/B21: transparency resolution + proxy fragment preservation ----

    #[test]
    fn transparency_resolution_prefers_byobject_then_layer_then_byblock() {
        // acadrust packs transparency as a byte: 0 opaque, 255 transparent.
        assert_eq!(
            resolve_entity_transparency(acadrust::Transparency::new(0), 0.5),
            EntityTransparency::Explicit(1.0)
        );
        // ByObject (Explicit) overrides the layer value.
        assert_eq!(
            resolve_entity_transparency(acadrust::Transparency::new(128), 0.5),
            EntityTransparency::Explicit(1.0 - 128.0 / 255.0)
        );
        // ByLayer uses the pre-resolved layer opacity.
        assert_eq!(
            resolve_entity_transparency(acadrust::Transparency::BY_LAYER, 0.25),
            EntityTransparency::Explicit(0.25)
        );
        // ByBlock stays symbolic so INSERT expansion can supply the value.
        assert_eq!(
            resolve_entity_transparency(acadrust::Transparency::BY_BLOCK, 0.25),
            EntityTransparency::ByBlock
        );
    }

    #[test]
    fn layer_opacity_maps_dwg_bytes_to_opacity() {
        assert_eq!(layer_opacity(acadrust::Transparency::OPAQUE), 1.0);
        assert_eq!(layer_opacity(acadrust::Transparency::TRANSPARENT), 0.0);
        // A degenerate ByLayer/ByBlock on a layer is opaque, not fabricated.
        assert_eq!(layer_opacity(acadrust::Transparency::BY_LAYER), 1.0);
    }

    // ---- §3.2/§7.1: colour and lineweight resolution ----

    #[test]
    fn color_resolution_prefers_byobject_then_layer_then_byblock() {
        // ByObject true colour wins over the layer.
        assert_eq!(
            resolve_entity_color(acadrust::Color::from_rgb(10, 20, 30), Some([1, 2, 3])),
            EntityColor::Explicit([10, 20, 30])
        );
        // An ACI index resolves through acadrust's canonical table, not a guess.
        assert_eq!(
            resolve_entity_color(acadrust::Color::Index(1), Some([1, 2, 3])),
            EntityColor::Explicit([255, 0, 0])
        );
        // ByLayer uses the layer's pre-resolved colour.
        assert_eq!(
            resolve_entity_color(acadrust::Color::ByLayer, Some([1, 2, 3])),
            EntityColor::Explicit([1, 2, 3])
        );
        // A materialised `None` keeps the layer's colour rather than black.
        assert_eq!(
            resolve_entity_color(acadrust::Color::None, Some([4, 5, 6])),
            EntityColor::Explicit([4, 5, 6])
        );
        // ByBlock stays symbolic so INSERT expansion can supply the value.
        assert_eq!(
            resolve_entity_color(acadrust::Color::ByBlock, Some([4, 5, 6])),
            EntityColor::ByBlock
        );
        // ByLayer with no reachable layer stays unresolved, not fabricated.
        assert_eq!(
            resolve_entity_color(acadrust::Color::ByLayer, None),
            EntityColor::ByLayer
        );
    }

    #[test]
    fn layer_rgb_resolves_index_and_rgb_and_falls_back_to_white() {
        assert_eq!(layer_rgb(acadrust::Color::from_rgb(9, 8, 7)), [9, 8, 7]);
        assert_eq!(layer_rgb(acadrust::Color::Index(5)), [0, 0, 255]);
        // A degenerate symbolic layer colour falls back to white.
        assert_eq!(layer_rgb(acadrust::Color::ByLayer), [255, 255, 255]);
    }

    #[test]
    fn lineweight_resolution_prefers_byobject_then_layer_then_byblock() {
        // A concrete weight is 1/100 mm; 35 -> 0.35 mm.
        assert_eq!(
            resolve_entity_lineweight(acadrust::LineWeight::Value(35), Some(0.5)),
            EntityLineWeight::Explicit(0.35)
        );
        // ByLayer uses the layer's pre-resolved weight.
        assert_eq!(
            resolve_entity_lineweight(acadrust::LineWeight::ByLayer, Some(0.5)),
            EntityLineWeight::Explicit(0.5)
        );
        // acadrust's Default keeps its explicit meaning.
        assert_eq!(
            resolve_entity_lineweight(acadrust::LineWeight::Default, Some(0.5)),
            EntityLineWeight::Default
        );
        // ByBlock stays symbolic.
        assert_eq!(
            resolve_entity_lineweight(acadrust::LineWeight::ByBlock, Some(0.5)),
            EntityLineWeight::ByBlock
        );
        // ByLayer with no reachable layer stays unresolved.
        assert_eq!(
            resolve_entity_lineweight(acadrust::LineWeight::ByLayer, None),
            EntityLineWeight::ByLayer
        );
    }

    #[test]
    fn lineweight_mm_only_reports_concrete_values() {
        assert_eq!(lineweight_mm(acadrust::LineWeight::Value(100)), Some(1.0));
        assert_eq!(lineweight_mm(acadrust::LineWeight::W0_25), Some(0.25));
        assert_eq!(lineweight_mm(acadrust::LineWeight::ByLayer), None);
        assert_eq!(lineweight_mm(acadrust::LineWeight::ByBlock), None);
        assert_eq!(lineweight_mm(acadrust::LineWeight::Default), None);
    }

    #[test]
    fn all_proxy_fragments_survive_as_a_compound() {
        let fragment = |id: u128| SemanticGeometry::Line {
            start: Point3::default(),
            end: Point3 {
                x: id as f64,
                y: 0.0,
                z: 0.0,
            },
        };
        match proxy_geometry_compound(vec![fragment(1), fragment(2), fragment(3)]) {
            SemanticGeometry::Compound(children) => {
                assert_eq!(children.len(), 3, "every proxy fragment must survive");
            }
            other => panic!("expected a compound, got {other:?}"),
        }
        // A single fragment stays itself: no needless wrapper.
        assert!(matches!(
            proxy_geometry_compound(vec![fragment(1)]),
            SemanticGeometry::Line { .. }
        ));
    }

    #[test]
    fn proxy_cache_is_only_used_for_entities_without_semantic_geometry() {
        let line = EntityType::Line(acadrust::entities::Line::new());
        assert!(
            !proxy_geometry_allowed(&line),
            "a LINE is drawn from semantics; its cache must not double-draw"
        );
        let unknown =
            EntityType::Unknown(acadrust::entities::UnknownEntity::new("ACAD_PROXY_ENTITY"));
        assert!(proxy_geometry_allowed(&unknown));
        let vendor = EntityType::Unknown(acadrust::entities::UnknownEntity::new("TCH_WALL"));
        assert!(proxy_geometry_allowed(&vendor));
    }

    // ---- ACIS neutral lift and kernel tessellation (F15 reachable subset) ----

    use cad_kernel_adapter::{
        BrepSurface, BrepTessellator, GeometryHandle, SolidExchange, SolidTessellator,
        TessellationBudget, TessellationOutcome, TessellationRequest, TessellationResult,
        TessellationTolerance,
    };

    fn tess_brep(exchange: SolidExchange) -> TessellationResult {
        let request = TessellationRequest {
            geometry: GeometryHandle::Resolved(ObjectId(1)),
            exchange,
            tolerance: TessellationTolerance::default(),
            budget: TessellationBudget::default(),
            stamp: TaskStamp::new(DocumentId(1), 0),
        };
        BrepTessellator.tessellate(&request, &|| false).unwrap()
    }

    #[test]
    fn acis_box_lifts_to_six_planar_faces_and_tessellates_closed() {
        use acadrust::entities::acis::primitives::build_box;
        let doc = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0);
        let brep = sat_to_brep(&doc);
        assert_eq!(brep.face_count(), 6, "a box has six faces");
        assert!(brep
            .shells
            .iter()
            .flat_map(|s| &s.faces)
            .all(|f| matches!(f.surface, BrepSurface::Plane { .. })));

        let result = tess_brep(SolidExchange::Brep(brep));
        match result.outcome {
            TessellationOutcome::Success { geometry, .. } => {
                assert_eq!(geometry.triangle_count(), 12);
                let area = cad_kernel_adapter::brep::mesh_area(&geometry.mesh);
                assert!((area - 24.0).abs() < 1e-9, "box area {area}");
            }
            other => panic!("box must tessellate as Success, got {other:?}"),
        }
    }

    #[test]
    fn solid3d_entity_round_trips_through_sat_text() {
        use acadrust::entities::acis::primitives::build_box;
        use acadrust::entities::Solid3D;
        let sat = build_box([1.0, 2.0, 3.0], 2.0, 4.0, 6.0).to_sat_string();
        let solid = Solid3D::from_sat(&sat);
        let exchange = solid_exchange_from_entity(&EntityType::Solid3D(solid))
            .expect("3DSOLID must expose an exchange");
        let SolidExchange::Brep(brep) = exchange else {
            panic!("a valid SAT payload must lift to a neutral B-rep");
        };
        assert_eq!(brep.face_count(), 6);
        assert!(matches!(
            tess_brep(SolidExchange::Brep(brep)).outcome,
            TessellationOutcome::Success { .. }
        ));
    }

    #[test]
    fn sab_payload_lifts_through_the_binary_reader() {
        use acadrust::entities::acis::primitives::build_box;
        use acadrust::entities::acis::SabWriter;
        use acadrust::entities::Solid3D;
        let doc = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0);
        let sab = SabWriter::write(&doc);
        let brep = sab_to_brep(&sab).expect("SAB must decode");
        assert_eq!(brep.face_count(), 6);
        let solid = Solid3D::from_sab(sab);
        let exchange = solid_exchange_from_entity(&EntityType::Solid3D(solid)).unwrap();
        assert!(matches!(exchange, SolidExchange::Brep(_)));
    }

    #[test]
    fn cylinder_caps_and_side_tessellate_closed() {
        use acadrust::entities::acis::primitives::build_cylinder;
        let doc = build_cylinder([0.0, 0.0, 0.0], 1.0, 3.0);
        let brep = sat_to_brep(&doc);
        assert_eq!(brep.face_count(), 3, "two caps plus the side");
        assert!(brep.shells[0]
            .faces
            .iter()
            .any(|f| matches!(f.surface, BrepSurface::Cylinder { .. })));
        match tess_brep(SolidExchange::Brep(brep)).outcome {
            TessellationOutcome::Success { geometry, .. } => {
                assert!(geometry.triangle_count() > 0);
                // The curved side makes it an approximation with a bound.
                assert!(matches!(
                    geometry.precision,
                    Precision::Approximate {
                        error_bound: Some(_)
                    }
                ));
            }
            other => panic!("cylinder must be a closed Success, got {other:?}"),
        }
    }

    #[test]
    fn sphere_lifts_to_one_loopless_face_and_tessellates() {
        use acadrust::entities::acis::primitives::build_sphere;
        let doc = build_sphere([0.0, 0.0, 0.0], 2.0);
        let brep = sat_to_brep(&doc);
        assert_eq!(brep.face_count(), 1);
        match tess_brep(SolidExchange::Brep(brep)).outcome {
            TessellationOutcome::Success { geometry, .. } => {
                assert!(geometry.triangle_count() >= 8);
            }
            other => panic!("sphere must be a closed Success, got {other:?}"),
        }
    }

    #[test]
    fn cone_reports_the_unsupported_side_face_not_a_fake_mesh() {
        use acadrust::entities::acis::primitives::build_cone;
        let doc = build_cone([0.0, 0.0, 0.0], 1.0, 2.0);
        let brep = sat_to_brep(&doc);
        let result = tess_brep(SolidExchange::Brep(brep));
        match result.outcome {
            TessellationOutcome::Partial {
                geometry,
                degradation,
                ..
            } => {
                assert!(geometry.triangle_count() > 0, "the base disc still draws");
                assert!(!degradation.missing_faces.is_empty());
            }
            // A future build that learns cones may return Success; a fabricated
            // mesh is the only wrong answer, and that cannot be represented.
            other => panic!("cone must be Partial with a missing face, got {other:?}"),
        }
    }

    #[test]
    fn fixture_cube_parses_to_a_closed_six_face_solid() {
        let sat = include_str!("../../../fixtures/acis/cube.sat");
        let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
        let brep = sat_to_brep(&doc);
        assert_eq!(brep.face_count(), 6);
        match tess_brep(SolidExchange::Brep(brep)).outcome {
            TessellationOutcome::Success { geometry, .. } => {
                assert_eq!(geometry.triangle_count(), 12);
                let area = cad_kernel_adapter::brep::mesh_area(&geometry.mesh);
                assert!((area - 24.0).abs() < 1e-9, "area {area}");
            }
            other => panic!("cube fixture must be a closed Success, got {other:?}"),
        }
    }

    #[test]
    fn fixture_box_with_square_hole_tessellates_closed_with_holes() {
        let sat = include_str!("../../../fixtures/acis/box-with-square-hole.sat");
        let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
        let brep = sat_to_brep(&doc);
        assert_eq!(brep.face_count(), 10, "two annuli plus eight walls");
        assert!(
            brep.shells
                .iter()
                .flat_map(|s| &s.faces)
                .any(|f| f.loops.len() == 2),
            "the annulus faces must carry an inner loop"
        );
        match tess_brep(SolidExchange::Brep(brep)).outcome {
            TessellationOutcome::Success { geometry, .. } => {
                assert!(geometry.triangle_count() > 0);
                // 2*(10*10) + 4*(10*4) - 2*(4*4) + 16*4 = 392.
                let area = cad_kernel_adapter::brep::mesh_area(&geometry.mesh);
                assert!((area - 392.0).abs() < 1e-9, "area {area}");
            }
            other => panic!("holed box must be a closed Success, got {other:?}"),
        }
    }

    #[test]
    fn non_solid_entities_have_no_acis_exchange() {
        let line = EntityType::Line(acadrust::entities::Line::new());
        assert!(solid_exchange_from_entity(&line).is_none());
    }

    #[test]
    fn region_body_and_surface_entities_share_the_acis_lift() {
        use acadrust::entities::acis::primitives::build_box;
        use acadrust::entities::{AcisData, Body, Region, Surface};
        let sat = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0).to_sat_string();
        let surface = Surface {
            acis_data: AcisData::from_sat(&sat),
            ..Surface::default()
        };
        let entities = [
            EntityType::Region(Region::from_sat(&sat)),
            EntityType::Body(Body::from_sat(&sat)),
            EntityType::Surface(surface),
        ];
        for entity in entities {
            match solid_exchange_from_entity(&entity) {
                Some(SolidExchange::Brep(brep)) => assert_eq!(brep.face_count(), 6),
                other => panic!("{entity:?} must lift to a B-rep, got {other:?}"),
            }
        }
    }

    #[test]
    fn empty_acis_data_stays_raw_and_empty() {
        let empty = acadrust::entities::AcisData::new();
        let exchange = acis_exchange(&empty);
        assert!(exchange.is_empty());
        let result = tess_brep(exchange);
        assert!(matches!(result.outcome, TessellationOutcome::Failed { .. }));
    }

    #[test]
    fn opaque_payload_retains_the_raw_acis_bytes() {
        let sat = "700 0 1 0\n@8 acadrust @8 ACIS 7.0 @24 Thu Jan 01 00:00:00 2023\n1e-06 1e-06\n-1 body $-1 $-1 $-1 $-1 #\n";
        let acis = acadrust::entities::AcisData::from_sat(sat);
        let (version, payload) = acis_raw_payload(&acis);
        assert_eq!(version, 1);
        assert!(!payload.is_empty());
    }
}
