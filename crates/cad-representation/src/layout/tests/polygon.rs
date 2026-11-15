use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, PaperViewport};
use cad_domain::{Completeness, DatabaseId, LayoutId, Point3, Transform3};

use crate::layout::clip_polygon::{clip_conic, clip_filled_polygon, clip_mesh, clip_polyline};
use crate::layout::image::{visible_image, ImageRegion};
use crate::layout::polygon::validate_paper_polygon;
use crate::{ConicSegment, DisplayPrimitive, ProviderRegistry};

/// A concave six-point "chevron" clip: `(0,0) (5,5) (10,0) (10,3) (5,8) (0,3)`.
///
/// It is concave at `(5,5)`, so a horizontal line at `y = 2` enters at `x = 0`,
/// leaves at `x = 2`, re-enters at `x = 8` and leaves at `x = 10`: **two**
/// visible spans of one segment. Its area is exactly 30.
fn chevron() -> Vec<[f64; 2]> {
    vec![
        [0.0, 0.0],
        [5.0, 5.0],
        [10.0, 0.0],
        [10.0, 3.0],
        [5.0, 8.0],
        [0.0, 3.0],
    ]
}

/// A concave six-point **L**: a 10x10 square with the top-right quad
/// `x > 6, y > 4` removed, so its inside is trivial to reason about.
fn l_shape() -> Vec<[f64; 2]> {
    vec![
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 4.0],
        [6.0, 4.0],
        [6.0, 10.0],
        [0.0, 10.0],
    ]
}

fn l_clip() -> PaperClip {
    let boundary = validate_paper_polygon(&points(&l_shape())).expect("the L is a simple polygon");
    PaperClip::new(boundary, None)
}

fn chevron_clip() -> PaperClip {
    let boundary =
        validate_paper_polygon(&points(&chevron())).expect("the chevron is a simple polygon");
    PaperClip::new(boundary, None)
}

fn points(boundary: &[[f64; 2]]) -> Vec<Point3> {
    boundary.iter().map(|c| p(c[0], c[1])).collect()
}

/// Twice the signed area of a polygon, halved: the polygon area.
fn area(boundary: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for index in 0..boundary.len() {
        let a = boundary[index];
        let b = boundary[(index + 1) % boundary.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    (sum * 0.5).abs()
}

fn positions(vertices: &[crate::ImageVertex]) -> Vec<[f64; 2]> {
    vertices
        .iter()
        .map(|v| [v.position.x, v.position.y])
        .collect()
}

// ---- the concave clip splits one segment into its real spans -------------

#[test]
fn concave_six_point_clip_splits_a_horizontal_line_into_two_visible_spans() {
    let clip = chevron_clip();
    assert!(!clip.is_convex(), "the chevron really is concave");
    assert!(clip.as_rect().is_none());

    let runs = clip_polyline(&[p(-1.0, 2.0), p(11.0, 2.0)], &clip);
    assert_eq!(
        runs.len(),
        2,
        "two inside spans, not one bounding box: {runs:?}"
    );
    assert_eq!(runs[0].len(), 2);
    assert_eq!(runs[1].len(), 2);
    assert!((runs[0][0].x - 0.0).abs() < 1e-9, "{:?}", runs[0]);
    assert!((runs[0][1].x - 2.0).abs() < 1e-9, "{:?}", runs[0]);
    assert!((runs[1][0].x - 8.0).abs() < 1e-9, "{:?}", runs[1]);
    assert!((runs[1][1].x - 10.0).abs() < 1e-9, "{:?}", runs[1]);
    for run in &runs {
        for vertex in run {
            assert!((vertex.y - 2.0).abs() < 1e-9);
        }
    }
}

#[test]
fn concave_clip_keeps_a_fully_inside_polyline_unchanged() {
    let clip = l_clip();
    assert!(!clip.is_convex());
    let inside = [p(1.0, 1.0), p(2.0, 2.0), p(3.0, 1.0)];
    let runs = clip_polyline(&inside, &clip);
    assert_eq!(runs, vec![inside.to_vec()]);
}

#[test]
fn concave_clip_drops_a_polyline_that_misses_it() {
    let clip = chevron_clip();
    let runs = clip_polyline(&[p(-5.0, 20.0), p(20.0, 20.0)], &clip);
    assert!(runs.is_empty());
}

#[test]
fn concave_clip_joins_a_run_that_stays_inside_across_a_vertex() {
    let clip = l_clip();
    // Every vertex is inside the L and no edge leaves it, so the three points
    // stay one continuous run rather than three single spans.
    let runs = clip_polyline(&[p(1.0, 1.0), p(4.0, 1.0), p(5.0, 3.0)], &clip);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].len(), 3, "{:?}", runs[0]);
}

// ---- meshes, filled polygons and images --------------------------------

#[test]
fn concave_clip_limits_a_mesh_to_the_polygon_area() {
    let clip = chevron_clip();
    let mesh = Mesh {
        vertices: vec![p(-2.0, -2.0), p(12.0, -2.0), p(12.0, 12.0), p(-2.0, 12.0)],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    let clipped = clip_mesh(&mesh, &clip)
        .expect("finite mesh")
        .expect("the mesh covers the clip");
    let clipped_area = mesh_area(&clipped);
    assert!(
        (clipped_area - 30.0).abs() < 1e-9,
        "clipped area {clipped_area} must equal the clip area"
    );
    for vertex in &clipped.vertices {
        assert!(
            clip.contains([vertex.x, vertex.y]),
            "a clipped vertex leaked out of the polygon: {vertex:?}"
        );
    }
}

#[test]
fn concave_clip_limits_a_filled_polygon_to_the_polygon() {
    let clip = chevron_clip();
    let quad = [[-2.0, -2.0], [12.0, -2.0], [12.0, 12.0], [-2.0, 12.0]];
    let pieces = clip_filled_polygon(&quad, &clip);
    assert!(!pieces.is_empty());
    let clipped: f64 = pieces.iter().map(|piece| area(piece)).sum();
    assert!(
        (clipped - 30.0).abs() < 1e-9,
        "the pieces must cover exactly the clip: {clipped}"
    );
    for piece in &pieces {
        assert!(piece.len() >= 3);
        for vertex in piece {
            assert!(clip.contains(*vertex), "{vertex:?} leaked out");
        }
    }
}

#[test]
fn concave_clip_limits_an_image_to_the_polygon() {
    let clip = chevron_clip();
    // The unit square mapped to (-2,-2)-(12,12), so the image covers the clip.
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = 14.0;
    matrix[1][1] = 14.0;
    matrix[0][3] = -2.0;
    matrix[1][3] = -2.0;
    let transform = Transform3 { matrix };
    let region = visible_image(&transform, None, &clip).expect("finite quad");
    let ImageRegion::Pieces(pieces) = region else {
        panic!("an image larger than a concave clip must be cut into pieces");
    };
    assert!(!pieces.is_empty());
    let visible: f64 = pieces.iter().map(|piece| area(&positions(piece))).sum();
    assert!(
        (visible - 30.0).abs() < 1e-9,
        "the image pieces must cover exactly the clip: {visible}"
    );
    for piece in &pieces {
        for vertex in piece {
            assert!(clip.contains([vertex.position.x, vertex.position.y]));
            assert!(
                (-1e-9..=1.0 + 1e-9).contains(&vertex.uv[0])
                    && (-1e-9..=1.0 + 1e-9).contains(&vertex.uv[1]),
                "UV {:?} left the unit square",
                vertex.uv
            );
        }
    }
}

#[test]
fn an_image_fully_inside_a_concave_clip_needs_no_cut() {
    let clip = l_clip();
    // The unit square maps to the small square (4,1)-(5,2), inside the L.
    let transform = Transform3::translation(p(4.0, 1.0));
    match visible_image(&transform, None, &clip).expect("finite quad") {
        ImageRegion::Whole { entity } => assert!(entity.is_none()),
        _ => panic!("a wholly visible image keeps its plain unit-square draw"),
    }
}

#[test]
fn an_image_outside_a_concave_clip_is_an_exact_omission() {
    let clip = chevron_clip();
    let transform = Transform3::translation(p(100.0, 100.0));
    assert!(matches!(
        visible_image(&transform, None, &clip).expect("finite quad"),
        ImageRegion::Empty
    ));
}

#[test]
fn concave_clip_keeps_only_the_inside_arcs_of_a_circle() {
    let clip = chevron_clip();
    // A circle of radius 6 around (2,2) crosses the L's bottom, left and notch
    // edges while its right arc stays inside the viewport.
    let conic = ConicSegment {
        center: p(2.0, 2.0),
        major: p(6.0, 0.0),
        minor: p(0.0, 6.0),
        start: 0.0,
        sweep: std::f64::consts::TAU,
        full: true,
    };
    let pieces = clip_conic(&conic, &clip).expect("finite conic");
    assert!(!pieces.is_empty());
    for piece in &pieces {
        assert!(
            !piece.full,
            "no whole circle may leak out of a polygon clip"
        );
        for step in 0..=64 {
            let t = piece.start + piece.sweep * (step as f64 / 64.0);
            let x = piece.center.x + piece.major.x * t.cos() + piece.minor.x * t.sin();
            let y = piece.center.y + piece.major.y * t.cos() + piece.minor.y * t.sin();
            assert!(clip.contains([x, y]), "({x}, {y}) leaked out of the clip");
        }
    }
}

// ---- rectangle compatibility -------------------------------------------

#[test]
fn a_rectangle_clip_is_still_carried_as_a_rectangle() {
    let ViewportState::Supported(t) = viewport_transform(&four_corner_viewport()) else {
        panic!("expected a supported viewport");
    };
    assert_eq!(t.paper_clip.as_rect(), Some(([0.0, 0.0], [100.0, 50.0])));
    assert_eq!(
        t.paper_clip.boundary(),
        &[[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]],
        "the rectangle keeps its canonical corner order"
    );
    assert!(t.paper_clip.is_convex());
    // The half-plane path is still the one rectangles take, so its output is
    // identical to the rectangle clipper: a segment across the right edge is cut
    // at x = 100 and the run geometry is unchanged.
    let segment = [p(95.0, 25.0), p(105.0, 25.0)];
    let dispatched = clip_polyline(&segment, &t.paper_clip);
    let rectangle = clip_polyline_to_rect(&segment, [50.0, 25.0], [50.0, 25.0]);
    assert_eq!(dispatched, rectangle);
    assert!(
        (dispatched[0][1].x - 100.0).abs() < 1e-9,
        "{:?}",
        dispatched
    );
}

// ---- a triangle (three points) now works -------------------------------

fn triangle_viewport() -> PaperViewport {
    // Paper triangle (0,0) (10,0) (0,10) with a real transform translation, the
    // shape polygonal `MVIEW`/`VPCLIP` write: `model = paper - paper_center`.
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = 1.0;
    matrix[1][1] = 1.0;
    matrix[0][3] = -10.0 / 3.0;
    matrix[1][3] = -10.0 / 3.0;
    PaperViewport {
        clip: vec![p(0.0, 0.0), p(10.0, 0.0), p(0.0, 10.0)],
        model_to_paper: Transform3 { matrix },
        completeness: Completeness::Complete,
        layer_overrides: std::collections::BTreeMap::new(),
    }
}

#[test]
fn a_three_point_clip_with_a_transform_is_a_triangle_viewport() {
    let ViewportState::Supported(t) = viewport_transform(&triangle_viewport()) else {
        panic!("a three-point polygon clip must be supported");
    };
    assert_eq!(
        t.paper_clip.boundary(),
        &[[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]]
    );
    assert!(t.paper_clip.as_rect().is_none());
    assert_eq!(t.paper_corners[0], p(0.0, 0.0));
    assert_eq!(t.paper_corners[2], p(10.0, 10.0));

    // The diagonal x + y = 10 is the hypotenuse: a horizontal line at y = 5 is
    // visible for x in 0..5 only.
    let runs = clip_polyline(&[p(-1.0, 5.0), p(11.0, 5.0)], &t.paper_clip);
    assert_eq!(runs.len(), 1);
    assert!((runs[0][0].x - 0.0).abs() < 1e-9, "{:?}", runs[0]);
    assert!((runs[0][1].x - 5.0).abs() < 1e-9, "{:?}", runs[0]);
}

// ---- refused by name ---------------------------------------------------

fn polygon_clip_unsupported(vertices: Vec<Point3>) -> ViewportUnsupported {
    let mut viewport = triangle_viewport();
    viewport.clip = vertices;
    match viewport_transform(&viewport) {
        ViewportState::Unsupported(why) => why,
        ViewportState::Supported(_) => panic!("expected a refused clip"),
    }
}

#[test]
fn a_clip_with_fewer_than_three_points_is_refused_by_arity() {
    let why = polygon_clip_unsupported(vec![p(0.0, 0.0), p(10.0, 0.0)]);
    assert_eq!(why.code, viewport_reason::CLIP_ARITY);
}

#[test]
fn a_degenerate_polygon_is_refused_by_name() {
    // Zero-area collinear boundary.
    let why = polygon_clip_unsupported(vec![p(0.0, 0.0), p(5.0, 5.0), p(10.0, 10.0)]);
    assert_eq!(why.code, viewport_reason::POLYGON_DEGENERATE);

    // A repeated vertex is degenerate, not a silent drop.
    let why = polygon_clip_unsupported(vec![
        p(0.0, 0.0),
        p(10.0, 0.0),
        p(10.0, 0.0),
        p(0.0, 10.0),
        p(5.0, 2.0),
    ]);
    assert_eq!(why.code, viewport_reason::POLYGON_DEGENERATE);
}

#[test]
fn a_self_intersecting_polygon_is_refused_by_name() {
    // A bow tie over four distinct points.
    let why =
        polygon_clip_unsupported(vec![p(0.0, 0.0), p(10.0, 10.0), p(10.0, 0.0), p(0.0, 10.0)]);
    assert_eq!(why.code, viewport_reason::POLYGON_SELF_INTERSECTING);
}

#[test]
fn a_non_planar_polygon_is_refused_by_name() {
    let why = polygon_clip_unsupported(vec![
        p(0.0, 0.0),
        p(10.0, 0.0),
        Point3 {
            x: 0.0,
            y: 10.0,
            z: 3.0,
        },
    ]);
    assert_eq!(why.code, viewport_reason::POLYGON_NON_PLANAR);
}

// ---- the paper build uses the polygon where it used to refuse ----------

fn model_line(id: u128, start: Point3, end: Point3) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line { start, end },
        draw_order: id as i64,
    }
}

fn polygon_layout(clip: Vec<Point3>, translation: [f64; 2]) -> cad_db::Layout {
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = 1.0;
    matrix[1][1] = 1.0;
    matrix[0][3] = translation[0];
    matrix[1][3] = translation[1];
    cad_db::Layout {
        id: LayoutId(1),
        name: "Poly".into(),
        viewports: vec![PaperViewport {
            clip,
            model_to_paper: Transform3 { matrix },
            completeness: Completeness::Complete,
            layer_overrides: std::collections::BTreeMap::new(),
        }],
    }
}

#[test]
fn a_notched_viewport_clips_model_geometry_to_the_notch() {
    // An 8-point notch: a 10x10 square minus the x in 3..7, y in 0..5 bite.
    let clip = vec![
        p(0.0, 0.0),
        p(3.0, 0.0),
        p(3.0, 5.0),
        p(7.0, 5.0),
        p(7.0, 0.0),
        p(10.0, 0.0),
        p(10.0, 10.0),
        p(0.0, 10.0),
    ];
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // One model line straight across the viewport at paper y = 2, where the
    // notch removes x in 3..7: exactly two visible spans survive.
    b.insert_entity(model_line(1, p(-1.0, 2.0), p(11.0, 2.0)))
        .unwrap();
    b.insert_layout(polygon_layout(clip, [0.0, 0.0])).unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    assert!(
        !rep.diagnostics
            .iter()
            .any(|d| d.code == "representation.viewport_unsupported"),
        "a simple polygon viewport must no longer be refused: {:?}",
        rep.diagnostics
    );
    let mut spans = Vec::new();
    for fragment in &rep.fragments {
        if let DisplayPrimitive::Lines(points) = &fragment.primitive {
            assert_eq!(points.len(), 2, "{:?}", points);
            spans.push((points[0].x, points[1].x));
        }
    }
    spans.sort_by(|left, right| left.0.partial_cmp(&right.0).unwrap());
    assert_eq!(spans.len(), 2, "{spans:?}");
    assert!((spans[0].0 - 0.0).abs() < 1e-9, "{spans:?}");
    assert!((spans[0].1 - 3.0).abs() < 1e-9, "{spans:?}");
    assert!((spans[1].0 - 7.0).abs() < 1e-9, "{spans:?}");
    assert!((spans[1].1 - 10.0).abs() < 1e-9, "{spans:?}");
}

#[test]
fn a_polygon_viewport_builds_a_mesh_only_inside_the_polygon() {
    let clip = vec![
        p(0.0, 0.0),
        p(5.0, 5.0),
        p(10.0, 0.0),
        p(10.0, 3.0),
        p(5.0, 8.0),
        p(0.0, 3.0),
    ];
    let region = chevron_clip();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // A filled region spanning the whole bounding box: the viewport must clip
    // every triangle, not just the envelope.
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbSolid".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mesh(Mesh {
            vertices: vec![p(-2.0, -2.0), p(12.0, -2.0), p(12.0, 12.0), p(-2.0, 12.0)],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        }),
        draw_order: 0,
    })
    .unwrap();
    b.insert_layout(polygon_layout(clip, [0.0, 0.0])).unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
    let mut total = 0.0;
    let mut seen = 0;
    for fragment in &rep.fragments {
        if let DisplayPrimitive::Mesh(mesh) = &fragment.primitive {
            total += mesh_area(mesh);
            seen += 1;
            for vertex in &mesh.vertices {
                assert!(
                    region.contains([vertex.x, vertex.y]),
                    "a mesh vertex leaked out of the polygon: {vertex:?}"
                );
            }
        }
    }
    assert!(seen > 0, "the hatch must still draw inside the polygon");
    assert!(
        (total - 30.0).abs() < 1e-6,
        "the visible filled area must equal the clip area: {total}"
    );
}
