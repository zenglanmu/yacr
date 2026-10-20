//! Unit tests.

use super::*;
use cad_db::{
    BlockDefinition, DbObject, DrawingDatabaseBuilder, EntityLineType, EntityRenderAttributes,
    EntityTransparency, Layer,
};
use cad_kernel_adapter::{
    BrepCurve, BrepData, BrepFace, BrepLoop, BrepShell, BrepSurface, BrepTessellator,
    GeometryHandle, SolidExchange, SolidTessellator, TessellationBudget, TessellationRequest,
    TessellationTolerance,
};
use cad_resources::{image_resource_key, DecodedImage, ResourceLimits};

mod annotative;
mod attributes;
mod basic;
mod blocks;
mod bulge_arcs;
mod conic;
mod dynamic_block;
mod image;
mod kernel_mesh;
mod linetype;
mod mask;

fn entity(id: u128, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: Some("1A".into()),
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: 0,
    }
}

fn context() -> RepresentationContext {
    RepresentationContext::new(
        DocumentId(1),
        TolerancePolicy::default(),
        TaskStamp::new(DocumentId(1), 0),
    )
}

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line_entity(id: u128, space: SpaceId, a: Point3, b: Point3) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Line { start: a, end: b },
        draw_order: id as i64,
    }
}

fn insert_entity(id: u128, space: SpaceId, block: u128, dx: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbBlockReference".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Insert {
            block: BlockId(block),
            transform: Transform3::translation(p(dx, 0.0)),
        },
        draw_order: id as i64,
    }
}

fn empty_db() -> DrawingDatabaseBuilder {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b
}

fn attrs(transparency: EntityTransparency, source: GeometrySource) -> EntityRenderAttributes {
    EntityRenderAttributes {
        transparency,
        color: cad_db::EntityColor::ByLayer,
        lineweight: cad_db::EntityLineWeight::ByLayer,
        linetype: cad_db::EntityLineType::default(),
        geometry_source: source,
        ..Default::default()
    }
}

fn attrs_styled(
    color: cad_db::EntityColor,
    lineweight: cad_db::EntityLineWeight,
) -> EntityRenderAttributes {
    EntityRenderAttributes {
        transparency: EntityTransparency::Explicit(1.0),
        color,
        lineweight,
        linetype: cad_db::EntityLineType::default(),
        geometry_source: GeometrySource::Analytic,
        ..Default::default()
    }
}

// ---- Kernel-mesh display path (F15 seam) ----

fn selection() -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(42),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn square_brep(with_unsupported: bool) -> BrepData {
    let loop_ = BrepLoop {
        edges: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
            .iter()
            .map(|q| BrepCurve::Line {
                start: Point3 {
                    x: q[0],
                    y: q[1],
                    z: 0.0,
                },
                end: Point3 {
                    x: q[0],
                    y: q[1],
                    z: 0.0,
                },
            })
            .collect(),
    };
    let mut faces = vec![BrepFace {
        id: 0,
        surface: BrepSurface::Plane {
            origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            u_dir: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
        reversed: false,
        loops: vec![loop_],
    }];
    if with_unsupported {
        faces.push(BrepFace {
            id: 9,
            surface: BrepSurface::Unsupported {
                type_key: "nurbs-surface".into(),
            },
            reversed: false,
            loops: Vec::new(),
        });
    }
    BrepData {
        shells: vec![BrepShell { id: 0, faces }],
        placement: None,
    }
}

fn tessellate(exchange: SolidExchange) -> cad_kernel_adapter::TessellationResult {
    let request = TessellationRequest {
        geometry: GeometryHandle::Resolved(ObjectId(7)),
        exchange,
        tolerance: TessellationTolerance::default(),
        budget: TessellationBudget::default(),
        stamp: TaskStamp::new(DocumentId(1), 0),
    };
    BrepTessellator.tessellate(&request, &|| false).unwrap()
}

// ---- §3.2/§7.1: linetype dash subdivision ----

fn attrs_linetype(linetype: EntityLineType) -> EntityRenderAttributes {
    EntityRenderAttributes {
        transparency: EntityTransparency::Explicit(1.0),
        color: cad_db::EntityColor::Explicit([255, 255, 255]),
        lineweight: cad_db::EntityLineWeight::ByLayer,
        linetype,
        geometry_source: GeometrySource::Analytic,
        ..Default::default()
    }
}

fn line_db(line: DbEntity, linetype: EntityLineType) -> cad_db::DrawingDatabase {
    let mut b = empty_db();
    let id = line.id;
    b.insert_entity(line).unwrap();
    b.set_entity_render_attributes(id, attrs_linetype(linetype))
        .unwrap();
    b.finish().unwrap()
}

fn line_primitives(rep: &DisplayRepresentation) -> Vec<Vec<Point3>> {
    rep.fragments
        .iter()
        .filter_map(|f| match &f.primitive {
            DisplayPrimitive::Lines(pts) => Some(pts.to_vec()),
            _ => None,
        })
        .collect()
}

// ---- Dynamic-block visibility (spec §3.2) ----

/// A model INSERT that references a two-state dynamic block.
///
/// Block 5 owns three lines (entities 50, 51, 52) and defines:
///   state "A" -> 50 + 51
///   state "B" -> 51 + 52
fn dynamic_block_db(active: &str) -> DrawingDatabase {
    use cad_db::{DynamicBlockState, DynamicBlockVisibility};
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(5),
        entities: vec![EntityId(50), EntityId(51), EntityId(52)],
        dynamic_visibility: None,
    })
    .unwrap();
    for (i, id) in [50u128, 51, 52].into_iter().enumerate() {
        b.insert_entity(line_entity(
            id,
            SpaceId::Block(BlockId(5)),
            p(i as f64, 0.0),
            p(i as f64, 1.0),
        ))
        .unwrap();
    }
    b.set_block_dynamic_visibility(
        BlockId(5),
        DynamicBlockVisibility {
            member_entities: vec![EntityId(50), EntityId(51), EntityId(52)],
            states: vec![
                DynamicBlockState {
                    name: "A".into(),
                    entities: vec![EntityId(50), EntityId(51)],
                },
                DynamicBlockState {
                    name: "B".into(),
                    entities: vec![EntityId(51), EntityId(52)],
                },
            ],
            active_state: Some(active.into()),
        },
    )
    .unwrap();
    b.insert_entity(insert_entity(1, SpaceId::Model, 5, 0.0))
        .unwrap();
    b.finish().unwrap()
}

fn emitted_member_xs(db: &DrawingDatabase) -> Vec<f64> {
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    let mut xs: Vec<f64> = rep
        .fragments
        .iter()
        .map(|f| match &f.primitive {
            DisplayPrimitive::Lines(pts) => pts[0].x,
            _ => panic!("expected lines"),
        })
        .collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    xs
}

// ---------------------------------------------------------------------------
// Annotative scaling
// ---------------------------------------------------------------------------

fn text_entity(id: u128, height: f64) -> DbEntity {
    let mut e = entity(
        id,
        SemanticGeometry::Text {
            text: "A".into(),
            position: Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0,
            },
            style: StyleId(0),
            height,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        },
    );
    e.object.type_key = "AcDbText".into();
    e
}

fn annotative_attributes(annotative: bool) -> cad_db::EntityRenderAttributes {
    cad_db::EntityRenderAttributes {
        annotative: cad_db::AnnotativeAttributes {
            annotative,
            overrides: Vec::new(),
        },
        ..Default::default()
    }
}

/// Without a font the text branch emits `DisplayPrimitive::Text` carrying the
/// effective height, so annotative scaling is observable without shaping.
fn emitted_text_height(r: &DisplayRepresentation) -> f64 {
    match &r.fragments[0].primitive {
        DisplayPrimitive::Text { height, .. } => *height,
        _ => panic!("expected a Text primitive"),
    }
}

fn white_pixel() -> DecodedImage {
    DecodedImage {
        width: 1,
        height: 1,
        rgba: Arc::from(vec![255u8, 255, 255, 255]),
    }
}

/// A context whose decoded-image cache holds `entries`, keyed by the
/// path-preserving [`image_resource_key`] (falling back to the bare-name
/// [`ResourceKey::sanitize`] for a reference that is not a usable image key).
fn image_context(entries: &[(&str, DecodedImage)]) -> RepresentationContext {
    let mut cache = DecodedImageCache::new();
    for (name, image) in entries {
        let key = image_resource_key(name).unwrap_or_else(|| ResourceKey::sanitize(name));
        cache
            .insert(key, image.clone(), &ResourceLimits::default())
            .unwrap();
    }
    context().with_images(Arc::new(cache))
}

/// An `Image` placed at `(1, 2)` with edge vectors `+X`/`+Y` and `2x3` pixels.
fn image_entity(id: u128, file: Option<&str>, visible: bool) -> DbEntity {
    entity(
        id,
        SemanticGeometry::Image {
            origin: p(1.0, 2.0),
            u: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            pixels: [2.0, 3.0],
            file: file.map(str::to_string),
            clip: None,
            visible,
        },
    )
}

/// An `Image` placed at `(1, 2)` with edge vectors `+X`/`+Y`, `2x3` pixels and
/// the given pixel-space `clip`.
fn image_entity_with_clip(id: u128, file: Option<&str>, clip: Option<ImageClip>) -> DbEntity {
    entity(
        id,
        SemanticGeometry::Image {
            origin: p(1.0, 2.0),
            u: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            pixels: [2.0, 3.0],
            file: file.map(str::to_string),
            clip,
            visible: true,
        },
    )
}

fn assert_point_eq(actual: Point3, expected: Point3) {
    assert!(
        (actual.x - expected.x).abs() < 1e-9
            && (actual.y - expected.y).abs() < 1e-9
            && (actual.z - expected.z).abs() < 1e-9,
        "{actual:?} != {expected:?}"
    );
}

// ---- Conic plumbing (SDF slice A: behaviour-neutral) --------------------

/// The pre-conic chord path, as a baseline to compare the conic expansion to.
fn chord_baseline(geometry: &SemanticGeometry, ctx: &RepresentationContext) -> Vec<Point3> {
    cad_geometry::tessellate_geometry(geometry, crate::provider::chord_tessellation_params(ctx))
}

/// The world-space points a built representation emits after conic expansion.
fn emitted_points(rep: &DisplayRepresentation) -> Vec<Point3> {
    let mut out = Vec::new();
    for fragment in &rep.fragments {
        match &fragment.primitive {
            DisplayPrimitive::Lines(pts) | DisplayPrimitive::LineSegments(pts) => {
                out.extend(pts.iter().copied());
            }
            _ => {}
        }
    }
    out
}

/// The analytic conics a built representation emits (slice B SDF path).
fn emitted_conics(rep: &DisplayRepresentation) -> Vec<ConicSegment> {
    rep.fragments
        .iter()
        .filter_map(|fragment| match &fragment.primitive {
            DisplayPrimitive::Conic(conic) => Some(**conic),
            _ => None,
        })
        .collect()
}

fn assert_points_close(actual: &[Point3], expected: &[Point3], tolerance: f64) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "point count differs: {} vs {}",
        actual.len(),
        expected.len()
    );
    for (index, (a, b)) in actual.iter().zip(expected).enumerate() {
        let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt();
        assert!(
            d <= tolerance,
            "point {index} differs by {d} (>{tolerance}): {a:?} vs {b:?}"
        );
    }
}

fn geometry_db(entity: DbEntity) -> cad_db::DrawingDatabase {
    let mut b = empty_db();
    b.insert_entity(entity).unwrap();
    b.finish().unwrap()
}

fn curve_entity(id: u128, type_key: &str, geometry: SemanticGeometry) -> DbEntity {
    let mut e = entity(id, geometry);
    e.object.type_key = type_key.into();
    e
}

fn circle_geometry(center: Point3, radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center,
        normal: p3(0.0, 0.0, 1.0),
        radius,
    }
}

fn arc_geometry(center: Point3, radius: f64, start: f64, sweep: f64) -> SemanticGeometry {
    SemanticGeometry::Arc {
        center,
        normal: p3(0.0, 0.0, 1.0),
        radius,
        start,
        sweep,
    }
}

fn insert_with_transform(id: u128, space: SpaceId, block: u128, transform: Transform3) -> DbEntity {
    let mut e = curve_entity(
        id,
        "AcDbBlockReference",
        SemanticGeometry::Insert {
            block: BlockId(block),
            transform,
        },
    );
    e.space = space;
    e
}
