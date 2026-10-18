use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};

use crate::{
    DisplayFragment, DisplayPrimitive, DisplayRepresentation, ImageVertex, ProviderRegistry,
    RepresentationContext,
};

// ---- paper build integration for the new clip paths --------------------

#[test]
fn paper_build_clips_a_model_mesh_per_triangle_and_stays_complete() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbFace".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mesh(triangle_mesh([
            p(0.0, 0.0),
            p(20000.0, 0.0),
            p(0.0, 20000.0),
        ])),
        draw_order: 0,
    })
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: vec![simple_viewport()],
    })
    .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    // Exact per-triangle clipping is not a degradation.
    assert_eq!(rep.completeness, Completeness::Complete);
    let mesh = rep
        .fragments
        .iter()
        .find_map(|f| match &f.primitive {
            DisplayPrimitive::Mesh(mesh) | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                Some(mesh.clone())
            }
            _ => None,
        })
        .expect("clipped mesh fragment");
    assert!(!mesh.triangles.is_empty());
    for v in &mesh.vertices {
        // Paper window is x in [0,100], y in [0,50].
        assert!(v.x >= -1e-9 && v.x <= 100.0 + 1e-9);
        assert!(v.y >= -1e-9 && v.y <= 50.0 + 1e-9);
    }
    assert!(!rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.viewport_clip_partial"));
}

#[test]
fn paper_build_keeps_an_unclippable_mesh_partial_and_reported() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // A triangle that references a missing vertex cannot be clipped exactly.
    let broken = Mesh {
        vertices: vec![p(0.0, 0.0), p(1.0, 0.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbFace".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mesh(broken),
        draw_order: 0,
    })
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: vec![simple_viewport()],
    })
    .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert!(rep.diagnostics.iter().any(|d| {
        d.code == "representation.viewport_clip_partial"
            && d.message.contains(clip_reason::MESH_UNCLIPPABLE)
    }));
    // The unclipped geometry is kept, never dropped silently.
    assert!(rep
        .fragments
        .iter()
        .any(|f| matches!(f.primitive, DisplayPrimitive::Mesh(_))));
}

#[test]
fn paper_build_keeps_unshaped_text_partial_with_a_precise_reason() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbText".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: "A".into(),
            position: p(10.0, 20.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        },
        draw_order: 0,
    })
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: vec![simple_viewport()],
    })
    .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert!(rep.diagnostics.iter().any(|d| {
        d.code == "representation.viewport_clip_partial"
            && d.message.contains(clip_reason::TEXT_FONT_DEPENDENT)
    }));
    // The unshaped placeholder is kept, never dropped or faked as clipped.
    assert!(rep
        .fragments
        .iter()
        .any(|f| matches!(f.primitive, DisplayPrimitive::Text { .. })));
}

/// A provider that emits a single `Image` quad, so the paper-build image
/// clip path can be exercised without an importer.
struct ImageProvider;

impl crate::RepresentationProvider for ImageProvider {
    fn registration(&self) -> crate::Registration {
        crate::Registration {
            type_key: "test.image".into(),
            version: 1,
            priority: 10,
            entity_types: vec!["TestImage".into()],
            capabilities: vec!["image".into()],
        }
    }

    fn build(
        &self,
        entity: &DbEntity,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation> {
        let mut matrix = Transform3::identity().matrix;
        matrix[0][0] = 20000.0;
        matrix[1][1] = 20000.0;
        matrix[0][3] = 10.0;
        matrix[1][3] = 20.0;
        Ok(DisplayRepresentation {
            fragments: vec![DisplayFragment {
                source: SelectionRef {
                    document: context.document,
                    entity: entity.id,
                    instance: InstancePath::default(),
                    sub_element: None,
                },
                geometry_source: GeometrySource::Analytic,
                precision: Precision::Analytic,
                alpha: 1.0,
                color: crate::DEFAULT_RENDER_COLOR,
                color_unresolved: true,
                lineweight: crate::DEFAULT_LINEWEIGHT_MM,
                lineweight_unresolved: true,
                linetype: cad_db::LinetypePattern::continuous(),
                linetype_unresolved: true,
                linetype_scale: 1.0,
                primitive: DisplayPrimitive::Image {
                    resource: cad_resources::ResourceKey("img:0".into()),
                    transform: Transform3 { matrix },
                    clip: None,
                },
            }],
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        })
    }
}

#[test]
fn paper_build_clips_a_model_image_quad_with_interpolated_uvs() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "TestImage".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Opaque {
            type_key: "TestImage".into(),
            version: 1,
            payload: Vec::new(),
        },
        draw_order: 0,
    })
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: vec![simple_viewport()],
    })
    .unwrap();
    let db = b.finish().unwrap();
    let mut registry = ProviderRegistry::with_default_provider();
    registry.register(Box::new(ImageProvider)).unwrap();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    // The clipped textured quad is representable, so this stays exact.
    assert_eq!(rep.completeness, Completeness::Complete);
    let clip = rep
        .fragments
        .iter()
        .find_map(|f| match &f.primitive {
            DisplayPrimitive::Image { clip, .. } => clip.clone(),
            _ => None,
        })
        .expect("clipped image carries its polygon");
    assert_eq!(clip.len(), 4);
    // The image maps the unit square in model space to paper
    // [50,250]x[25,225] through the 1:100 window; the window cuts it to
    // [50,100]x[25,50]. The corner at (100,50) keeps texture UV (0.25,0.125).
    let corner = clip
        .iter()
        .find(|v| (v.position.x - 100.0).abs() < 1e-9 && (v.position.y - 50.0).abs() < 1e-9)
        .expect("clipped corner");
    assert!((corner.uv[0] - 0.25).abs() < 1e-9, "uv {:?}", corner.uv);
    assert!((corner.uv[1] - 0.125).abs() < 1e-9, "uv {:?}", corner.uv);
}

/// A provider that emits a single `Image` quad carrying the given entity
/// clip (in the primitive's model-space coordinates), so the entity-clip +
/// viewport-clip path can be exercised without an importer. The quad uses
/// the same model→paper mapping as [`ImageProvider`]: the unit square maps
/// to paper `[50,250]x[25,225]` inside the `1:100` window `[0,100]x[0,50]`.
struct ClippedImageProvider {
    clip: std::sync::Arc<[ImageVertex]>,
}

impl crate::RepresentationProvider for ClippedImageProvider {
    fn registration(&self) -> crate::Registration {
        crate::Registration {
            type_key: "test.clipped_image".into(),
            version: 1,
            priority: 10,
            entity_types: vec!["TestClippedImage".into()],
            capabilities: vec!["image".into()],
        }
    }

    fn build(
        &self,
        entity: &DbEntity,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation> {
        let mut matrix = Transform3::identity().matrix;
        matrix[0][0] = 20000.0;
        matrix[1][1] = 20000.0;
        matrix[0][3] = 10.0;
        matrix[1][3] = 20.0;
        Ok(DisplayRepresentation {
            fragments: vec![DisplayFragment {
                source: SelectionRef {
                    document: context.document,
                    entity: entity.id,
                    instance: InstancePath::default(),
                    sub_element: None,
                },
                geometry_source: GeometrySource::Analytic,
                precision: Precision::Analytic,
                alpha: 1.0,
                color: crate::DEFAULT_RENDER_COLOR,
                color_unresolved: true,
                lineweight: crate::DEFAULT_LINEWEIGHT_MM,
                lineweight_unresolved: true,
                linetype: cad_db::LinetypePattern::continuous(),
                linetype_unresolved: true,
                linetype_scale: 1.0,
                primitive: DisplayPrimitive::Image {
                    resource: cad_resources::ResourceKey("img:0".into()),
                    transform: Transform3 { matrix },
                    clip: Some(self.clip.clone()),
                },
            }],
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        })
    }
}

/// A database with one model-space `TestClippedImage` and the standard
/// `1:100` legacy viewport.
fn clipped_image_layout() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "TestClippedImage".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Opaque {
            type_key: "TestClippedImage".into(),
            version: 1,
            payload: Vec::new(),
        },
        draw_order: 0,
    })
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: vec![simple_viewport()],
    })
    .unwrap();
    b.finish().unwrap()
}

#[test]
fn paper_build_intersects_an_entity_clip_with_a_viewport_cut() {
    // Entity clip: the bottom strip of the image, model y in [20, 2020]
    // (texture v in [0, 0.1]) across the full width. In paper space it is
    // [50,250]x[25,45]; the viewport cuts it to [50,100]x[25,45], so the
    // surviving v stays at most 0.1. Dropping the entity clip (the old
    // behaviour) would have stretched to the full quad's v = 0.125.
    let clip: std::sync::Arc<[ImageVertex]> = std::sync::Arc::from(
        vec![
            ImageVertex {
                position: p(10.0, 20.0),
                uv: [0.0, 0.0],
            },
            ImageVertex {
                position: p(20010.0, 20.0),
                uv: [1.0, 0.0],
            },
            ImageVertex {
                position: p(20010.0, 2020.0),
                uv: [1.0, 0.1],
            },
            ImageVertex {
                position: p(10.0, 2020.0),
                uv: [0.0, 0.1],
            },
        ]
        .into_boxed_slice(),
    );
    let db = clipped_image_layout();
    let mut registry = ProviderRegistry::with_default_provider();
    registry
        .register(Box::new(ClippedImageProvider { clip }))
        .unwrap();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    // The intersection of two convex polygons is exact, not a degradation.
    assert_eq!(rep.completeness, Completeness::Complete);
    let polygon = rep
        .fragments
        .iter()
        .find_map(|f| match &f.primitive {
            DisplayPrimitive::Image { clip, .. } => clip.clone(),
            _ => None,
        })
        .expect("the intersected image carries its polygon");
    assert_eq!(polygon.len(), 4);
    let max_v = polygon.iter().map(|v| v.uv[1]).fold(f64::MIN, f64::max);
    assert!(
        (max_v - 0.1).abs() < 1e-9,
        "the entity clip's v=0.1 edge must survive, not the full quad's 0.125: {max_v}"
    );
    // The viewport cut still applies: no vertex escapes the window.
    for v in polygon.iter() {
        assert!(v.position.x <= 100.0 + 1e-9, "{:?}", v.position);
        assert!(v.position.y <= 50.0 + 1e-9, "{:?}", v.position);
    }
}

#[test]
fn paper_build_keeps_a_non_convex_entity_clip_and_reports_it() {
    // A concave "chevron" in model space; a provider contract violation, but
    // the paper build must not silently widen or drop it.
    let clip: std::sync::Arc<[ImageVertex]> = std::sync::Arc::from(
        vec![
            ImageVertex {
                position: p(10.0, 20.0),
                uv: [0.0, 0.0],
            },
            ImageVertex {
                position: p(20010.0, 20.0),
                uv: [1.0, 0.0],
            },
            ImageVertex {
                position: p(10010.0, 10020.0),
                uv: [0.5, 0.5],
            },
            ImageVertex {
                position: p(20010.0, 20020.0),
                uv: [1.0, 1.0],
            },
            ImageVertex {
                position: p(10.0, 20020.0),
                uv: [0.0, 1.0],
            },
        ]
        .into_boxed_slice(),
    );
    let db = clipped_image_layout();
    let mut registry = ProviderRegistry::with_default_provider();
    registry
        .register(Box::new(ClippedImageProvider { clip }))
        .unwrap();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert!(rep.diagnostics.iter().any(|d| {
        d.code == "representation.viewport_clip_partial"
            && d.message
                .contains(clip_reason::IMAGE_INTERSECTION_UNSUPPORTED)
    }));
    // The entity clip is preserved unchanged (never widened to the window).
    let polygon = rep
        .fragments
        .iter()
        .find_map(|f| match &f.primitive {
            DisplayPrimitive::Image { clip, .. } => clip.clone(),
            _ => None,
        })
        .expect("the entity clip is kept");
    assert_eq!(polygon.len(), 5);
}
