//! Object-snap equivalence: `cad-measure`'s bulge expansion
//! (`bulge_arc_params`) must agree with the shared analytic table.
//!
//! The snap engine is the one consumer that exposes the arc **analytically**
//! rather than as samples: a bulged polyline edge contributes a `Center`
//! candidate at the arc's centre and a `Midpoint` candidate at the arc's
//! midpoint — which for a bulge is the apex, the farthest point from the chord.
//! A mirrored centre (the defect this table guards) moves both: the centre to
//! the apex side and the midpoint to the mirror image of the apex.
//!
//! The table itself lives in `cad-domain/tests/support/bulge_cases.rs` and is
//! included, not copied.

#[path = "../../cad-domain/tests/support/bulge_cases.rs"]
mod bulge_cases;

use cad_domain::{
    DocumentId, EntityId, InstancePath, Point3, Precision, Ray3, SelectionRef, SemanticGeometry,
    SpaceId, WorkPlane,
};
use cad_measure::{MeasurementEngine, SnapCandidate, SnapKind, SnapTarget};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn source() -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath(Vec::new()),
        sub_element: None,
    }
}

fn z_plane() -> WorkPlane {
    WorkPlane {
        origin: p(0.0, 0.0, 0.0),
        u: p(1.0, 0.0, 0.0),
        v: p(0.0, 1.0, 0.0),
    }
}

/// A pick ray straight down through the cursor, so the work-plane probe lands
/// exactly on `(x, y, 0)`.
fn cursor_ray(x: f64, y: f64) -> Ray3 {
    Ray3 {
        origin: p(x, y, 10.0),
        direction: p(0.0, 0.0, -1.0),
    }
}

fn snapped(target: &SnapTarget, at: [f64; 2]) -> Vec<SnapCandidate> {
    MeasurementEngine::default()
        .snap_targets(
            std::slice::from_ref(target),
            &cursor_ray(at[0], at[1]),
            Some(&z_plane()),
            1.0,
            Some(SpaceId::Model),
        )
        .expect("a valid snap query")
}

fn candidate_of_kind(candidates: &[SnapCandidate], kind: SnapKind) -> Point3 {
    candidates
        .iter()
        .find(|candidate| candidate.kind == kind)
        .map(|candidate| candidate.point)
        .unwrap_or_else(|| panic!("no {kind:?} candidate in {candidates:?}"))
}

fn distance(a: Point3, b: [f64; 2]) -> f64 {
    (a.x - b[0]).hypot(a.y - b[1])
}

#[test]
fn bulge_cases_are_the_hand_computed_analytic_arcs() {
    bulge_cases::assert_table_is_hand_computed();
}

#[test]
fn snap_centre_and_apex_follow_the_analytic_arc() {
    bulge_cases::assert_table_is_hand_computed();
    for case in bulge_cases::table() {
        let geometry = SemanticGeometry::Polyline {
            points: vec![
                p(case.p0[0], case.p0[1], 0.0),
                p(case.p1[0], case.p1[1], 0.0),
            ],
            bulges: vec![case.bulge],
            closed: false,
        };
        let target = SnapTarget::new(geometry, source(), SpaceId::Model, Precision::Analytic);
        let tol = 1e-9 * (1.0 + case.radius + case.center[0].abs() + case.center[1].abs());

        // Probe at the analytic centre: the `Center` snap must be there. A
        // mirrored centre puts this candidate half a chord away (a full `2 r
        // cos(theta/2)` from the analytic one).
        let at_center = snapped(&target, case.center);
        let center = candidate_of_kind(&at_center, SnapKind::Center);
        assert!(
            distance(center, case.center) <= tol,
            "{}: snap centre ({}, {}) != analytic centre {:?}",
            case.name,
            center.x,
            center.y,
            case.center
        );

        // Probe at the analytic apex: the edge midpoint of a bulge arc is the
        // apex, not the chord midpoint.
        let at_apex = snapped(&target, case.apex);
        let apex = candidate_of_kind(&at_apex, SnapKind::Midpoint);
        assert!(
            distance(apex, case.apex) <= tol,
            "{}: snap midpoint ({}, {}) != analytic apex {:?} (bulge {}); the arc is \
             mirrored across its chord",
            case.name,
            apex.x,
            apex.y,
            case.apex,
            case.bulge
        );
    }
}
