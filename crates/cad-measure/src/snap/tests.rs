//! Contract tests for the snap engine (moved verbatim from `snap.rs`).

use super::*;
use crate::{MeasurementEngine, MeasurementPoint, MeasurementRequest, MeasurementSpace};
use cad_db::MeasurementAlgorithm;

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn flat(x: f64, y: f64) -> Point3 {
    p(x, y, 0.0)
}

fn z_plane() -> WorkPlane {
    WorkPlane {
        origin: p(0.0, 0.0, 0.0),
        u: p(1.0, 0.0, 0.0),
        v: p(0.0, 1.0, 0.0),
    }
}

fn source(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath(Vec::new()),
        sub_element: None,
    }
}

fn target(geometry: SemanticGeometry) -> SnapTarget {
    SnapTarget::new(geometry, source(1), SpaceId::Model, Precision::Analytic)
}

fn line(a: (f64, f64), b: (f64, f64)) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: flat(a.0, a.1),
        end: flat(b.0, b.1),
    }
}

fn circle(center: (f64, f64), radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center: flat(center.0, center.1),
        normal: p(0.0, 0.0, 1.0),
        radius,
    }
}

/// Ray straight down onto the z=0 plane through the cursor.
fn cursor_ray(cx: f64, cy: f64) -> Ray3 {
    Ray3 {
        origin: p(cx, cy, 10.0),
        direction: p(0.0, 0.0, -1.0),
    }
}

fn snaps(targets: &[SnapTarget], cx: f64, cy: f64, world_per_px: f64) -> Vec<SnapCandidate> {
    MeasurementEngine::default()
        .snap_targets(
            targets,
            &cursor_ray(cx, cy),
            Some(&z_plane()),
            world_per_px,
            Some(SpaceId::Model),
        )
        .unwrap()
}

#[test]
fn endpoint_snap_lands_on_the_line_end() {
    let t = [target(line((0.0, 0.0), (10.0, 0.0)))];
    let best = MeasurementEngine::default()
        .snap_best(&t, &cursor_ray(0.0, 0.2), Some(&z_plane()), 1.0, None)
        .unwrap()
        .expect("a snap");
    assert_eq!(best.kind, SnapKind::Endpoint);
    assert_eq!(best.point, flat(0.0, 0.0));
    assert_eq!(best.source.entity, EntityId(1));
}

#[test]
fn midpoint_snap_lands_on_the_segment_middle() {
    let t = [target(line((0.0, 0.0), (10.0, 0.0)))];
    let best = MeasurementEngine::default()
        .snap_best(&t, &cursor_ray(5.0, 0.2), Some(&z_plane()), 1.0, None)
        .unwrap()
        .expect("a snap");
    assert_eq!(best.kind, SnapKind::Midpoint);
    assert_eq!(best.point, flat(5.0, 0.0));
}

#[test]
fn center_snap_lands_on_the_circle_center() {
    let t = [target(circle((2.0, 3.0), 5.0))];
    let best = MeasurementEngine::default()
        .snap_best(&t, &cursor_ray(2.1, 3.1), Some(&z_plane()), 1.0, None)
        .unwrap()
        .expect("a snap");
    assert_eq!(best.kind, SnapKind::Center);
    assert_eq!(best.point, flat(2.0, 3.0));
}

#[test]
fn quadrant_snap_lands_on_the_circle_quadrant() {
    let t = [target(circle((0.0, 0.0), 5.0))];
    let all = snaps(&t, 5.0, 0.0, 1.0);
    let quad = all
        .iter()
        .find(|c| c.kind == SnapKind::Quadrant && c.point == flat(5.0, 0.0));
    assert!(quad.is_some(), "{all:?}");
    // At the exact quadrant the perpendicular candidate coincides, and the
    // quadrant kind has the higher priority.
    assert_eq!(all[0].kind, SnapKind::Quadrant);
}

#[test]
fn perpendicular_snap_drops_onto_the_segment() {
    let t = [target(line((0.0, 0.0), (10.0, 0.0)))];
    let best = MeasurementEngine::default()
        .snap_best(&t, &cursor_ray(3.0, 2.0), Some(&z_plane()), 1.0, None)
        .unwrap()
        .expect("a snap");
    assert_eq!(best.kind, SnapKind::Perpendicular);
    assert_eq!(best.point, flat(3.0, 0.0));
}

#[test]
fn perpendicular_is_not_offered_past_a_segment_end() {
    // The cursor is beyond `b`; the perpendicular foot would be behind the
    // segment, so only the endpoint snap is available.
    let t = [target(line((0.0, 0.0), (10.0, 0.0)))];
    let all = snaps(&t, 12.0, 2.0, 2.0);
    assert!(
        all.iter().all(|c| c.kind != SnapKind::Perpendicular),
        "{all:?}"
    );
}

#[test]
fn local_intersection_snap_finds_the_crossing() {
    let a = SnapTarget::new(
        line((0.0, 0.0), (10.0, 10.0)),
        source(1),
        SpaceId::Model,
        Precision::Analytic,
    );
    let b = SnapTarget::new(
        line((0.0, 10.0), (10.0, 0.0)),
        source(2),
        SpaceId::Model,
        Precision::Analytic,
    );
    let all = snaps(&[a, b], 5.1, 5.1, 1.0);
    let hit = all
        .iter()
        .find(|c| c.kind == SnapKind::LocalIntersection)
        .expect("intersection candidate");
    assert_eq!(hit.point, flat(5.0, 5.0));
    assert!(hit.secondary.is_some());
    assert_eq!(hit.secondary.as_ref().unwrap().entity, EntityId(2));
}

#[test]
fn candidate_behind_the_pick_ray_origin_is_not_returned() {
    let ray = Ray3 {
        origin: p(0.0, 0.0, 0.0),
        direction: p(1.0, 0.0, 0.0),
    };
    let behind = SnapTarget::new(
        line((-5.0, 0.0), (-1.0, 0.0)),
        source(1),
        SpaceId::Model,
        Precision::Analytic,
    );
    let found = MeasurementEngine::default()
        .snap_targets(&[behind], &ray, None, 1.0, None)
        .unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn pixel_tolerance_scales_with_world_per_px() {
    let t = [target(line((10.0, 0.0), (20.0, 0.0)))];
    // Distance 10 world units from the cursor: at 0.5 world/px the 12 px
    // aperture is 6 units and misses; at 1.0 it is 12 units and hits.
    assert!(snaps(&t, 0.0, 0.0, 0.5).is_empty());
    let all = snaps(&t, 0.0, 0.0, 1.0);
    let endpoint = all
        .iter()
        .find(|c| c.kind == SnapKind::Endpoint && c.point == flat(10.0, 0.0))
        .expect("endpoint within 12 px");
    assert!((endpoint.logical_pixel_distance - 10.0).abs() < 1e-9);
}

#[test]
fn snap_is_independent_of_the_display_pixel_budget() {
    let t = [
        target(line((0.0, 0.0), (10.0, 0.0))),
        target(circle((20.0, 20.0), 3.0)),
    ];
    let fine = TolerancePolicy {
        display_pixels: 0.01,
        ..Default::default()
    };
    let coarse = TolerancePolicy {
        display_pixels: 50.0,
        ..Default::default()
    };
    let ray = cursor_ray(1.0, 0.5);
    let a = collect_candidates(&t, &ray, Some(&z_plane()), 1.0, None, &fine).unwrap();
    let b = collect_candidates(&t, &ray, Some(&z_plane()), 1.0, None, &coarse).unwrap();
    assert_eq!(a, b);
}

#[test]
fn measurement_is_independent_of_the_display_pixel_budget() {
    let request = MeasurementRequest::from_tapped(
        MeasurementAlgorithm::Distance3d,
        vec![
            MeasurementPoint::model(flat(0.0, 0.0)),
            MeasurementPoint::model(flat(3.0, 4.0)),
        ],
        MeasurementSpace::World3d,
        UnitContext::drawing_units(),
        GeometrySource::UserPoints,
        Precision::Analytic,
    );
    let fine = MeasurementEngine {
        tolerance: TolerancePolicy {
            display_pixels: 0.01,
            ..Default::default()
        },
    };
    let coarse = MeasurementEngine {
        tolerance: TolerancePolicy {
            display_pixels: 50.0,
            ..Default::default()
        },
    };
    assert_eq!(
        fine.measure(&request).unwrap().value,
        coarse.measure(&request).unwrap().value
    );
}

#[test]
fn non_finite_or_degenerate_queries_are_rejected() {
    let t = [target(line((0.0, 0.0), (1.0, 0.0)))];
    let engine = MeasurementEngine::default();
    let zero_dir = Ray3 {
        origin: p(0.0, 0.0, 0.0),
        direction: p(0.0, 0.0, 0.0),
    };
    assert!(engine
        .snap_targets(&t, &zero_dir, Some(&z_plane()), 1.0, None)
        .is_err());
    let nan_ray = Ray3 {
        origin: p(f64::NAN, 0.0, 0.0),
        direction: p(0.0, 0.0, 1.0),
    };
    assert!(engine
        .snap_targets(&t, &nan_ray, Some(&z_plane()), 1.0, None)
        .is_err());
    let ray = cursor_ray(0.0, 0.0);
    assert!(engine
        .snap_targets(&t, &ray, Some(&z_plane()), 0.0, None)
        .is_err());
    let zero_tol = MeasurementEngine {
        tolerance: TolerancePolicy {
            interaction_logical_pixels: 0.0,
            ..Default::default()
        },
    };
    assert!(zero_tol
        .snap_targets(&t, &ray, Some(&z_plane()), 1.0, None)
        .is_err());
    // A poisoned target contributes nothing rather than a bogus point.
    let bad = SnapTarget::new(
        SemanticGeometry::Line {
            start: p(f64::NAN, 0.0, 0.0),
            end: flat(1.0, 0.0),
        },
        source(1),
        SpaceId::Model,
        Precision::Analytic,
    );
    assert!(engine
        .snap_targets(&[bad], &ray, Some(&z_plane()), 1.0, None)
        .unwrap()
        .is_empty());
}

#[test]
fn snapped_measurement_records_provenance() {
    let engine = MeasurementEngine::default();
    let request = MeasurementRequest::from_tapped(
        MeasurementAlgorithm::Distance3d,
        vec![
            MeasurementPoint::model(flat(0.0, 0.0)),
            MeasurementPoint::model(flat(3.0, 4.0)),
        ],
        MeasurementSpace::World3d,
        UnitContext::drawing_units(),
        GeometrySource::UserPoints,
        Precision::Analytic,
    );
    let snaps = vec![
        SnapProvenance {
            kind: SnapKind::Endpoint,
            source: source(1),
            secondary: None,
            precision: Precision::Analytic,
            logical_pixel_distance: 0.5,
        },
        SnapProvenance {
            kind: SnapKind::LocalIntersection,
            source: source(2),
            secondary: Some(source(3)),
            precision: Precision::Analytic,
            logical_pixel_distance: 0.25,
        },
    ];
    let snapped = engine.measure_snapped(&request, snaps).unwrap();
    assert!((snapped.record.value - 5.0).abs() < 1e-12);
    assert_eq!(snapped.snaps[0].kind, SnapKind::Endpoint);
    assert_eq!(
        snapped.snaps[1].secondary.as_ref().unwrap().entity,
        EntityId(3)
    );
    // A provenance count that does not match the inputs is an error.
    assert!(engine
        .measure_snapped(
            &request,
            vec![SnapProvenance {
                kind: SnapKind::Endpoint,
                source: source(1),
                secondary: None,
                precision: Precision::Analytic,
                logical_pixel_distance: 0.0,
            }]
        )
        .is_err());
}

#[test]
fn image_and_mask_yield_no_snap_candidates() {
    let image = SemanticGeometry::Image {
        origin: flat(0.0, 0.0),
        u: flat(1.0, 0.0),
        v: flat(0.0, 1.0),
        pixels: [4.0, 4.0],
        file: None,
        clip: None,
        visible: true,
    };
    let mask = SemanticGeometry::Mask {
        boundary: vec![flat(-5.0, -5.0), flat(5.0, -5.0), flat(5.0, 5.0)],
        inverted: false,
    };
    // Even hovering exactly over the image origin / mask vertex, neither
    // exposes a snap point.
    assert!(snaps(&[target(image)], 0.0, 0.0, 1.0).is_empty());
    assert!(snaps(&[target(mask)], 5.0, 5.0, 1.0).is_empty());
}
