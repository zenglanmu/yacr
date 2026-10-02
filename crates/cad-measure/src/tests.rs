//! Unit tests.

use super::*;

use cad_db::MeasurementAlgorithm;
use cad_domain::*;

fn engine() -> MeasurementEngine {
    MeasurementEngine::default()
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn z_plane() -> WorkPlane {
    WorkPlane {
        origin: p(0.0, 0.0, 0.0),
        u: p(1.0, 0.0, 0.0),
        v: p(0.0, 1.0, 0.0),
    }
}

fn request(algorithm: MeasurementAlgorithm, points: Vec<Point3>) -> MeasurementRequest {
    MeasurementRequest {
        algorithm,
        points,
        tapped: Vec::new(),
        space: MeasurementSpace::World3d,
        units: UnitContext::drawing_units(),
        source: GeometrySource::Analytic,
        precision: Precision::Analytic,
    }
}

fn plane_request(algorithm: MeasurementAlgorithm, points: Vec<Point3>) -> MeasurementRequest {
    let mut r = request(algorithm, points);
    r.space = MeasurementSpace::Plane(z_plane());
    r
}

fn selection() -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath(Vec::new()),
        sub_element: None,
    }
}

fn candidate(kind: SnapKind, point: Point3, space: SpaceId) -> SnapCandidate {
    SnapCandidate {
        kind,
        point,
        space,
        source: selection(),
        secondary: None,
        precision: Precision::Analytic,
        logical_pixel_distance: 0.0,
    }
}

// ---- planar space policy (B24) -----------------------------------------

#[test]
fn distance_2d_requires_an_explicit_plane() {
    let r = engine().measure(&request(
        MeasurementAlgorithm::Distance2d,
        vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
    ));
    assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
}

#[test]
fn polyline_length_requires_an_explicit_plane() {
    let r = engine().measure(&request(
        MeasurementAlgorithm::PolylineLength,
        vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
    ));
    assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
}

#[test]
fn planar_area_requires_an_explicit_plane() {
    let r = engine().measure(&request(
        MeasurementAlgorithm::PlanarPolygonArea,
        vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
    ));
    assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
}

#[test]
fn distance_2d_in_plane_matches_planar_projection() {
    // A point off the plane contributes no in-plane distance.
    let r = engine()
        .measure(&plane_request(
            MeasurementAlgorithm::Distance2d,
            vec![p(0.0, 0.0, 0.0), p(3.0, 4.0, 100.0)],
        ))
        .unwrap();
    assert!((r.value - 5.0).abs() < 1e-12);
}

// ---- work-plane-aware area (B24) ---------------------------------------

#[test]
fn area_on_tilted_work_plane_is_projected() {
    // A 2x1 rectangle in the XZ plane, measured on a work plane that *is*
    // that XZ plane. Projection must recover 2.0, and the ring is coplanar.
    let mut req = plane_request(
        MeasurementAlgorithm::PlanarPolygonArea,
        vec![
            p(0.0, 0.0, 0.0),
            p(2.0, 0.0, 0.0),
            p(2.0, 0.0, 1.0),
            p(0.0, 0.0, 1.0),
        ],
    );
    req.space = MeasurementSpace::Plane(WorkPlane {
        origin: p(0.0, 0.0, 0.0),
        u: p(1.0, 0.0, 0.0),
        v: p(0.0, 0.0, 1.0),
    });
    let r = engine().measure(&req).unwrap();
    assert!((r.value - 2.0).abs() < 1e-12, "got {}", r.value);
}

#[test]
fn area_rejects_ring_off_the_measurement_plane() {
    // Three of four corners lie on the z=0 work plane, one is lifted. The
    // old code flattened z to 0 and returned a valid-looking 1.0.
    let r = engine().measure(&plane_request(
        MeasurementAlgorithm::PlanarPolygonArea,
        vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 5.0),
            p(0.0, 1.0, 0.0),
        ],
    ));
    assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
    let msg = format!("{}", r.unwrap_err());
    assert!(msg.contains("coplanar"), "{msg}");
}

#[test]
fn area_rejects_skewed_work_plane() {
    let mut req = plane_request(
        MeasurementAlgorithm::PlanarPolygonArea,
        vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
    );
    req.space = MeasurementSpace::Plane(WorkPlane {
        origin: p(0.0, 0.0, 0.0),
        u: p(1.0, 0.0, 0.0),
        v: p(1.0, 1.0, 0.0), // 45°, not orthogonal
    });
    let r = engine().measure(&req);
    assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
    let msg = format!("{}", r.unwrap_err());
    assert!(msg.contains("skewed"), "{msg}");
}

#[test]
fn area_rejects_degenerate_work_plane() {
    let mut req = plane_request(
        MeasurementAlgorithm::PlanarPolygonArea,
        vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
    );
    req.space = MeasurementSpace::Plane(WorkPlane {
        origin: p(0.0, 0.0, 0.0),
        u: p(1.0, 0.0, 0.0),
        v: p(2.0, 0.0, 0.0), // parallel to u
    });
    let r = engine().measure(&req);
    assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
}

#[test]
fn area_rejects_non_finite_work_plane() {
    let mut req = plane_request(
        MeasurementAlgorithm::PlanarPolygonArea,
        vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
    );
    req.space = MeasurementSpace::Plane(WorkPlane {
        origin: p(0.0, 0.0, 0.0),
        u: p(f64::NAN, 0.0, 0.0),
        v: p(0.0, 1.0, 0.0),
    });
    let r = engine().measure(&req);
    assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
}

#[test]
fn area_rejects_extreme_finite_coordinates_without_overflowing() {
    // Large but finite coordinates; the shoelace products overflow to inf,
    // which must surface as an explicit error, never a bogus number.
    let big = 1e200;
    let r = engine().measure(&plane_request(
        MeasurementAlgorithm::PlanarPolygonArea,
        vec![
            p(big, 0.0, 0.0),
            p(big, big, 0.0),
            p(0.0, big, 0.0),
            p(0.0, 0.0, 0.0),
        ],
    ));
    assert!(r.is_err(), "expected overflow rejection, got {r:?}");
}

// ---- space constraints (B24 / F04) -------------------------------------

#[test]
fn distance_3d_in_paper_space_is_refused() {
    let mut req = request(
        MeasurementAlgorithm::Distance3d,
        vec![p(0.0, 0.0, 0.0), p(3.0, 4.0, 0.0)],
    );
    req.space = MeasurementSpace::Paper(LayoutId(1));
    assert!(matches!(
        engine().measure(&req),
        Err(CadError::Unsupported(_))
    ));
}

#[test]
fn angle_in_paper_space_is_refused() {
    let mut req = request(
        MeasurementAlgorithm::Angle3Points,
        vec![p(1.0, 0.0, 0.0), p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
    );
    req.space = MeasurementSpace::Paper(LayoutId(1));
    assert!(matches!(
        engine().measure(&req),
        Err(CadError::Unsupported(_))
    ));
}

#[test]
fn mixing_spaces_in_one_measurement_is_refused() {
    let tapped = vec![
        MeasurementPoint::model(p(0.0, 0.0, 0.0)),
        MeasurementPoint::paper(p(3.0, 4.0, 0.0), LayoutId(1)),
    ];
    let req = MeasurementRequest::from_tapped(
        MeasurementAlgorithm::Distance3d,
        tapped,
        MeasurementSpace::World3d,
        UnitContext::drawing_units(),
        GeometrySource::UserPoints,
        Precision::Analytic,
    );
    let r = engine().measure(&req);
    assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
    assert!(format!("{}", r.unwrap_err()).contains("mixes"));
}

#[test]
fn model_space_cannot_consume_paper_points() {
    let tapped = vec![
        MeasurementPoint::paper(p(0.0, 0.0, 0.0), LayoutId(1)),
        MeasurementPoint::paper(p(3.0, 4.0, 0.0), LayoutId(1)),
    ];
    let req = MeasurementRequest::from_tapped(
        MeasurementAlgorithm::Distance3d,
        tapped,
        MeasurementSpace::World3d,
        UnitContext::drawing_units(),
        GeometrySource::UserPoints,
        Precision::Analytic,
    );
    let r = engine().measure(&req);
    assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
}

#[test]
fn consistent_model_points_are_accepted() {
    let tapped = vec![
        MeasurementPoint::model(p(0.0, 0.0, 0.0)),
        MeasurementPoint::model(p(3.0, 4.0, 0.0)),
    ];
    let req = MeasurementRequest::from_tapped(
        MeasurementAlgorithm::Distance3d,
        tapped,
        MeasurementSpace::World3d,
        UnitContext::drawing_units(),
        GeometrySource::UserPoints,
        Precision::Analytic,
    );
    let r = engine().measure(&req).unwrap();
    assert!((r.value - 5.0).abs() < 1e-12);
}

// ---- snapping (B24) ----------------------------------------------------

#[test]
fn snap_ignores_candidates_behind_the_ray_origin() {
    let ray = Ray3 {
        origin: p(0.0, 0.0, 0.0),
        direction: p(1.0, 0.0, 0.0),
    };
    let behind = candidate(SnapKind::Endpoint, p(-0.5, 0.0, 0.0), SpaceId::Model);
    let front = candidate(SnapKind::Endpoint, p(0.25, 0.0, 0.0), SpaceId::Model);
    let hit = engine().snap_to_points(&[behind, front], &ray, 1.0, None);
    assert_eq!(hit.map(|c| c.point.x), Some(0.25));
}

#[test]
fn snap_rejects_non_unit_ray_direction() {
    let ray = Ray3 {
        origin: p(0.0, 0.0, 0.0),
        direction: p(0.0, 0.0, 0.0),
    };
    let c = candidate(SnapKind::Endpoint, p(0.0, 0.0, 0.0), SpaceId::Model);
    assert!(engine().snap_to_points(&[c], &ray, 1.0, None).is_none());
}

#[test]
fn snap_returns_original_logical_distance_and_kind() {
    let ray = Ray3 {
        origin: p(0.0, 0.0, 0.0),
        direction: p(2.0, 0.0, 0.0), // not unit; engine normalises
    };
    let c = candidate(SnapKind::Midpoint, p(1.0, 0.2, 0.0), SpaceId::Model);
    let hit = engine()
        .snap_to_points(&[c], &ray, 0.1, None)
        .expect("midpoint within tolerance");
    assert!((hit.logical_pixel_distance - 2.0).abs() < 1e-9);
}

#[test]
fn snap_filters_by_space() {
    let ray = Ray3 {
        origin: p(0.0, 0.0, 0.0),
        direction: p(1.0, 0.0, 0.0),
    };
    let model = candidate(SnapKind::Endpoint, p(1.0, 0.0, 0.0), SpaceId::Model);
    let paper = candidate(
        SnapKind::Endpoint,
        p(0.5, 0.0, 0.0),
        SpaceId::Paper(LayoutId(1)),
    );
    let hit = engine()
        .snap_to_points(&[model, paper], &ray, 1.0, Some(SpaceId::Model))
        .expect("model candidate");
    assert_eq!(hit.space, SpaceId::Model);
    assert_eq!(hit.point.x, 1.0);
}

#[test]
fn snap_rejects_non_positive_world_per_px() {
    let ray = Ray3 {
        origin: p(0.0, 0.0, 0.0),
        direction: p(1.0, 0.0, 0.0),
    };
    let c = candidate(SnapKind::Endpoint, p(1.0, 0.0, 0.0), SpaceId::Model);
    assert!(engine().snap_to_points(&[c], &ray, 0.0, None).is_none());
}

// ---- pre-existing behaviour preserved ----------------------------------

#[test]
fn distance_3d_is_euclidean() {
    let r = engine()
        .measure(&request(
            MeasurementAlgorithm::Distance3d,
            vec![p(0.0, 0.0, 0.0), p(3.0, 4.0, 0.0)],
        ))
        .unwrap();
    assert!((r.value - 5.0).abs() < 1e-12);
}

#[test]
fn angle_three_points_is_degrees() {
    let r = engine()
        .measure(&request(
            MeasurementAlgorithm::Angle3Points,
            vec![p(1.0, 0.0, 0.0), p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        ))
        .unwrap();
    assert!((r.value - 90.0).abs() < 1e-9);
}

#[test]
fn non_finite_points_are_rejected() {
    let r = engine().measure(&request(
        MeasurementAlgorithm::Distance3d,
        vec![p(f64::NAN, 0.0, 0.0), p(0.0, 0.0, 0.0)],
    ));
    assert!(r.is_err());
}

#[test]
fn polygon_area_rejects_self_intersection() {
    let bowtie = vec![
        p(0.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(0.0, 1.0, 0.0),
    ];
    let r = engine().measure(&plane_request(
        MeasurementAlgorithm::PlanarPolygonArea,
        bowtie,
    ));
    assert!(r.is_err());
}

// ---- paper vs viewport-model measurement (F04/F06) ---------------------

#[test]
fn paper_distance_is_measured_directly_on_the_sheet() {
    let mut req = request(
        MeasurementAlgorithm::Distance2d,
        vec![p(0.0, 0.0, 0.0), p(3.0, 4.0, 0.0)],
    );
    req.space = MeasurementSpace::Paper(LayoutId(1));
    let record = engine().measure(&req).unwrap();
    assert!((record.value - 5.0).abs() < 1e-12);
    assert!(record.plane.is_some());
    // Paper inputs stay in paper coordinates.
    assert_eq!(record.inputs[0], p(0.0, 0.0, 0.0));
}

#[test]
fn paper_area_is_planar_on_the_sheet() {
    let mut req = request(
        MeasurementAlgorithm::PlanarPolygonArea,
        vec![
            p(0.0, 0.0, 0.0),
            p(2.0, 0.0, 0.0),
            p(2.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
        ],
    );
    req.space = MeasurementSpace::Paper(LayoutId(1));
    let record = engine().measure(&req).unwrap();
    assert!((record.value - 2.0).abs() < 1e-12);
}

#[test]
fn viewport_model_distance_uses_the_verified_inverse() {
    // 1:100: one paper unit is 100 model units, so the paper→model inverse
    // scales by 100.
    let inverse = Transform3::scale(100.0);
    let mut req = request(
        MeasurementAlgorithm::Distance2d,
        vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
    );
    req.space = MeasurementSpace::ViewportModel {
        layout: LayoutId(1),
        inverse,
    };
    let record = engine().measure(&req).unwrap();
    assert!((record.value - 100.0).abs() < 1e-9, "got {}", record.value);
    // The recorded input is a model point, not a paper pixel.
    assert_eq!(record.inputs[1], p(100.0, 0.0, 0.0));
}

#[test]
fn paper_and_viewport_measurements_are_distinguishable() {
    let paper = {
        let mut req = request(
            MeasurementAlgorithm::Distance2d,
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
        );
        req.space = MeasurementSpace::Paper(LayoutId(1));
        engine().measure(&req).unwrap()
    };
    let model = {
        let mut req = request(
            MeasurementAlgorithm::Distance2d,
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
        );
        req.space = MeasurementSpace::ViewportModel {
            layout: LayoutId(1),
            inverse: Transform3::scale(100.0),
        };
        engine().measure(&req).unwrap()
    };
    assert!((paper.value - 1.0).abs() < 1e-12);
    assert!((model.value - 100.0).abs() < 1e-9);
    assert_ne!(paper.value, model.value);
    // Paper records paper coordinates, model records model coordinates.
    assert_eq!(paper.inputs[1], p(1.0, 0.0, 0.0));
    assert_eq!(model.inputs[1], p(100.0, 0.0, 0.0));
    assert!(paper.plane.is_some() && model.plane.is_some());
}

#[test]
fn viewport_model_without_a_valid_inverse_is_disabled() {
    let mut singular = Transform3::scale(100.0).matrix;
    singular[0][0] = 0.0;
    for inverse in [Transform3::scale(0.0), Transform3 { matrix: singular }] {
        let mut req = request(
            MeasurementAlgorithm::Distance2d,
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
        );
        req.space = MeasurementSpace::ViewportModel {
            layout: LayoutId(1),
            inverse,
        };
        assert!(
            matches!(engine().measure(&req), Err(CadError::Unsupported(_))),
            "singular inverse {inverse:?} must be refused, not guessed"
        );
    }
    // A non-finite inverse is refused too.
    let mut m = Transform3::scale(100.0).matrix;
    m[0][0] = f64::NAN;
    let mut req = request(
        MeasurementAlgorithm::Distance2d,
        vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
    );
    req.space = MeasurementSpace::ViewportModel {
        layout: LayoutId(1),
        inverse: Transform3 { matrix: m },
    };
    assert!(matches!(
        engine().measure(&req),
        Err(CadError::Unsupported(_))
    ));
}

#[test]
fn viewport_model_area_rejects_a_non_coplanar_ring() {
    let mut req = request(
        MeasurementAlgorithm::PlanarPolygonArea,
        vec![
            p(0.0, 0.0, 0.0),
            p(2.0, 0.0, 0.0),
            p(2.0, 1.0, 0.0),
            p(0.0, 1.0, 5.0),
        ],
    );
    req.space = MeasurementSpace::ViewportModel {
        layout: LayoutId(1),
        inverse: Transform3::scale(100.0),
    };
    assert!(matches!(
        engine().measure(&req),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn viewport_model_angle_is_defined_in_model_space() {
    let mut req = request(
        MeasurementAlgorithm::Angle3Points,
        vec![p(1.0, 0.0, 0.0), p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
    );
    req.space = MeasurementSpace::ViewportModel {
        layout: LayoutId(1),
        inverse: Transform3::scale(100.0),
    };
    let record = engine().measure(&req).unwrap();
    assert!((record.value - 90.0).abs() < 1e-9);
}

#[test]
fn unknown_units_display_drawing_units_and_record_the_source() {
    let mut req = request(
        MeasurementAlgorithm::Distance3d,
        vec![p(0.0, 0.0, 0.0), p(3.0, 4.0, 0.0)],
    );
    req.units = UnitContext::drawing_units();
    let record = engine().measure(&req).unwrap();
    assert_eq!(record.units.label(), "drawing units");
    assert_eq!(record.units.source, Unit::DrawingUnits);
    assert_eq!(record.units.display, Unit::DrawingUnits);

    // A known display unit keeps its source unit on the record.
    req.units = UnitContext {
        source: Unit::Millimeter,
        display: Unit::Inch,
        display_per_source: Some(0.039_370_078_7),
        decimal_places: 3,
    };
    let record = engine().measure(&req).unwrap();
    assert_eq!(record.units.source, Unit::Millimeter);
    assert_eq!(record.units.label(), "in");
}
