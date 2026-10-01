//! Sole acadrust integration boundary. No third-party entity escapes.
//!
//! Spec v2.0 §7.1: read bytes → acadrust parse → walk model/paper space and
//! block definitions → proxy supplement → normalise into the database. The
//! library is never modified and nothing here assumes internal hooks.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use acadrust::entities::EntityCommon;
use acadrust::{DwgReadOptions, DwgReader, EntityType, ReadStats};
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer, Layout,
    PaperViewport, Style,
};
use cad_domain::*;
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
    style_ids: HashMap<String, StyleId>,
    block_ids: HashMap<String, BlockId>,
    layout_ids: HashMap<String, LayoutId>,
    next_entity: u128,
    next_object: u128,
    entity_total: usize,
    dropped: usize,
    proxy: ProxyPlayer,
    model_layout: LayoutId,
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
            style_ids: HashMap::new(),
            block_ids: HashMap::new(),
            layout_ids: HashMap::new(),
            next_entity: 1,
            next_object: 1,
            entity_total: 0,
            dropped: 0,
            proxy,
            model_layout: LayoutId(0),
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
        let completeness = if diagnostics.iter().any(|d| d.code.starts_with("import.")) {
            Completeness::Partial(diagnostics.iter().map(|d| d.message.clone()).collect())
        } else {
            Completeness::Complete
        };
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
            if !block.name.starts_with("*Paper_Space") {
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
            if block.name == "*Model_Space" || block.name.starts_with("*Paper_Space") {
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
        // Model space.
        let model_space = self.model_space_id();
        for entity in self.acad.model_space_entities() {
            self.push_entity(entity, SpaceId::Model, model_space);
        }
        // Block definitions are stored but not drawn as top-level geometry.
        for block in self.acad.block_records.iter() {
            if block.name == "*Model_Space" || block.name.starts_with("*Paper_Space") {
                continue;
            }
            let Some(id) = self.block_ids.get(&block.name).copied() else {
                continue;
            };
            let mut entity_ids = Vec::new();
            for entity in self.acad.entities_in_block(&block.name) {
                if let Some(e) = self.push_entity(entity, SpaceId::Model, self.model_layout) {
                    entity_ids.push(e);
                }
            }
            self.builder.insert_block(BlockDefinition {
                id,
                entities: entity_ids,
            })?;
        }
        // Paper space.
        for (name, layout) in self.layout_ids.clone() {
            for entity in self.acad.entities_in_block(&name) {
                self.push_entity(entity, SpaceId::Paper(layout), layout);
            }
        }
        Ok(())
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

        let entity_id = EntityId(self.next_entity);
        let object_id = ObjectId(self.next_object);
        self.next_entity += 1;
        self.next_object += 1;
        self.entity_total += 1;

        self.note_capability(&class_name, &geometry, &completeness);

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
            EntityType::Ellipse(e) => (
                SemanticGeometry::Ellipse {
                    center: p3(e.center),
                    major_axis: p3(e.major_axis),
                    ratio: e.minor_axis_ratio,
                    start: e.start_parameter,
                    sweep: normalize_sweep(e.end_parameter - e.start_parameter),
                },
                Completeness::Complete,
            ),
            EntityType::Point(p) => (
                SemanticGeometry::Point(p3(p.location)),
                Completeness::Complete,
            ),
            EntityType::LwPolyline(pl) => {
                let points: Vec<Point3> = pl
                    .vertices
                    .iter()
                    .map(|v| Point3 {
                        x: v.location.x,
                        y: v.location.y,
                        z: pl.elevation,
                    })
                    .collect();
                let bulges: Vec<f64> = pl.vertices.iter().map(|v| v.bulge).collect();
                (
                    SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed: pl.is_closed,
                    },
                    Completeness::Complete,
                )
            }
            EntityType::Polyline2D(pl) => {
                let points: Vec<Point3> = pl
                    .vertices
                    .iter()
                    .map(|v| Point3 {
                        x: v.location.x,
                        y: v.location.y,
                        z: pl.elevation,
                    })
                    .collect();
                let bulges: Vec<f64> = pl.vertices.iter().map(|v| v.bulge).collect();
                let closed = pl.flags.bits() & 1 != 0;
                (
                    SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed,
                    },
                    Completeness::Complete,
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
            EntityType::Text(t) => (
                SemanticGeometry::Text {
                    text: t.value.clone(),
                    position: p3(t.insertion_point),
                    style: self.style_id(&t.style),
                    height: t.height,
                    rotation: t.rotation,
                },
                Completeness::Complete,
            ),
            EntityType::MText(t) => (
                SemanticGeometry::Text {
                    text: t.value.clone(),
                    position: p3(t.insertion_point),
                    style: self.style_id(&t.style),
                    height: t.height,
                    rotation: t.rotation,
                },
                Completeness::Complete,
            ),
            EntityType::Spline(s) => (
                SemanticGeometry::Spline {
                    degree: s.degree.max(1) as u32,
                    knots: s.knots.clone(),
                    control_points: s.control_points.iter().map(|p| p3(*p)).collect(),
                    weights: s.weights.clone(),
                },
                if s.weights.is_empty() {
                    Completeness::Complete
                } else {
                    Completeness::Partial(vec!["weighted spline".into()])
                },
            ),
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
                    (out.geometry.into_iter().next().unwrap(), completeness)
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

    fn block_id(&self, name: &str) -> BlockId {
        self.block_ids
            .get(name)
            .copied()
            .unwrap_or(BlockId(u128::MAX))
    }

    fn note_capability(
        &mut self,
        class_name: &str,
        geometry: &SemanticGeometry,
        completeness: &Completeness,
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
                render: semantic,
                pick: semantic,
                measure,
            });
        // Keep the weakest status observed for the class.
        entry.semantic = weaker(entry.semantic, semantic);
        entry.render = weaker(entry.render, semantic);
        entry.pick = weaker(entry.pick, semantic);
        entry.measure = weaker(entry.measure, measure);
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
}
