//! INSERT/block placement: base point, OCS, arrays, SOLID and splines.

use super::*;
// ---- B31: INSERT base point, OCS normal and array semantics ----

#[test]
fn insert_subtracts_the_block_base_point() {
    // A block whose base point is (1, 1) and an INSERT at (10, 10): the
    // block point (1, 1) must land on (10, 10), not (11, 11).
    let mut i =
        acadrust::entities::Insert::new("BLOCK", acadrust::types::Vector3::new(10.0, 10.0, 0.0));
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
