//! `SolidOperation::ExtrudeProfile` through the seam.
//!
//! The real pinned kernel integrates every volume below, so a wrong arc angle, a
//! wrong winding or an open loop shows up as a wrong number, an `Unsupported`
//! refusal, or no body — never as an empty success. Evidence boundary: Linux
//! x86-64, this repository's test profile; no GPU, no host, no other platform
//! (ADR-0036).

mod common;

use cad_domain::BrepCurve;
use cad_kernel_adapter::{execute_solid, SolidModeler, SolidOperation, SolidOutcome};
use cad_kernel_solidcore::{SolidCoreModeler, EXTRUDE_PROFILE_FAILED, LIFT_UNSUPPORTED};

use common::*;

fn line(start: (f64, f64), end: (f64, f64)) -> BrepCurve {
    BrepCurve::Line {
        start: p(start.0, start.1, 0.0),
        end: p(end.0, end.1, 0.0),
    }
}

/// A 2 × 3 rectangle in `z = 0`, counter-clockwise seen from `+Z`.
fn rectangle() -> Vec<BrepCurve> {
    vec![
        line((0.0, 0.0), (2.0, 0.0)),
        line((2.0, 0.0), (2.0, 3.0)),
        line((2.0, 3.0), (0.0, 3.0)),
        line((0.0, 3.0), (0.0, 0.0)),
    ]
}

fn circle(radius: f64) -> Vec<BrepCurve> {
    vec![BrepCurve::Circle {
        center: p(0.0, 0.0, 0.0),
        normal: p(0.0, 0.0, 1.0),
        u_dir: p(1.0, 0.0, 0.0),
        radius,
    }]
}

fn extrude(modeler: &dyn SolidModeler, profile: &[BrepCurve], distance: f64) -> SolidOutcome {
    execute(
        modeler,
        SolidOperation::ExtrudeProfile {
            profile,
            direction: p(0.0, 0.0, 1.0),
            distance,
        },
    )
}

#[test]
fn a_rectangular_profile_extrudes_to_a_closed_prism() {
    let modeler = SolidCoreModeler::new();
    let brep = body(extrude(&modeler, &rectangle(), 5.0));
    assert!(near(volume(&modeler, &brep), 30.0));
    assert_eq!(brep.shells.len(), 1);
    // A capped prism: two caps plus four walls.
    assert_eq!(brep.face_count(), 6);
}

#[test]
fn a_full_circle_profile_extrudes_to_a_cylinder() {
    let modeler = SolidCoreModeler::new();
    let brep = body(extrude(&modeler, &circle(1.0), 5.0));
    let expected = std::f64::consts::PI * 5.0;
    assert!(
        near(volume(&modeler, &brep), expected),
        "expected {expected}, got {}",
        volume(&modeler, &brep)
    );
}

#[test]
fn the_other_side_is_the_negated_direction_not_a_negative_distance() {
    let modeler = SolidCoreModeler::new();
    let down = execute(
        &modeler,
        SolidOperation::ExtrudeProfile {
            profile: &rectangle(),
            direction: p(0.0, 0.0, -1.0),
            distance: 5.0,
        },
    );
    let brep = body(down);
    assert!(near(volume(&modeler, &brep), 30.0));
    // The body lies below the profile plane.
    let z = brep.shells[0]
        .faces
        .iter()
        .flat_map(|face| &face.loops)
        .flat_map(|loop_| &loop_.edges)
        .filter_map(|curve| match curve {
            BrepCurve::Line { start, .. } => Some(start.z),
            _ => None,
        })
        .fold(f64::INFINITY, f64::min);
    assert!(near(z, -5.0), "the extruded body starts at z = -5, got {z}");
}

#[test]
fn an_open_profile_is_refused_by_name() {
    let modeler = SolidCoreModeler::new();
    let open = vec![
        line((0.0, 0.0), (2.0, 0.0)),
        line((2.0, 0.0), (2.0, 3.0)),
        line((2.0, 3.0), (0.0, 3.0)),
    ];
    let outcome = extrude(&modeler, &open, 5.0);
    assert_eq!(unsupported_code(&outcome), EXTRUDE_PROFILE_FAILED);
    assert!(outcome.body().is_none());
}

#[test]
fn a_zero_distance_is_bad_input_before_the_kernel() {
    let modeler = SolidCoreModeler::new();
    let profile = rectangle();
    let request = request(SolidOperation::ExtrudeProfile {
        profile: &profile,
        direction: p(0.0, 0.0, 1.0),
        distance: 0.0,
    });
    let result = execute_solid(&modeler, &request, &never());
    assert!(
        matches!(result, Err(cad_domain::CadError::InvalidInput(_))),
        "zero distance is refused by validate"
    );
}

#[test]
fn a_zero_direction_is_bad_input_before_the_kernel() {
    let modeler = SolidCoreModeler::new();
    let profile = rectangle();
    let request = request(SolidOperation::ExtrudeProfile {
        profile: &profile,
        direction: p(0.0, 0.0, 0.0),
        distance: 1.0,
    });
    let result = execute_solid(&modeler, &request, &never());
    assert!(matches!(result, Err(cad_domain::CadError::InvalidInput(_))));
}

#[test]
fn the_solidcore_modeler_declares_the_profile_capability() {
    let modeler = SolidCoreModeler::new();
    assert!(modeler.capabilities().extrude_profile);
    let registration = modeler.registration();
    assert!(
        registration
            .capabilities
            .iter()
            .any(|entry| entry == "extrude.profile"),
        "{:?}",
        registration.capabilities
    );
}

#[test]
fn a_partial_arc_wall_is_refused_by_the_neutral_readback() {
    // The kernel builds a half-disc, but its cap arc comes back as a rational
    // spline edge, which this seam's `record → BrepData` reader does not map
    // (only lines, full/open circles and exactly straight splines). The honest
    // answer is a refusal — never a body the next lift would reject. The command
    // layer therefore offers line and full-circle profiles only.
    let profile = vec![
        line((-1.0, 0.0), (1.0, 0.0)),
        BrepCurve::Arc {
            center: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
            radius: 1.0,
            start: p(1.0, 0.0, 0.0),
            end: p(-1.0, 0.0, 0.0),
        },
    ];
    let modeler = SolidCoreModeler::new();
    let outcome = extrude(&modeler, &profile, 4.0);
    assert_eq!(unsupported_code(&outcome), LIFT_UNSUPPORTED);
    assert!(outcome.body().is_none());
}
