//! `validate_geometry` contract tests.
//!
//! Moved out of `validate.rs` when the source file reached the 1000-line gate.

use crate::validate::{
    validate_geometry, REASON_BODY_BREP_TOLERANCE, REASON_BODY_EDGE_POLYLINE,
    REASON_BODY_SOLID_WITHOUT_EXACT, REASON_BODY_SOLID_WITH_EMPTY_BREP, REASON_BODY_TOLERANCE,
    REASON_POLYLINE_WIDTH, REASON_TEXT_LINE_SPACING, REASON_TEXT_WIDTH_FACTOR,
};
use cad_domain::*;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

#[test]
fn rejects_non_finite_and_degenerate_geometry() {
    assert!(validate_geometry(&SemanticGeometry::Line {
        start: p(0.0, 0.0),
        end: p(1.0, 0.0)
    })
    .is_ok());
    assert!(validate_geometry(&SemanticGeometry::Line {
        start: p(0.0, 0.0),
        end: p(0.0, 0.0)
    })
    .is_err());
    assert!(validate_geometry(&SemanticGeometry::Line {
        start: p(f64::NAN, 0.0),
        end: p(1.0, 0.0)
    })
    .is_err());
    assert!(validate_geometry(&SemanticGeometry::Circle {
        center: p(0.0, 0.0),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0
        },
        radius: 0.0
    })
    .is_err());
    assert!(validate_geometry(&SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0)],
        bulges: Vec::new(),
        closed: false,
        width: 0.0,
    })
    .is_err());
    // A stored wrap boundary must be usable; `None` means "no wrap".
    let text = |wrap_width| SemanticGeometry::Text {
        text: "A".into(),
        position: p(0.0, 0.0),
        style: cad_domain::StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
    };
    assert!(validate_geometry(&text(None)).is_ok());
    assert!(validate_geometry(&text(Some(4.0))).is_ok());
    assert!(validate_geometry(&text(Some(0.0))).is_err());
    assert!(validate_geometry(&text(Some(-1.0))).is_err());
    assert!(validate_geometry(&text(Some(f64::NAN))).is_err());
}

#[test]
fn text_width_factor_must_be_positive_and_finite() {
    let text = |width_factor| SemanticGeometry::Text {
        text: "A".into(),
        position: p(0.0, 0.0),
        style: cad_domain::StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width: None,
        width_factor,
        line_spacing: cad_domain::TextLineSpacing::default(),
    };
    // The neutral factor and a genuine stretch are both representable.
    assert!(validate_geometry(&text(1.0)).is_ok());
    assert!(validate_geometry(&text(2.5)).is_ok());
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let error = validate_geometry(&text(bad)).expect_err("rejected");
        let message = error.to_string();
        assert!(
            message.contains(REASON_TEXT_WIDTH_FACTOR),
            "the refusal must carry the stable reason: {message}"
        );
    }
}

#[test]
fn text_line_spacing_factor_must_be_positive_and_finite() {
    let text = |line_spacing| SemanticGeometry::Text {
        text: "A".into(),
        position: p(0.0, 0.0),
        style: cad_domain::StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing,
    };
    // The neutral spacing, a compressed one and a stretched one with the
    // "exactly" style are all representable.
    assert!(validate_geometry(&text(cad_domain::TextLineSpacing::default())).is_ok());
    assert!(validate_geometry(&text(cad_domain::TextLineSpacing {
        factor: 0.5,
        style: cad_domain::TextLineSpacingStyle::Exactly,
    }))
    .is_ok());
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let error = validate_geometry(&text(cad_domain::TextLineSpacing {
            factor: bad,
            style: cad_domain::TextLineSpacingStyle::AtLeast,
        }))
        .expect_err("rejected");
        let message = error.to_string();
        assert!(
            message.contains(REASON_TEXT_LINE_SPACING),
            "the refusal must carry the stable reason: {message}"
        );
    }
}

#[test]
fn polyline_width_defaults_to_hairline_and_rejects_bad_values() {
    let polyline = |width| SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0), p(1.0, 0.0)],
        bulges: vec![0.0, 0.0],
        closed: false,
        width,
    };
    // `0.0` is the hairline default; a positive uniform width is a real
    // stroke and both are representable.
    assert!(validate_geometry(&polyline(0.0)).is_ok());
    assert!(validate_geometry(&polyline(0.75)).is_ok());
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        let error = validate_geometry(&polyline(bad)).expect_err("rejected");
        let message = error.to_string();
        assert!(
            message.contains(REASON_POLYLINE_WIDTH),
            "the refusal must carry the stable reason: {message}"
        );
    }
}

#[test]
fn accepts_well_formed_image_and_mask() {
    let image = SemanticGeometry::Image {
        origin: p(1.0, 2.0),
        u: p(0.5, 0.0),
        v: p(0.0, 0.25),
        pixels: [640.0, 480.0],
        file: None,
        clip: None,
        visible: true,
        adjust: ImageAdjust::default(),
    };
    assert!(validate_geometry(&image).is_ok());

    let mask = SemanticGeometry::Mask {
        boundary: vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)],
        inverted: false,
    };
    assert!(validate_geometry(&mask).is_ok());
}

#[test]
fn rejects_malformed_image_and_mask() {
    // An image with non-positive pixel dimensions cannot be placed.
    assert!(validate_geometry(&SemanticGeometry::Image {
        origin: p(0.0, 0.0),
        u: p(1.0, 0.0),
        v: p(0.0, 1.0),
        pixels: [0.0, 10.0],
        file: None,
        clip: None,
        visible: true,
        adjust: ImageAdjust::default(),
    })
    .is_err());
    // A non-finite edge vector is rejected.
    assert!(validate_geometry(&SemanticGeometry::Image {
        origin: p(0.0, 0.0),
        u: p(f64::NAN, 0.0),
        v: p(0.0, 1.0),
        pixels: [10.0, 10.0],
        file: None,
        clip: None,
        visible: true,
        adjust: ImageAdjust::default(),
    })
    .is_err());
    // A mask needs at least three points to bound an area.
    assert!(validate_geometry(&SemanticGeometry::Mask {
        boundary: vec![p(0.0, 0.0), p(1.0, 1.0)],
        inverted: false,
    })
    .is_err());
}

#[test]
fn rejects_out_of_range_image_adjustment_and_accepts_the_edges() {
    let image_with = |adjust: ImageAdjust| SemanticGeometry::Image {
        origin: p(0.0, 0.0),
        u: p(1.0, 0.0),
        v: p(0.0, 1.0),
        pixels: [10.0, 10.0],
        file: None,
        clip: None,
        visible: true,
        adjust,
    };
    // 0 and 100 are the real DXF percentage bounds.
    for adjust in [
        ImageAdjust {
            brightness: 0,
            contrast: 100,
            fade: 0,
            monochrome: true,
        },
        ImageAdjust {
            brightness: 100,
            contrast: 0,
            fade: 100,
            monochrome: false,
        },
    ] {
        assert!(validate_geometry(&image_with(adjust)).is_ok(), "{adjust:?}");
    }
    // `u8` can hold >100; validation rejects it instead of clamping.
    for adjust in [
        ImageAdjust {
            brightness: 101,
            ..ImageAdjust::default()
        },
        ImageAdjust {
            contrast: 200,
            ..ImageAdjust::default()
        },
        ImageAdjust {
            fade: 255,
            ..ImageAdjust::default()
        },
    ] {
        assert!(
            validate_geometry(&image_with(adjust)).is_err(),
            "{adjust:?} must be rejected"
        );
    }
}

// ---------------------------------------------------------------------------
// `SemanticGeometry::Body3d` contract tests.
//
// The fixtures are deliberately small: one planar face with a square boundary
// loop is enough to exercise "exact geometry exists" and "the solid rules see a
// non-empty B-rep"; the display side is one triangle plus one edge polyline.
// ---------------------------------------------------------------------------

fn square_loop() -> BrepLoop {
    let corners = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
    BrepLoop {
        edges: (0..4)
            .map(|i| BrepCurve::Line {
                start: corners[i],
                end: corners[(i + 1) % 4],
            })
            .collect(),
    }
}

/// A one-face plane B-rep (an exact `Sheet`).
fn plane_brep() -> BrepData {
    BrepData {
        shells: vec![BrepShell {
            id: 0,
            faces: vec![BrepFace {
                id: 0,
                surface: BrepSurface::Plane {
                    origin: p(0.0, 0.0),
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
                loops: vec![square_loop()],
                tolerance: None,
                domain: None,
            }],
        }],
        placement: None,
        tolerance: None,
    }
}

fn triangle_mesh() -> Mesh {
    Mesh {
        vertices: vec![p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

fn sheet_body() -> Body3d {
    Body3d {
        kind: BodyKind::Sheet,
        exact: BodyExact::Exact(Box::new(plane_brep())),
        display: BodyDisplay {
            mesh: triangle_mesh(),
            edges: vec![vec![p(0.0, 0.0), p(10.0, 0.0)]],
            tolerance: Some(0.1),
            precision: Precision::Analytic,
            completeness: Completeness::Complete,
        },
    }
}

fn body_geometry(body: Body3d) -> SemanticGeometry {
    SemanticGeometry::Body3d(Box::new(body))
}

#[test]
fn body_sheet_with_exact_face_and_display_passes() {
    assert!(validate_geometry(&body_geometry(sheet_body())).is_ok());
}

#[test]
fn body_solid_requires_an_exact_brep() {
    let mut body = sheet_body();
    body.kind = BodyKind::Solid;
    body.exact = BodyExact::Unavailable {
        reason: "synthetic: no exact body".into(),
    };
    let message = validate_geometry(&body_geometry(body))
        .expect_err("a solid without exact geometry is rejected")
        .to_string();
    assert!(
        message.contains(REASON_BODY_SOLID_WITHOUT_EXACT),
        "the refusal must carry the stable reason: {message}"
    );
}

#[test]
fn body_solid_with_an_empty_brep_is_rejected() {
    let mut body = sheet_body();
    body.kind = BodyKind::Solid;
    body.exact = BodyExact::Exact(Box::default());
    let message = validate_geometry(&body_geometry(body))
        .expect_err("an empty exact B-rep is not a solid")
        .to_string();
    assert!(
        message.contains(REASON_BODY_SOLID_WITH_EMPTY_BREP),
        "the refusal must carry the stable reason: {message}"
    );
}

#[test]
fn body_display_edges_and_tolerance_are_validated() {
    let body_with = |edges: Vec<Vec<Point3>>, tolerance: Option<f64>| {
        let mut body = sheet_body();
        body.display.edges = edges;
        body.display.tolerance = tolerance;
        body_geometry(body)
    };
    // Baseline: one valid polyline and a positive tolerance.
    assert!(validate_geometry(&body_with(vec![vec![p(0.0, 0.0), p(1.0, 1.0)]], Some(0.5))).is_ok());

    let single_point = validate_geometry(&body_with(vec![vec![p(0.0, 0.0)]], Some(0.5)))
        .expect_err("a one-point polyline is not an edge")
        .to_string();
    assert!(
        single_point.contains(REASON_BODY_EDGE_POLYLINE),
        "the refusal must name the edge rule: {single_point}"
    );

    let non_finite = validate_geometry(&body_with(
        vec![vec![p(0.0, 0.0), p(f64::NAN, 0.0)]],
        Some(0.5),
    ))
    .expect_err("a non-finite edge point is not storable")
    .to_string();
    assert!(
        non_finite.contains(REASON_BODY_EDGE_POLYLINE),
        "the refusal must name the edge rule: {non_finite}"
    );

    for bad in [Some(0.0), Some(-1.0), Some(f64::NAN), Some(f64::INFINITY)] {
        let message = validate_geometry(&body_with(vec![vec![p(0.0, 0.0), p(1.0, 1.0)]], bad))
            .expect_err("the tolerance must be finite and positive")
            .to_string();
        assert!(
            message.contains(REASON_BODY_TOLERANCE),
            "the refusal must name the tolerance rule: {message}"
        );
    }
    // `None` means "not known" and is representable.
    assert!(validate_geometry(&body_with(vec![vec![p(0.0, 0.0), p(1.0, 1.0)]], None)).is_ok());
}

#[test]
fn body_brep_tolerance_must_be_a_usable_length_when_stated() {
    let with_body_tolerance = |tolerance: Option<f64>| {
        let mut brep = plane_brep();
        brep.tolerance = tolerance;
        let mut body = sheet_body();
        body.exact = BodyExact::Exact(Box::new(brep));
        body_geometry(body)
    };
    // `None` is "the source stated none" and is representable; a stated tolerance
    // must be a positive finite length.
    assert!(validate_geometry(&with_body_tolerance(None)).is_ok());
    assert!(validate_geometry(&with_body_tolerance(Some(1e-7))).is_ok());
    for bad in [Some(0.0), Some(-1e-6), Some(f64::NAN), Some(f64::INFINITY)] {
        let message = validate_geometry(&with_body_tolerance(bad))
            .expect_err("the B-rep tolerance must be finite and positive")
            .to_string();
        assert!(
            message.contains(REASON_BODY_BREP_TOLERANCE),
            "the refusal must name the B-rep tolerance rule: {message}"
        );
    }

    // The same rule for a face, which the record cannot even carry today: a
    // producer that states one must state a usable length.
    let mut brep = plane_brep();
    brep.shells[0].faces[0].tolerance = Some(0.0);
    let mut body = sheet_body();
    body.exact = BodyExact::Exact(Box::new(brep));
    let message = validate_geometry(&body_geometry(body))
        .expect_err("a face tolerance of zero is not a length")
        .to_string();
    assert!(
        message.contains(REASON_BODY_BREP_TOLERANCE),
        "the refusal must name the B-rep tolerance rule: {message}"
    );
}

#[test]
fn body_bounds_include_edges_when_the_mesh_is_empty() {
    let mut body = sheet_body();
    body.display.mesh = Mesh::default();
    body.display.edges = vec![vec![p(-2.0, 3.0), p(4.0, -1.0)]];
    let mut bounds = crate::BoundsAccumulator::new();
    bounds.add_geometry(&body_geometry(body));
    let (min, max) = bounds.finish().expect("edges bound the body");
    assert_eq!(
        min,
        Point3 {
            x: -2.0,
            y: -1.0,
            z: 0.0
        }
    );
    assert_eq!(
        max,
        Point3 {
            x: 4.0,
            y: 3.0,
            z: 0.0
        }
    );
}

#[test]
fn body_uniform_scale_transforms_display_and_keeps_exact() {
    let moved = crate::transform_geometry(&body_geometry(sheet_body()), &Transform3::scale(2.0))
        .expect("a uniform scale is representable");
    let SemanticGeometry::Body3d(moved) = moved else {
        panic!("expected a body");
    };
    assert!(moved.is_exact(), "a similarity keeps the exact B-rep");
    assert_eq!(moved.display.mesh.vertices[1], p(2.0, 0.0));
    assert_eq!(moved.display.edges[0][1], p(20.0, 0.0));
    let placement = moved
        .exact_brep()
        .expect("exact B-rep")
        .placement
        .expect("the composed placement is stored");
    assert!((placement.scale - 2.0).abs() < 1e-12);
    // The placement and the display agree: the source corner (10, 0, 0) maps to
    // the same world point the edge polyline now ends at.
    let corner = placement.apply(p(10.0, 0.0));
    assert!((corner.x - 20.0).abs() < 1e-9, "{corner:?}");
}

#[test]
fn body_brep_tolerance_stays_a_model_length_under_a_similarity() {
    let mut brep = plane_brep();
    brep.tolerance = Some(0.5);
    brep.shells[0].faces[0].tolerance = Some(0.25);
    let mut body = sheet_body();
    body.exact = BodyExact::Exact(Box::new(brep));
    let moved = crate::transform_geometry(&body_geometry(body), &Transform3::scale(2.0))
        .expect("a uniform scale is representable");
    let SemanticGeometry::Body3d(moved) = moved else {
        panic!("expected a body");
    };
    let brep = moved.exact_brep().expect("exact B-rep");
    // A B-rep tolerance is in **model** units, and a transform only composes the
    // placement: the shell coordinates are untouched, so both tolerances — body
    // and face — stay exactly as they were. Scaling them here as well would scale
    // them twice, because baking the composed placement into a record scales the
    // geometry *and* the tolerances by the placement's factor (pinned by
    // `cad-kernel-solidcore`'s `baking_a_similarity_scales_body_and_face_tolerances_as_lengths`).
    assert_eq!(brep.tolerance, Some(0.5));
    assert_eq!(brep.shells[0].faces[0].tolerance, Some(0.25));
    assert!(
        (brep.placement.expect("composed placement").scale - 2.0).abs() < 1e-12,
        "the scale lives in the placement, not in the tolerance"
    );

    // `None` (unspecified) stays unspecified — never filled in from the scale.
    let mut instant = plane_brep();
    instant.tolerance = None;
    let mut body = sheet_body();
    body.exact = BodyExact::Exact(Box::new(instant));
    let moved = crate::transform_geometry(&body_geometry(body), &Transform3::scale(3.0))
        .expect("a uniform scale is representable");
    let SemanticGeometry::Body3d(moved) = moved else {
        panic!("expected a body");
    };
    assert_eq!(moved.exact_brep().expect("exact").tolerance, None);
}

#[test]
fn body_non_uniform_scale_keeps_display_and_demotes_exact_and_solid_kind() {
    let mut body = sheet_body();
    body.kind = BodyKind::Solid;
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = 2.0;
    matrix[1][1] = 3.0;
    let moved = crate::transform_geometry(&body_geometry(body), &Transform3 { matrix })
        .expect("the display is still transformable");
    let SemanticGeometry::Body3d(moved) = moved else {
        panic!("expected a body");
    };
    assert!(matches!(
        &moved.exact,
        BodyExact::Unavailable { reason } if reason == "body.nonuniform_scale"
    ));
    assert_eq!(moved.kind, BodyKind::Unknown);
    // The display cache is still the transformed one, not the source.
    assert_eq!(moved.display.mesh.vertices[1], p(2.0, 0.0));
    assert_eq!(moved.display.edges[0][1], p(20.0, 0.0));
    assert_eq!(
        moved.display.tolerance, None,
        "a direction-dependent deflection is recorded as unknown"
    );
}

#[test]
fn body_face_sources_must_be_parallel_to_its_triangles() {
    use cad_domain::{Revision, SubElementId};

    let source = |key: &str| SubElementId {
        source_key: key.into(),
        topology_revision: Revision(0),
    };

    // Empty is "not face-addressable" and stays valid.
    let mut body = sheet_body();
    assert!(body.display.mesh.face_sources.is_empty());
    assert!(validate_geometry(&body_geometry(body.clone())).is_ok());

    // One B-rep face source per triangle is the populated form: accepted, and
    // the id itself stays opaque to the database.
    body.display.mesh.face_sources = vec![Some(source("brep:face:0/3"))];
    assert!(validate_geometry(&body_geometry(body.clone())).is_ok());

    // A source list that is not parallel to the triangles is unrepresentable.
    body.display.mesh.face_sources = vec![Some(source("brep:face:0/3")), None];
    let message = validate_geometry(&body_geometry(body))
        .expect_err("a mismatched face-source list must be rejected")
        .to_string();
    assert!(
        message.contains("face sources"),
        "the refusal must name the face-source rule: {message}"
    );
}
