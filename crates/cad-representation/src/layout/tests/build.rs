use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, PaperViewport};
use std::sync::Arc;

use crate::{ConicStats, DisplayPrimitive, ImageVertex, ProviderRegistry};

// ---- paper build -------------------------------------------------------

#[test]
fn paper_build_maps_and_clips_model_geometry_through_the_viewport() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for (id, a, z) in [
        // y = 20000 is far above the 1:100 window (visible model y is
        // 20 ± 2500), so this line is clipped away entirely.
        (1u128, p(-5000.0, 20000.0), p(5000.0, 20000.0)),
        (2, p(9.0, 20.0), p(11.0, 20.0)),
    ] {
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line { start: a, end: z },
            draw_order: id as i64,
        })
        .unwrap();
    }
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: vec![simple_viewport()],
    })
    .unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    assert_eq!(rep.completeness, Completeness::Complete);
    assert_eq!(rep.fragments.len(), 1);
    match &rep.fragments[0].primitive {
        DisplayPrimitive::Lines(pts) => {
            assert_eq!(pts.len(), 2);
            // Model x=9 => paper 50 + (9-10)*0.01 = 49.99; x=11 => 50.01.
            assert!((pts[0].x - 49.99).abs() < 1e-9, "got {:?}", pts[0]);
            assert!((pts[1].x - 50.01).abs() < 1e-9, "got {:?}", pts[1]);
            assert!((pts[0].y - 25.0).abs() < 1e-9);
        }
        _ => panic!("expected a clipped line"),
    }
}

#[test]
fn paper_build_reports_an_unsupported_viewport_as_partial() {
    let db = db_with_layouts();
    let registry = ProviderRegistry::with_default_provider();
    // Layout2's only viewport has a zero scale: unsupported. The sheet is
    // still a sheet, so the result is Partial (not Missing) and no guessed
    // viewport geometry is emitted.
    let rep = build_paper_space(&registry, &db, LayoutId(2), &context(), &|_| true).unwrap();
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert!(rep.fragments.is_empty());
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.viewport_unsupported"));
}

#[test]
fn paper_build_draws_paper_entities_directly() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // A paper-space entity belonging to layout 1, plus a layout with a
    // deliberately unsupported viewport (zero scale).
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Paper(LayoutId(1)),
        geometry: SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(5.0, 0.0),
        },
        draw_order: 0,
    })
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: vec![viewport(vec![p(0.0, 0.0), p(10.0, 10.0), p(0.0, 0.0)], 0.0)],
    })
    .unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    // The paper entity is present verbatim even though the viewport is
    // unsupported; the unsupported viewport is reported, not guessed.
    assert_eq!(rep.fragments.len(), 1);
    match &rep.fragments[0].primitive {
        DisplayPrimitive::Lines(pts) => {
            assert_eq!(pts[0], p(0.0, 0.0));
            assert_eq!(pts[1], p(5.0, 0.0));
        }
        _ => panic!("expected the paper line"),
    }
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.viewport_unsupported"));
}

#[test]
fn paper_build_reports_a_rotated_viewport_as_partial_not_a_squared_off_guess() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "Rotated".into(),
        viewports: vec![PaperViewport {
            // A 45°-rotated paper rectangle: four corners, but not axis
            // aligned, so it must be refused rather than squared off.
            clip: vec![p(50.0, 0.0), p(100.0, 50.0), p(50.0, 100.0), p(0.0, 50.0)],
            model_to_paper: Transform3::scale(100.0),
            completeness: Completeness::Complete,
        }],
    })
    .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert!(rep.fragments.is_empty());
    let diagnostic = rep
        .diagnostics
        .iter()
        .find(|d| d.code == "representation.viewport_unsupported")
        .expect("rotated viewport must be reported");
    assert!(
        diagnostic.message.contains(viewport_reason::ROTATED_CLIP),
        "got {}",
        diagnostic.message
    );
}

#[test]
fn paper_build_for_a_missing_layout_is_missing_not_empty_success() {
    let db = db_with_layouts();
    let registry = ProviderRegistry::with_default_provider();
    let rep = build_paper_space(&registry, &db, LayoutId(99), &context(), &|_| true).unwrap();
    assert!(matches!(rep.completeness, Completeness::Missing(_)));
    assert!(rep.diagnostics.iter().any(|d| d.code == "layout.missing"));
}

#[test]
fn paper_build_honours_the_visibility_predicate() {
    let db = db_with_layouts();
    let registry = ProviderRegistry::with_default_provider();
    // Everything hidden: layout 1 has no paper entities and the model line
    // is filtered, so nothing is drawn.
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| false).unwrap();
    assert!(rep.fragments.is_empty());
    assert_eq!(rep.completeness, Completeness::Complete);
}

// ---- per-triangle / per-quad viewport clipping -------------------------
#[test]
fn glyph_outline_quad_is_clipped_by_the_line_path() {
    // Shaped text arrives as `Lines` glyph outlines, so a font-shaped glyph
    // is clipped by the same Liang–Barsky path as any other polyline. A
    // closed square contour straddling the window keeps only its inside runs.
    let contour = [
        p(-2.0, -1.0),
        p(2.0, -1.0),
        p(2.0, 1.0),
        p(-2.0, 1.0),
        p(-2.0, -1.0),
    ];
    let runs = clip_polyline_to_rect(&contour, [0.0, 0.0], [1.0, 1.0]);
    assert!(!runs.is_empty());
    for run in &runs {
        assert!(run.len() >= 2);
        for v in run {
            assert!(v.x >= -1.0 - 1e-9 && v.x <= 1.0 + 1e-9);
            assert!(v.y >= -1.0 - 1e-9 && v.y <= 1.0 + 1e-9);
        }
    }
}

#[test]
fn clip_polygon_to_rect_keeps_inside_and_drops_outside() {
    let inside = clip_polygon_to_rect(
        &[[-0.5, -0.5], [0.5, -0.5], [0.0, 0.5]],
        [0.0, 0.0],
        [1.0, 1.0],
    );
    assert_eq!(inside.len(), 3);
    let outside = clip_polygon_to_rect(
        &[[2.0, 2.0], [3.0, 2.0], [2.5, 3.0]],
        [0.0, 0.0],
        [1.0, 1.0],
    );
    assert!(outside.is_empty());
}

#[test]
fn mesh_triangle_fully_inside_is_reused_verbatim() {
    let mesh = triangle_mesh([p(-0.5, -0.5), p(0.5, -0.5), p(0.0, 0.5)]);
    let clipped = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
        .unwrap()
        .expect("a triangle inside the window survives");
    assert_eq!(clipped.triangles.len(), 1);
    assert_eq!(clipped.vertices.len(), 3);
    assert_eq!(clipped.vertices[0], p(-0.5, -0.5));
    assert_eq!(clipped.vertices[2], p(0.0, 0.5));
}

#[test]
fn mesh_triangle_fully_outside_disappears() {
    let mesh = triangle_mesh([p(2.0, 2.0), p(3.0, 2.0), p(2.5, 3.0)]);
    assert!(clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
        .unwrap()
        .is_none());
}

#[test]
fn mesh_triangle_partially_clipped_keeps_area_and_fan_triangles() {
    // Triangle (-2,-2),(2,-2),(0,2) ∩ [-1,1]² is a hexagon of area 3.5.
    let mesh = triangle_mesh([p(-2.0, -2.0), p(2.0, -2.0), p(0.0, 2.0)]);
    let clipped = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
        .unwrap()
        .expect("the triangle overlaps the window");
    assert_eq!(clipped.vertices.len(), 6);
    // A fan over an n-gon yields n-2 triangles.
    assert_eq!(clipped.triangles.len(), clipped.vertices.len() - 2);
    let area = mesh_area(&clipped);
    assert!((area - 3.5).abs() < 1e-9, "clipped area {area}");
    // Every clipped vertex stays inside the window.
    for v in &clipped.vertices {
        assert!(v.x >= -1.0 - 1e-9 && v.x <= 1.0 + 1e-9);
        assert!(v.y >= -1.0 - 1e-9 && v.y <= 1.0 + 1e-9);
    }
}

#[test]
fn mesh_per_vertex_colors_interpolate_at_clip_vertices() {
    // Red ramp along x: (-2,0)=0, (0,0)=100, (0,2)=200.
    let mesh = Mesh {
        vertices: vec![p(-2.0, 0.0), p(0.0, 0.0), p(0.0, 2.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: vec![[0, 0, 0], [100, 0, 0], [200, 0, 0]],
    };
    let clipped = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
        .unwrap()
        .expect("overlaps");
    assert_eq!(clipped.colors.len(), clipped.vertices.len());
    // (-1,0) is the x=-1 crossing of the (-2,0)->(0,0) edge: 50%.
    let left = clipped
        .vertices
        .iter()
        .position(|v| (v.x + 1.0).abs() < 1e-9 && v.y.abs() < 1e-9)
        .expect("left crossing");
    assert_eq!(clipped.colors[left][0], 50, "interpolated red channel");
    // (0,1) is the y=1 crossing of the (0,0)->(0,2) edge: 150.
    let top = clipped
        .vertices
        .iter()
        .position(|v| v.x.abs() < 1e-9 && (v.y - 1.0).abs() < 1e-9)
        .expect("top crossing");
    assert_eq!(clipped.colors[top][0], 150, "interpolated red channel");
}

#[test]
fn mesh_with_a_missing_index_is_refused_not_dropped() {
    let mesh = Mesh {
        vertices: vec![p(0.0, 0.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    let refusal = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
        .expect_err("a missing index cannot be clipped exactly");
    assert_eq!(refusal.reason, clip_reason::MESH_UNCLIPPABLE);
}

#[test]
fn mesh_with_a_non_finite_vertex_is_refused_not_dropped() {
    let mesh = triangle_mesh([p(f64::NAN, 0.0), p(0.0, 0.0), p(1.0, 1.0)]);
    let refusal = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
        .expect_err("a non-finite mesh cannot be clipped exactly");
    assert_eq!(refusal.reason, clip_reason::MESH_UNCLIPPABLE);
}

#[test]
fn image_quad_clip_interpolates_uvs() {
    // Identity image: unit square with UV = position. The window looks at
    // x in [0,1], y in [-0.5,0.5], so the top edge is clipped at y=0.5 and
    // its template coordinate v must interpolate to 0.5 (not stretch).
    let vertices = clip_image_quad_to_rect(&Transform3::identity(), [0.5, 0.0], [0.5, 0.5])
        .expect("finite quad");
    assert_eq!(vertices.len(), 4);
    for v in &vertices {
        assert!((v.position.x - v.uv[0]).abs() < 1e-9);
        if (v.position.y - 0.5).abs() < 1e-9 {
            assert!((v.uv[1] - 0.5).abs() < 1e-9, "uv {:?}", v.uv);
        }
    }
}

#[test]
fn image_quad_fully_outside_clips_to_nothing() {
    let vertices = clip_image_quad_to_rect(&Transform3::identity(), [3.0, 3.0], [1.0, 1.0])
        .expect("finite quad");
    assert!(vertices.is_empty());
}

#[test]
fn transformed_image_clip_maps_positions_but_keeps_uvs() {
    let clip = std::sync::Arc::from(
        vec![
            ImageVertex {
                position: p(0.0, 0.0),
                uv: [0.0, 0.0],
            },
            ImageVertex {
                position: p(1.0, 0.0),
                uv: [1.0, 0.0],
            },
        ]
        .into_boxed_slice(),
    );
    let primitive = DisplayPrimitive::Image {
        resource: cad_resources::ResourceKey("img:0".into()),
        transform: Transform3::identity(),
        clip: Some(clip),
    };
    match primitive.transformed(&Transform3::translation(p(10.0, 5.0))) {
        DisplayPrimitive::Image { clip: Some(v), .. } => {
            assert_eq!(v[0].position, p(10.0, 5.0));
            assert_eq!(v[1].position, p(11.0, 5.0));
            assert_eq!(v[0].uv, [0.0, 0.0]);
            assert_eq!(v[1].uv, [1.0, 0.0]);
        }
        _ => panic!("expected a clipped image primitive"),
    }
}

// ---- one model build, reused by every viewport -------------------------

/// A four-corner 1:100 viewport whose (0,0)-(100,50) paper rectangle starts at
/// `min_x`, still looking at model centre (10, 20). Two of these see the same
/// model geometry through different paper windows.
fn four_corner_viewport_at(min_x: f64) -> PaperViewport {
    let paper_center_x = min_x + 50.0;
    let translation = Point3 {
        x: 10.0 - 100.0 * paper_center_x,
        y: 20.0 - 100.0 * 25.0,
        z: 0.0,
    };
    PaperViewport {
        clip: vec![
            p(min_x, 0.0),
            p(min_x + 100.0, 0.0),
            p(min_x + 100.0, 50.0),
            p(min_x, 50.0),
        ],
        model_to_paper: stored_view(100.0, translation),
        completeness: Completeness::Complete,
    }
}

fn model_entity(id: u128, geometry: SemanticGeometry, draw_order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: match &geometry {
                SemanticGeometry::Circle { .. } => "AcDbCircle",
                SemanticGeometry::Text { .. } => "AcDbText",
                _ => "AcDbLine",
            }
            .into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order,
    }
}

#[test]
fn paper_build_reuses_one_model_representation_across_viewports() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // The analytic circle is the observable build: every `build_expanded` call
    // records one produced conic.
    b.insert_entity(model_entity(
        1,
        SemanticGeometry::Circle {
            center: p(10.0, 20.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 5.0,
        },
        0,
    ))
    .unwrap();
    // A line the viewport clip actually cuts, so each viewport's own paper
    // window is observable as a separate fragment.
    b.insert_entity(model_entity(
        2,
        SemanticGeometry::Line {
            start: p(5.0, 20.0),
            end: p(15.0, 20.0),
        },
        1,
    ))
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: vec![four_corner_viewport_at(0.0), four_corner_viewport_at(200.0)],
    })
    .unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let stats = Arc::new(ConicStats::new());
    let ctx = context().with_conic_stats(stats.clone());
    let rep = build_paper_space(&registry, &db, LayoutId(1), &ctx, &|_| true).unwrap();

    // The model entity is built once even though two viewports see it; the old
    // per-viewport loop recorded two produced conics.
    assert_eq!(stats.produced(), 1, "one model build feeds both viewports");

    // Both viewports still emit their own clipped copy of the same model line:
    // paper centre x = 50 and 250 respectively, scale 1:100.
    let lines: Vec<_> = rep
        .fragments
        .iter()
        .filter_map(|f| match &f.primitive {
            DisplayPrimitive::Lines(points) => Some(points.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 2, "one clipped line per viewport");
    for (points, expected_start) in lines.iter().zip([49.95_f64, 249.95]) {
        assert_eq!(points.len(), 2);
        assert!(
            (points[0].x - expected_start).abs() < 1e-9,
            "got {:?}",
            points[0]
        );
        assert!(
            (points[1].x - (expected_start + 0.1)).abs() < 1e-9,
            "got {:?}",
            points[1]
        );
    }

    // The single built circle is transformed independently into both windows.
    let conics = rep
        .fragments
        .iter()
        .filter(|f| matches!(f.primitive, DisplayPrimitive::Conic(_)))
        .count();
    assert_eq!(
        conics, 2,
        "the same built circle is placed in both viewports"
    );
}

#[test]
fn paper_build_dedups_repeated_clip_diagnostics() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // An unshaped model text placeholder cannot be clipped exactly, so every
    // viewport that draws it would report the same limitation.
    b.insert_entity(model_entity(
        1,
        SemanticGeometry::Text {
            text: "A".into(),
            position: p(10.0, 20.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
        },
        0,
    ))
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: vec![four_corner_viewport_at(0.0), four_corner_viewport_at(200.0)],
    })
    .unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();

    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    let clip_diagnostics: Vec<_> = rep
        .diagnostics
        .iter()
        .filter(|d| d.code == "representation.viewport_clip_partial")
        .collect();
    // Same (reason, message) for both viewports: reported once, naming the
    // first viewport that hit it.
    assert_eq!(
        clip_diagnostics.len(),
        1,
        "a shared clip limitation is reported once per layout build"
    );
    assert!(clip_diagnostics[0]
        .message
        .contains(clip_reason::TEXT_FONT_DEPENDENT));
    assert!(
        clip_diagnostics[0].message.contains("viewport 0"),
        "got {}",
        clip_diagnostics[0].message
    );

    // Deduplicating the report never drops geometry: both viewports still draw
    // their own unclipped placeholder.
    let texts = rep
        .fragments
        .iter()
        .filter(|f| matches!(f.primitive, DisplayPrimitive::Text { .. }))
        .count();
    assert_eq!(texts, 2, "both viewports keep their unclipped text");
}
