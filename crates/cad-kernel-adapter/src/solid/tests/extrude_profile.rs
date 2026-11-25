//! Profile-extrusion seam contracts: validation and the capability flag.
//!
//! Split out of `solid/tests.rs` to keep that file inside the code-size gate.
use super::*;
use crate::codes;

/// A two-line profile used by the extrusion validation/probe tests.
fn profile() -> Vec<cad_domain::BrepCurve> {
    vec![
        cad_domain::BrepCurve::Line {
            start: p(0.0, 0.0, 0.0),
            end: p(1.0, 0.0, 0.0),
        },
        cad_domain::BrepCurve::Line {
            start: p(1.0, 0.0, 0.0),
            end: p(0.0, 0.0, 0.0),
        },
    ]
}

#[test]
fn extrude_profile_validation_rejects_empty_zero_and_bad_directions() {
    let profile = profile();
    for distance in [0.0, -1.0, f64::NAN, MAX_EXTRUDE_DISTANCE + 1.0] {
        let req = request(SolidOperation::ExtrudeProfile {
            profile: &profile,
            direction: p(0.0, 0.0, 1.0),
            distance,
        });
        let error = req
            .validate()
            .expect_err("a non-positive/non-finite/oversized distance must be rejected");
        assert_eq!(error.code, codes::SOLID_INVALID_DIMENSION, "{error}");
    }
    let empty: [cad_domain::BrepCurve; 0] = [];
    let error = request(SolidOperation::ExtrudeProfile {
        profile: &empty,
        direction: p(0.0, 0.0, 1.0),
        distance: 1.0,
    })
    .validate()
    .expect_err("an empty profile must be rejected");
    assert_eq!(error.code, codes::SOLID_EMPTY_OPERAND, "{error}");
    let error = request(SolidOperation::ExtrudeProfile {
        profile: &profile,
        direction: p(0.0, 0.0, 0.0),
        distance: 1.0,
    })
    .validate()
    .expect_err("a zero direction must be rejected");
    assert_eq!(error.code, codes::SOLID_INVALID_POINT, "{error}");
}
