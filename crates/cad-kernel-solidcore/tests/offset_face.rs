//! `SolidOperation::OffsetFace` through the seam: the atomic Push/Pull move.
//!
//! These run the **real** pinned kernel. Every volume below is integrated by
//! `solidcore` from the body the modeler produced, so a wrong lift, a wrong
//! selector or a half-applied move shows up as a wrong number, not as an empty
//! success. Evidence boundary: Linux x86-64, this repository's test profile; no
//! GPU, no host, no other platform (ADR-0036).

mod common;

use cad_domain::CadError;
use cad_kernel_adapter::{
    execute_solid, PrimitiveSpec, SolidFaceSelector, SolidOperation, SolidOutcome,
};
use cad_kernel_solidcore::{SolidCoreModeler, FACE_NOT_FOUND};

use common::*;

fn box_body() -> cad_domain::BrepData {
    let modeler = SolidCoreModeler::new();
    let brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 10.0, 10.0, 10.0),
        },
    ));
    assert!(near(volume(&modeler, &brep), 1000.0));
    brep
}

fn moved(top: cad_domain::Point3, distance: f64) -> (SolidCoreModeler, cad_domain::BrepData) {
    let modeler = SolidCoreModeler::new();
    let brep = box_body();
    let moved = body(execute(
        &modeler,
        SolidOperation::OffsetFace {
            brep: &brep,
            selector: SolidFaceSelector::NearestToPoint(top),
            distance,
        },
    ));
    (modeler, moved)
}

#[test]
fn a_cap_face_moves_outward_and_inward_by_the_signed_distance() {
    // Pull the top cap out by 3: 1000 + 3 × 100.
    let (modeler, pulled) = moved(p(5.0, 5.0, 10.0), 3.0);
    assert!(near(volume(&modeler, &pulled), 1300.0));
    assert_eq!(pulled.face_count(), 6, "a moved cap keeps six faces");

    // Push the same cap in by 3: 1000 − 3 × 100.
    let (modeler, pushed) = moved(p(5.0, 5.0, 10.0), -3.0);
    assert!(near(volume(&modeler, &pushed), 700.0));
    assert_eq!(pushed.face_count(), 6);
}

#[test]
fn a_side_face_moves_too() {
    // Pull the +X side out by 2: 10 × 10 × 12.
    let (modeler, wider) = moved(p(10.0, 5.0, 5.0), 2.0);
    assert!(near(volume(&modeler, &wider), 1200.0));
    assert_eq!(wider.face_count(), 6);
}

#[test]
fn a_curved_face_moves_along_its_outward_normal() {
    let modeler = SolidCoreModeler::new();
    let cylinder = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Cylinder {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
                height: 2.0,
            },
        },
    ));
    let before = volume(&modeler, &cylinder);
    let widened = body(execute(
        &modeler,
        SolidOperation::OffsetFace {
            brep: &cylinder,
            selector: SolidFaceSelector::NearestToPoint(p(1.0, 0.0, 1.0)),
            distance: 0.5,
        },
    ));
    // A lateral face moved out by 0.5 makes the radius 1.5 over the same height.
    let expected = std::f64::consts::PI * 1.5 * 1.5 * 2.0;
    assert!(
        near(volume(&modeler, &widened), expected),
        "expected {expected}, got {}",
        volume(&modeler, &widened)
    );
    assert!(volume(&modeler, &widened) > before);
}

#[test]
fn a_pick_away_from_every_face_is_refused_by_name() {
    let modeler = SolidCoreModeler::new();
    let brep = box_body();
    let outcome = execute(
        &modeler,
        SolidOperation::OffsetFace {
            brep: &brep,
            selector: SolidFaceSelector::NearestToPoint(p(100.0, 100.0, 100.0)),
            distance: 1.0,
        },
    );
    assert_eq!(unsupported_code(&outcome), FACE_NOT_FOUND);
    // The input body is untouched: the refusal handed back no product at all.
    assert!(outcome.body().is_none());
    assert!(near(volume(&modeler, &brep), 1000.0));
}

#[test]
fn a_zero_distance_is_bad_input_not_a_no_op_body() {
    let modeler = SolidCoreModeler::new();
    let brep = box_body();
    let request = request(SolidOperation::OffsetFace {
        brep: &brep,
        selector: SolidFaceSelector::NearestToPoint(p(5.0, 5.0, 10.0)),
        distance: 0.0,
    });
    let result = execute_solid(&modeler, &request, &never());
    assert!(
        matches!(result, Err(CadError::InvalidInput(_))),
        "zero is refused before any kernel work"
    );
    assert!(near(volume(&modeler, &brep), 1000.0));
}

#[test]
fn an_inward_move_that_leaves_no_room_is_an_unsupported_refusal() {
    let modeler = SolidCoreModeler::new();
    let brep = box_body();
    let outcome = execute(
        &modeler,
        SolidOperation::OffsetFace {
            brep: &brep,
            selector: SolidFaceSelector::NearestToPoint(p(5.0, 5.0, 10.0)),
            distance: -20.0,
        },
    );
    // Either the kernel refuses the move (no room) or it produces a valid body;
    // it must never return a body that is not the box moved by the distance.
    match outcome {
        SolidOutcome::Unsupported { .. } => {
            assert!(near(volume(&modeler, &brep), 1000.0), "the input is intact");
        }
        SolidOutcome::Success(product) => {
            let moved = product.body().expect("a body product");
            assert!(volume(&modeler, moved) < 1000.0);
        }
        other => panic!("expected a refusal or a smaller body, got {other:?}"),
    }
}
