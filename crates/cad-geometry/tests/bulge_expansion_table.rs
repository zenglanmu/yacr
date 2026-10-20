//! Equivalence test: every bulge -> arc expansion must place the arc on the
//! side the signed bulge encodes.
//!
//! Two of the workspace's six copies live in this crate: the display
//! tessellator (`append_bulge_arc`, reached through `tessellate_geometry`) and
//! EXPLODE's analytic recovery (`bulge_arc_geometry`). Both are checked against
//! the shared analytic table in
//! `cad-domain/tests/support/bulge_cases.rs`, which is included here (not
//! copied) so all seven consumers compare against the same numbers.
//!
//! Regression context: the display/EXPLODE centre used to be displaced to the
//! *apex* side, mirroring every arc while leaving semicircles (`cos(theta/2) =
//! 0`) untouched. Existing tests were semicircle/quadrant-symmetric, so they
//! passed. `bulge_cases` pins the centre side explicitly.

#[path = "../../cad-domain/tests/support/bulge_cases.rs"]
mod bulge_cases;

use cad_domain::{Point3, SemanticGeometry};
use cad_geometry::{bulge_arc_geometry, tessellate_geometry, TessellationParams};

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn flat(points: &[Point3]) -> Vec<[f64; 2]> {
    points.iter().map(|q| [q.x, q.y]).collect()
}

/// The display tessellation of the single-segment polyline `p0 -> p1`.
fn display_samples(case: &bulge_cases::Case, params: TessellationParams) -> Vec<[f64; 2]> {
    let geometry = SemanticGeometry::Polyline {
        points: vec![p(case.p0[0], case.p0[1]), p(case.p1[0], case.p1[1])],
        bulges: vec![case.bulge],
        closed: false,
    };
    flat(&tessellate_geometry(&geometry, params))
}

#[test]
fn bulge_cases_are_the_hand_computed_analytic_arcs() {
    bulge_cases::assert_table_is_hand_computed();
}

/// Display path: `append_bulge_arc` samples onto the analytic circle, sweeps the
/// analytic span in the analytic direction, and reaches the analytic apex on the
/// side opposite the centre.
#[test]
fn display_tessellation_matches_the_analytic_arc() {
    bulge_cases::assert_table_is_hand_computed();
    let params = TessellationParams {
        tolerance: 1e-4,
        ..TessellationParams::default()
    };
    for case in bulge_cases::table() {
        let samples = display_samples(&case, params);
        bulge_cases::assert_samples(
            "cad-geometry display (append_bulge_arc)",
            &case,
            &samples,
            params.tolerance + 1e-9,
        );
        // The extreme sample can be up to one half-step short of the apex.
        bulge_cases::assert_apex(
            "cad-geometry display (append_bulge_arc)",
            &case,
            &samples,
            2.0 * params.tolerance + 1e-9,
        );
    }
}

/// EXPLODE path: `bulge_arc_geometry` names the same centre, radius and signed
/// sweep as the analytic arc.
#[test]
fn explode_arc_recovery_matches_the_analytic_arc() {
    bulge_cases::assert_table_is_hand_computed();
    let normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    for case in bulge_cases::table() {
        let geometry = bulge_arc_geometry(
            p(case.p0[0], case.p0[1]),
            p(case.p1[0], case.p1[1]),
            case.bulge,
            normal,
        )
        .expect("a non-zero bulge is an arc");
        let SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } = geometry
        else {
            panic!("bulge_arc_geometry must return an Arc");
        };
        bulge_cases::assert_analytic_arc(
            "cad-geometry EXPLODE (bulge_arc_geometry)",
            &case,
            [center.x, center.y],
            radius,
            start,
            sweep,
        );
    }
}

/// The two representations must describe the *same curve*: tessellating the
/// recovered `Arc` must reproduce the display samples point for point (same
/// adaptive segment count, same start angle, same sweep).
#[test]
fn exploded_arc_tessellates_like_the_display_arc() {
    bulge_cases::assert_table_is_hand_computed();
    let params = TessellationParams {
        tolerance: 1e-4,
        ..TessellationParams::default()
    };
    let normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    for case in bulge_cases::table() {
        let display = display_samples(&case, params);
        let arc = bulge_arc_geometry(
            p(case.p0[0], case.p0[1]),
            p(case.p1[0], case.p1[1]),
            case.bulge,
            normal,
        )
        .expect("a non-zero bulge is an arc");
        let exploded = flat(&tessellate_geometry(&arc, params));
        assert_eq!(
            display.len(),
            exploded.len(),
            "{}: display emitted {} samples, the exploded Arc emitted {}",
            case.name,
            display.len(),
            exploded.len()
        );
        let tol = 1e-9 * (1.0 + case.radius + case.p0[0].abs() + case.p0[1].abs());
        for (index, (a, b)) in display.iter().zip(exploded.iter()).enumerate() {
            let d = (a[0] - b[0]).hypot(a[1] - b[1]);
            assert!(
                d <= tol,
                "{}: sample {index} differs between display {a:?} and exploded Arc {b:?} by {d}",
                case.name
            );
        }
    }
}
