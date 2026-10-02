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
    BlockDefinition, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder,
    EntityRenderAttributes, EntityTransparency, Layer, Layout, PaperViewport, Style,
};
use cad_domain::*;
use cad_geometry::{arbitrary_axis, tessellate_bspline, PatternLine, TessellationParams};
use cad_proxy::{DecodeLimits, ProxyPlayer, ProxySource};

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
    style_ids: HashMap<String, StyleId>,
    /// Lower-cased style name -> primary font file name.
    style_fonts: HashMap<String, String>,
    block_ids: HashMap<String, BlockId>,
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
#[derive(Debug, Clone, Copy)]
struct BlockMember {
    render: SupportStatus,
    insert_block: Option<BlockId>,
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
            style_ids: HashMap::new(),
            style_fonts: HashMap::new(),
            block_ids: HashMap::new(),
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
                    let center = p3(v.center);
                    let half = p3(acadrust::Vector3::new(v.width / 2.0, v.height / 2.0, 0.0));
                    let scale = if v.height.abs() > 1e-12 {
                        v.view_height / v.height
                    } else {
                        1.0
                    };
                    let complex = !v.clip_boundary_handle.null_or_value_zero();
                    if complex {
                        self.diagnostics.push(Diagnostic {
                            object: None,
                            code: "import.viewport_complex_clip".into(),
                            message: format!(
                                "layout {}: non-rectangular viewport clip is not applied",
                                block.name
                            ),
                        });
                    }
                    viewports.push(PaperViewport {
                        clip: vec![
                            sub(center, half),
                            Point3 {
                                x: center.x + half.x,
                                y: center.y - half.y,
                                z: 0.0,
                            },
                            p3(v.view_center),
                        ],
                        model_to_paper: Transform3::scale(scale),
                        completeness: if complex {
                            Completeness::Partial(vec!["complex viewport clip".into()])
                        } else {
                            Completeness::Complete
                        },
                    });
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
                    let child = match member.insert_block {
                        Some(block) => self
                            .block_status
                            .get(&block)
                            .copied()
                            .unwrap_or(SupportStatus::Unverified),
                        None => member.render,
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
        // (audit B20). An INSERT inherits the resolved status of its block.
        let (mut render, mut pick) = display_support(&geometry);
        let insert_block = match &geometry {
            SemanticGeometry::Insert { block, .. } => Some(*block),
            _ => None,
        };
        if let Some(block) = insert_block {
            if let Some(status) = self.block_status.get(&block) {
                render = *status;
                pick = *status;
            }
        }
        if let SpaceId::Block(block) = &space {
            self.block_members
                .entry(*block)
                .or_default()
                .push(BlockMember {
                    render,
                    insert_block,
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

        // Resolve the entity's effective display opacity and geometry source so
        // the representation/scene layers can carry real transparency instead
        // of always drawing opaque (audit F14). ByObject wins over ByLayer;
        // ByBlock is kept symbolic for INSERT expansion to resolve.
        let layer_alpha = self.layer_transparency.get(&layer).copied().unwrap_or(1.0);
        let attributes = EntityRenderAttributes {
            transparency: resolve_entity_transparency(common.transparency, layer_alpha),
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
                    center: p3(c.center),
                    normal: p3(c.normal),
                    radius: c.radius,
                },
                Completeness::Complete,
            ),
            EntityType::Arc(a) => (
                SemanticGeometry::Arc {
                    center: p3(a.center),
                    normal: p3(a.normal),
                    radius: a.radius,
                    start: a.start_angle,
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
            EntityType::Solid(s) => {
                let m = quad_mesh([
                    p3(s.first_corner),
                    p3(s.second_corner),
                    p3(s.third_corner),
                    p3(s.fourth_corner),
                ]);
                (SemanticGeometry::Mesh(m), Completeness::Complete)
            }
            EntityType::Face3D(f) => {
                let m = quad_mesh([
                    p3(f.first_corner),
                    p3(f.second_corner),
                    p3(f.third_corner),
                    p3(f.fourth_corner),
                ]);
                (SemanticGeometry::Mesh(m), Completeness::Complete)
            }
            EntityType::Insert(i) => (
                SemanticGeometry::Insert {
                    block: self.block_id(&i.block_name),
                    transform: insert_transform(i),
                },
                Completeness::Complete,
            ),
            EntityType::Unknown(u) => {
                self.proxy_geometry(&u.dxf_name, common, u.raw_dwg_data.as_deref())
            }
            EntityType::Extended(x) if x.class_name() == "ACAD_PROXY_ENTITY" => {
                self.proxy_geometry(x.class_name(), common, None)
            }
            EntityType::Hatch(h) => self.hatch_geometry(h),
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
            EntityType::Solid3D(_)
            | EntityType::Region(_)
            | EntityType::Body(_)
            | EntityType::Surface(_) => (
                SemanticGeometry::Opaque {
                    type_key: entity_class_name(entity),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Unverified,
            ),
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

    /// Convert a HATCH into a compound of boundary loops plus a solid fill or
    /// pattern lines. Boundaries are always emitted; fills that cannot be
    /// generated are reported as Partial rather than faked.
    fn hatch_geometry(&self, h: &Hatch) -> (SemanticGeometry, Completeness) {
        let normal = p3(h.normal);
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
            if loops.len() == 1 {
                // Simplify tessellated curves before filling (ear clipping is
                // cubic in the worst case).
                let loop2 = &loops[0];
                let (min, max) = loop2.iter().fold(
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
                let mut closed = loop2.clone();
                closed.push(closed[0]);
                let mut simplified = cad_geometry::simplify(&closed, tolerance);
                if simplified.len() > 1 && simplified.last() == simplified.first() {
                    simplified.pop();
                }
                if simplified.len() > cad_geometry::MAX_FILL_POINTS {
                    completeness = Completeness::Partial(vec![
                        "solid hatch boundary is too complex to fill".into(),
                    ]);
                } else {
                    let triangles = cad_geometry::triangulate(&simplified);
                    if triangles.is_empty() {
                        completeness = Completeness::Partial(vec![
                            "solid hatch boundary could not be triangulated".into(),
                        ]);
                    } else {
                        let vertices: Vec<Point3> =
                            simplified.iter().map(|p| to_world(*p)).collect();
                        let normals = vec![un; vertices.len()];
                        children.push(SemanticGeometry::Mesh(Mesh {
                            vertices,
                            triangles,
                            normals,
                            face_sources: Vec::new(),
                        }));
                    }
                }
            } else {
                completeness = Completeness::Partial(vec![
                    "solid hatch islands are outlined but not filled".into(),
                ]);
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

/// Convert an ELLIPSE, accounting for its extrusion.
///
/// The domain `Ellipse` carries only a major axis and ratio, so its plane is
/// implicitly parallel to world XY. An ellipse on a tilted OCS plane cannot be
/// represented exactly and is reported Partial instead of being flattened.
fn ellipse_semantics(e: &acadrust::entities::Ellipse) -> (SemanticGeometry, Completeness) {
    let normal = p3(e.normal);
    let geometry = SemanticGeometry::Ellipse {
        center: p3(e.center),
        major_axis: p3(e.major_axis),
        ratio: e.minor_axis_ratio,
        start: e.start_parameter,
        sweep: normalize_sweep(e.end_parameter - e.start_parameter),
    };
    let completeness = if is_world_z(normal) {
        Completeness::Complete
    } else {
        Completeness::Partial(vec![
            "ellipse on a tilted extrusion cannot be encoded exactly; plane approximated by world XY"
                .into(),
        ])
    };
    (geometry, completeness)
}

/// Convert a SPLINE, preserving the source's own knots and weights.
fn spline_semantics(s: &acadrust::entities::Spline) -> (SemanticGeometry, Completeness) {
    let geometry = SemanticGeometry::Spline {
        degree: s.degree.max(1) as u32,
        knots: s.knots.clone(),
        control_points: s.control_points.iter().map(|p| p3(*p)).collect(),
        weights: s.weights.clone(),
    };
    let completeness = if s.weights.is_empty() {
        Completeness::Complete
    } else {
        Completeness::Partial(vec!["weighted spline".into()])
    };
    (geometry, completeness)
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
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

fn insert_transform(i: &acadrust::entities::Insert) -> Transform3 {
    let (s, c) = i.rotation.sin_cos();
    let sx = i.x_scale();
    let sy = i.y_scale();
    let sz = i.z_scale();
    let mut m = [[0.0f64; 4]; 4];
    m[0][0] = c * sx;
    m[0][1] = -s * sy;
    m[1][0] = s * sx;
    m[1][1] = c * sy;
    m[2][2] = sz;
    m[3][3] = 1.0;
    m[0][3] = i.insert_point.x;
    m[1][3] = i.insert_point.y;
    m[2][3] = i.insert_point.z;
    Transform3 { matrix: m }
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
    fn tilted_ellipse_is_partial_but_world_z_is_complete() {
        let mut e = acadrust::entities::Ellipse::new();
        e.normal = acadrust::types::Vector3::new(1.0, 0.0, 0.0);
        let (geom, completeness) = ellipse_semantics(&e);
        assert!(matches!(geom, SemanticGeometry::Ellipse { .. }));
        assert!(
            matches!(completeness, Completeness::Partial(_)),
            "tilted ellipse must not be silently flattened: {completeness:?}"
        );
        e.normal = acadrust::types::Vector3::UNIT_Z;
        assert_eq!(ellipse_semantics(&e).1, Completeness::Complete);
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
        // Rational splines are still not tessellated exactly by every backend.
        assert!(matches!(completeness, Completeness::Partial(_)));
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
}
