//! `PYRAMID`'s typed regular n-gon base and its `Top radius` truncation.
//!
//! Split out of [`super::options`] to keep that file inside the code-size gate;
//! the fake modeler and the shared spec helpers are the parent module's.

use super::super::*;
use super::options::{run, spec_at_elev, spec_of};
use super::{app_with_modeler, entity_count, p, Answer};
use cad_kernel_adapter::PrimitiveSpec;

// ---- PYRAMID: the typed regular n-gon base ------------------------------

#[test]
fn pyramid_polygon_base_is_the_polygon_ring_at_the_session_elevation() {
    // Inscribed: the first vertex lies on +X at exactly the typed radius, so the
    // seam receives that radius and rotation 0.
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Pyramid,
            SolidPrimitiveCommand::PyramidPolygon {
                base: [2.0, 3.0],
                radius: 5.0,
                sides: 6,
                circumscribed: false,
                height: 8.0,
                top_radius: None,
            },
            7.0,
        ),
        PrimitiveSpec::PyramidNgon {
            base_center: p(2.0, 3.0, 7.0),
            radius: 5.0,
            sides: 6,
            rotation: 0.0,
            height: 8.0,
        },
        "ELEV sets the base plane; the inscribed ring's vertex radius is the typed one"
    );

    // A triangle is the smallest base: the ring is still the POLYGON one.
    let spec = spec_of(
        ExtendedCommandId::Pyramid,
        SolidPrimitiveCommand::PyramidPolygon {
            base: [0.0, 0.0],
            radius: 2.0,
            sides: 3,
            circumscribed: false,
            height: 1.0,
            top_radius: None,
        },
    );
    assert!(
        matches!(
            spec,
            PrimitiveSpec::PyramidNgon {
                radius,
                sides: 3,
                rotation,
                ..
            } if (radius - 2.0).abs() < 1e-12 && rotation.abs() < 1e-12
        ),
        "{spec:?}"
    );
}

#[test]
fn pyramid_polygon_circumscribed_uses_the_polygon_tangent_ring() {
    // The typed radius is the tangent circle; the ring's vertex radius is
    // `r / cos(π/n)` and the first vertex is half a step clockwise of +X —
    // exactly `POLYGON CIRCUMSCRIBED`'s construction.
    let spec = spec_of(
        ExtendedCommandId::Pyramid,
        SolidPrimitiveCommand::PyramidPolygon {
            base: [1.0, -1.0],
            radius: 5.0,
            sides: 6,
            circumscribed: true,
            height: 4.0,
            top_radius: None,
        },
    );
    let PrimitiveSpec::PyramidNgon {
        base_center,
        radius,
        sides,
        rotation,
        height,
    } = spec
    else {
        panic!("an n-gon pyramid spec, got {spec:?}");
    };
    assert_eq!(base_center, p(1.0, -1.0, 0.0));
    assert_eq!(sides, 6);
    assert_eq!(height, 4.0);
    let expected_radius = 5.0 / (std::f64::consts::PI / 6.0).cos();
    assert!(
        (radius - expected_radius).abs() < 1e-12,
        "vertex radius {radius} vs {expected_radius}"
    );
    assert!(
        (rotation + std::f64::consts::PI / 6.0).abs() < 1e-12,
        "the first vertex is half a step clockwise of +X, got {rotation}"
    );
}

#[test]
fn pyramid_polygon_top_radius_builds_the_truncated_spec_from_the_same_ring() {
    // Inscribed hexagon, vertex radius 5 truncated to a typed top radius of 2:
    // the top ring is the same construction at `5 · 2/5 = 2`.
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Pyramid,
            SolidPrimitiveCommand::PyramidPolygon {
                base: [2.0, 3.0],
                radius: 5.0,
                sides: 6,
                circumscribed: false,
                height: 8.0,
                top_radius: Some(2.0),
            },
            7.0,
        ),
        PrimitiveSpec::PyramidNgonFrustum {
            base_center: p(2.0, 3.0, 7.0),
            radius: 5.0,
            top_radius: 2.0,
            sides: 6,
            rotation: 0.0,
            height: 8.0,
        },
        "both rings are vertex radii of the POLYGON construction"
    );

    // Circumscribed: the typed radii are tangent circles, so both rings scale
    // by `1/cos(π/n)` and keep the polygon's half-step rotation.
    let spec = spec_of(
        ExtendedCommandId::Pyramid,
        SolidPrimitiveCommand::PyramidPolygon {
            base: [1.0, -1.0],
            radius: 5.0,
            sides: 6,
            circumscribed: true,
            height: 4.0,
            top_radius: Some(2.5),
        },
    );
    let PrimitiveSpec::PyramidNgonFrustum {
        base_center,
        radius,
        top_radius,
        sides,
        rotation,
        height,
    } = spec
    else {
        panic!("a truncated n-gon spec, got {spec:?}");
    };
    assert_eq!(base_center, p(1.0, -1.0, 0.0));
    assert_eq!(sides, 6);
    assert!((height - 4.0).abs() < 1e-12);
    let vertex_radius = 5.0 / (std::f64::consts::PI / 6.0).cos();
    assert!((radius - vertex_radius).abs() < 1e-12);
    assert!(
        (top_radius - vertex_radius / 2.0).abs() < 1e-12,
        "the top ring keeps the circumscribed scaling, got {top_radius}"
    );
    assert!((rotation + std::f64::consts::PI / 6.0).abs() < 1e-12);
}

#[test]
fn pyramid_polygon_top_radius_zero_is_the_documented_apex() {
    // AutoCAD's `Top radius` prompt defaults to 0, which *is* the apex form; the
    // command maps it onto the existing `PyramidNgon` spec rather than a
    // degenerate frustum.
    let spec = spec_of(
        ExtendedCommandId::Pyramid,
        SolidPrimitiveCommand::PyramidPolygon {
            base: [0.0, 0.0],
            radius: 2.0,
            sides: 5,
            circumscribed: false,
            height: 3.0,
            top_radius: Some(0.0),
        },
    );
    assert!(
        matches!(
            spec,
            PrimitiveSpec::PyramidNgon {
                radius,
                sides: 5,
                height,
                ..
            } if (radius - 2.0).abs() < 1e-12 && (height - 3.0).abs() < 1e-12
        ),
        "{spec:?}"
    );
}

#[test]
fn pyramid_polygon_refuses_out_of_range_input_before_the_modeler() {
    let cases = [
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 1.0,
                sides: 2,
                circumscribed: false,
                height: 1.0,
                top_radius: None,
            },
            "pyramid.sides:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 1.0,
                sides: 1025,
                circumscribed: false,
                height: 1.0,
                top_radius: None,
            },
            "pyramid.sides:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 0.0,
                sides: 4,
                circumscribed: false,
                height: 1.0,
                top_radius: None,
            },
            "pyramid.radius:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [f64::NAN, 0.0],
                radius: 1.0,
                sides: 4,
                circumscribed: false,
                height: 1.0,
                top_radius: None,
            },
            "pyramid.invalid_point:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 1.0,
                sides: 4,
                circumscribed: false,
                height: 0.0,
                top_radius: None,
            },
            "pyramid.degenerate_extent:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 1.0,
                sides: 4,
                circumscribed: false,
                height: 1.0,
                top_radius: Some(1.0),
            },
            "pyramid.equal_radii:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 1.0,
                sides: 4,
                circumscribed: false,
                height: 1.0,
                top_radius: Some(2.0),
            },
            "pyramid.top_radius:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 1.0,
                sides: 4,
                circumscribed: false,
                height: 1.0,
                top_radius: Some(-1.0),
            },
            "pyramid.top_radius:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 1.0,
                sides: 4,
                circumscribed: false,
                height: 1.0,
                top_radius: Some(f64::NAN),
            },
            "pyramid.top_radius:",
        ),
    ];
    for (input, expected) in cases {
        let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
        let before = revision(&app);
        let error = run(&mut app, &mut session, ExtendedCommandId::Pyramid, input)
            .expect_err("an out-of-range polygon base is refused");
        let CadError::InvalidInput(text) = error else {
            panic!("a degenerate construction is InvalidInput");
        };
        assert!(text.starts_with(expected), "{text}");
        assert!(specs.lock().unwrap().is_empty(), "no modeler call");
        assert_eq!(revision(&app), before);
        assert!(!app.can_undo(&DocumentId(1)));
        assert_eq!(entity_count(&app), 0);
    }
}
