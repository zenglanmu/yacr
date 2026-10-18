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

// ---- B22: paper-space viewport → four corners + full transform ----
