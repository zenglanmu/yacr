//! OCS/extrusion normalisation of flat entities.

use super::*;
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

// ---- `-Z` extrusion: the OCS is the x-mirror of world XY ----

/// Convert one entity through the real importer dispatch, so the test covers
/// the path a file takes, not just the helpers.
fn convert_entity(entity: EntityType) -> (SemanticGeometry, Completeness) {
    let acad = acadrust::CadDocument::new();
    let req = request(Vec::new());
    let mut builder =
        ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));
    let common = entity.common().clone();
    builder.convert(&entity, &common)
}

fn lwpolyline(normal: Point3, elevation: f64, verts: &[(f64, f64, f64)]) -> EntityType {
    let mut pl = acadrust::entities::LwPolyline::new();
    for &(x, y, bulge) in verts {
        pl.add_point_with_bulge(acadrust::types::Vector2::new(x, y), bulge);
    }
    pl.elevation = elevation;
    pl.normal = acadrust::types::Vector3::new(normal.x, normal.y, normal.z);
    EntityType::LwPolyline(pl)
}

fn minus_z() -> Point3 {
    Point3 {
        x: 0.0,
        y: 0.0,
        z: -1.0,
    }
}

#[test]
fn minus_z_lwpolyline_is_the_mirror_of_the_plus_z_file() {
    // Same OCS data, `+Z` and `-Z` extrusions. A `-Z` OCS is the x-mirror of
    // world XY (`elevation` runs down -Z), and a mirror reverses every arc, so
    // the world polyline must mirror `x`/`z` and negate each bulge.
    let verts = [(1.0, 0.0, 0.5), (2.0, 1.0, 0.0), (3.0, 0.0, 0.0)];
    let (plus, plus_status) = convert_entity(lwpolyline(world_z(), 2.0, &verts));
    let (minus, minus_status) = convert_entity(lwpolyline(minus_z(), 2.0, &verts));
    assert_eq!(plus_status, Completeness::Complete);
    assert_eq!(minus_status, Completeness::Complete);

    let SemanticGeometry::Polyline {
        points: plus_points,
        bulges: plus_bulges,
        ..
    } = plus
    else {
        panic!("expected a polyline");
    };
    let SemanticGeometry::Polyline {
        points: minus_points,
        bulges: minus_bulges,
        ..
    } = minus
    else {
        panic!("expected a polyline");
    };
    assert_eq!(minus_points.len(), plus_points.len());
    for (p, m) in plus_points.iter().zip(&minus_points) {
        assert!((m.x + p.x).abs() < 1e-9, "+Z {p:?}, -Z {m:?}");
        assert!((m.y - p.y).abs() < 1e-9, "+Z {p:?}, -Z {m:?}");
        assert!((m.z + p.z).abs() < 1e-9, "+Z {p:?}, -Z {m:?}");
    }
    // The tangential bulge keeps its magnitude and flips sign with the mirror.
    assert!(
        (minus_bulges[0] + plus_bulges[0]).abs() < 1e-12,
        "-Z bulges {minus_bulges:?}"
    );
    assert!(
        (minus_bulges[0] + 0.5).abs() < 1e-12,
        "-Z bulges {minus_bulges:?}"
    );
}

#[test]
fn minus_z_circle_centre_is_x_mirrored_and_normal_preserved() {
    // ACIS circle centres already run through the arbitrary-axis frame, so
    // this path was correct; the test pins it after the `-Z` change.
    let mut c = acadrust::entities::Circle::new();
    c.center = acadrust::types::Vector3::new(2.0, 3.0, 5.0);
    c.normal = acadrust::types::Vector3::new(0.0, 0.0, -1.0);
    c.radius = 4.0;
    let (geometry, completeness) = convert_entity(EntityType::Circle(c));
    assert_eq!(completeness, Completeness::Complete);
    let SemanticGeometry::Circle {
        center,
        normal,
        radius,
    } = geometry
    else {
        panic!("expected a circle");
    };
    assert!((center.x + 2.0).abs() < 1e-9, "{center:?}");
    assert!((center.y - 3.0).abs() < 1e-9, "{center:?}");
    assert!((center.z + 5.0).abs() < 1e-9, "{center:?}");
    assert!((normal.z + 1.0).abs() < 1e-9, "{normal:?}");
    assert!((radius - 4.0).abs() < 1e-9);
}

#[test]
fn minus_z_text_position_and_rotation_follow_the_mirror() {
    // TEXT stores its insertion point and rotation in OCS. Under `-Z` the
    // world point mirrors `x`/`z` and a baseline angle becomes `PI - angle`.
    let mut t = acadrust::entities::Text::new();
    t.value = "mirror".into();
    t.insertion_point = acadrust::types::Vector3::new(2.0, 3.0, 4.0);
    t.height = 1.0;
    t.rotation = 0.5;
    t.normal = acadrust::types::Vector3::new(0.0, 0.0, -1.0);
    let (geometry, completeness) = convert_entity(EntityType::Text(t));
    assert_eq!(completeness, Completeness::Complete);
    let SemanticGeometry::Text {
        position, rotation, ..
    } = geometry
    else {
        panic!("expected text");
    };
    assert!((position.x + 2.0).abs() < 1e-9, "{position:?}");
    assert!((position.y - 3.0).abs() < 1e-9, "{position:?}");
    assert!((position.z + 4.0).abs() < 1e-9, "{position:?}");
    assert!(
        (rotation - (std::f64::consts::PI - 0.5)).abs() < 1e-12,
        "rotation {rotation}"
    );
}

#[test]
fn minus_z_ellipse_keeps_its_normal() {
    // The ELLIPSE's minor axis is `cross(normal, major)`, so coercing a `-Z`
    // normal to `+Z` would put the minor axis (and the sweep) on the wrong
    // side. Only a degenerate normal may default to world Z.
    let mut e = acadrust::entities::Ellipse::new();
    e.normal = acadrust::types::Vector3::new(0.0, 0.0, -1.0);
    e.center = acadrust::types::Vector3::new(1.0, 2.0, 0.0);
    e.major_axis = acadrust::types::Vector3::new(3.0, 0.0, 0.0);
    let (geometry, _) = convert_entity(EntityType::Ellipse(e));
    let SemanticGeometry::Ellipse { normal, .. } = geometry else {
        panic!("expected an ellipse");
    };
    assert!((normal.x).abs() < 1e-12, "{normal:?}");
    assert!((normal.y).abs() < 1e-12, "{normal:?}");
    assert!((normal.z + 1.0).abs() < 1e-12, "{normal:?}");
}

// ---- B22: paper-space viewport → four corners + full transform ----
