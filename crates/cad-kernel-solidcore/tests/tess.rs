//! ADR-0036 P2.1c part 1: NURBS face display through `solidcore-tess`, behind the
//! `SolidTessellator` seam.
//!
//! Every contract here drives the **real** pinned mesher over a body this crate
//! lifts itself (no mock, no golden). Evidence boundary: Linux x86-64, this
//! repository's test profile, the real kernel. No GPU, no window, no other
//! platform, and nothing here says anything about pixels or performance.
//!
//! The subject body is a unit box whose `+Z` face is a **degree-1x1 NURBS patch**
//! (a plane, weights exactly `1.0`, its own `(u, v)` box) — the smallest body that
//! carries a face the neutral tessellator cannot mesh and the pinned one can.

use std::sync::atomic::{AtomicBool, Ordering};

use cad_domain::{BrepData, BrepSurface, DocumentId, ObjectId, Point3, Precision, TaskStamp};
use cad_kernel_adapter::{
    codes, GeometryHandle, SolidExchange, SolidTessellator, TessellationBudget,
    TessellationOutcome, TessellationRequest, TessellationTolerance,
};
use cad_kernel_solidcore::{SolidCoreTessellator, LIFT_UNSUPPORTED};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// A unit box (lower corner at the origin) whose `+Z` face is a degree-1x1 NURBS
/// plane over its own `[0, 1] x [0, 1]` box.
///
/// `box_brep` authors the `+Z` face second (`cycle [4, 5, 6, 7]`), wound
/// counter-clockwise seen from `+Z`. The patch is the same plane
/// `(u, v) -> (u, v, 1)`, so every side face's top edge lies on it exactly and the
/// shell stays closed.
fn nurbs_top_box() -> BrepData {
    let mut brep = cad_kernel_solidcore::box_brep(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0);
    let top = &mut brep.shells[0].faces[1];
    assert!(
        matches!(top.surface, BrepSurface::Plane { .. }),
        "box_brep's second face is the +Z plane"
    );
    // Degree 1 in both directions: clamped knots `[a, a, b, b]`, a 2x2 net. Every
    // weight is exactly 1.0, so the record writes it as a plain `Spline`.
    top.surface = BrepSurface::Nurbs {
        u_knots: vec![0.0, 0.0, 1.0, 1.0],
        v_knots: vec![0.0, 0.0, 1.0, 1.0],
        // `points[i][j]` is the coefficient at `(u_i, v_j)`, written
        // `[x*w, y*w, z*w, w]`.
        points: vec![
            vec![[0.0, 0.0, 1.0, 1.0], [0.0, 1.0, 1.0, 1.0]],
            vec![[1.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
        ],
    };
    top.domain = Some([[0.0, 1.0], [0.0, 1.0]]);
    brep
}

fn request(brep: BrepData, linear_deflection: f64) -> TessellationRequest {
    TessellationRequest {
        geometry: GeometryHandle::Resolved(ObjectId(7)),
        exchange: SolidExchange::Brep(brep),
        tolerance: TessellationTolerance {
            linear_deflection,
            angular_deflection: TessellationTolerance::default().angular_deflection,
        },
        budget: TessellationBudget::default(),
        stamp: TaskStamp::new(DocumentId(1), 1),
    }
}

fn tessellate(request: &TessellationRequest) -> TessellationOutcome {
    SolidCoreTessellator::new()
        .tessellate(request, &|| false)
        .expect("no cancellation, so no error")
        .outcome
}

#[test]
fn a_nurbs_face_is_meshed_through_the_pinned_mesher() {
    let requested = 1.0e-2;
    let request = request(nurbs_top_box(), requested);
    let outcome = tessellate(&request);

    assert!(
        outcome.is_success(),
        "nothing was lost, so this must be a Success: {outcome:?}"
    );
    let mesh = outcome.mesh().expect("a facet set");
    assert!(!mesh.mesh.triangles.is_empty(), "the body is drawn");
    assert!(
        !mesh.edges.is_empty(),
        "the plan samples the body's own edges"
    );
    assert_eq!(
        mesh.mesh.face_sources.len(),
        mesh.mesh.triangles.len(),
        "one source per triangle"
    );
    // Every triangle is attributed to a face of this body, under the neutral
    // `brep:face:<shell>/<face>` convention, never to a fabricated id.
    for source in &mesh.mesh.face_sources {
        let source = source.as_ref().expect("every triangle names its face");
        let id: u32 = source
            .source_key
            .strip_prefix("brep:face:0/")
            .expect("the neutral source-key convention")
            .parse()
            .expect("a numeric face id");
        assert!(id < 6, "face id {id} is not one of the box's six faces");
    }
    assert_eq!(mesh.mesh.normals.len(), mesh.mesh.vertices.len());
    // Facets, at a stated chord — never claimed analytic.
    match mesh.precision {
        Precision::Approximate {
            error_bound: Some(bound),
        } => assert!(
            bound <= requested,
            "the reported error bound {bound} claims less than the {requested} requested"
        ),
        ref other => panic!("a NURBS face is an approximate mesh, got {other:?}"),
    }
}

#[test]
fn a_finer_chord_never_produces_fewer_triangles() {
    // Tolerance must be monotone in what the caller asked for. The subject's
    // NURBS face is a plane, so its facet count may stay the same (the mesher is
    // allowed to be exactly as good at both tolerances); what is forbidden is a
    // finer request yielding *fewer* triangles, which would mean the request's
    // chord did not reach the planner.
    let coarse_request = request(nurbs_top_box(), 1.0e-2);
    let fine_request = request(nurbs_top_box(), 5.0e-3);
    let coarse = tessellate(&coarse_request);
    let fine = tessellate(&fine_request);
    let coarse_mesh = coarse.mesh().expect("coarse facet set");
    let fine_mesh = fine.mesh().expect("fine facet set");
    assert!(
        fine_mesh.mesh.triangles.len() >= coarse_mesh.mesh.triangles.len(),
        "halving the chord reduced the triangle count: {} -> {}",
        coarse_mesh.mesh.triangles.len(),
        fine_mesh.mesh.triangles.len()
    );
    // The request's chord is what the outcome reports, so the two differ exactly
    // as asked (both are far above the mesher's `1e-6 x size` floor).
    let bound = |outcome: &TessellationOutcome| match outcome.mesh().map(|m| &m.precision) {
        Some(Precision::Approximate {
            error_bound: Some(bound),
        }) => *bound,
        other => panic!("expected an approximate mesh, got {other:?}"),
    };
    assert!(
        (bound(&coarse) - 1.0e-2).abs() < 1e-12,
        "{}",
        bound(&coarse)
    );
    assert!((bound(&fine) - 5.0e-3).abs() < 1e-12, "{}", bound(&fine));
}

#[test]
fn a_body_the_lift_refuses_is_unsupported_and_draws_nothing() {
    // Two shells cannot be lifted (this build lifts exactly one closed shell):
    // the refusal is `solid.lift_unsupported`, and no mesh may come back.
    let mut brep = nurbs_top_box();
    let second = brep.shells[0].clone();
    brep.shells.push(second);
    let outcome = tessellate(&request(brep, 1.0e-2));
    let TessellationOutcome::Unsupported { reason } = &outcome else {
        panic!("expected Unsupported for an unliftable body, got {outcome:?}");
    };
    assert_eq!(reason.code, LIFT_UNSUPPORTED);
    assert!(
        reason.detail.contains("shells"),
        "the refusal is the lift's own message: {}",
        reason.detail
    );
    assert!(outcome.mesh().is_none(), "a refusal never carries facets");
}

#[test]
fn one_face_of_budget_is_a_named_failure() {
    let mut request = request(nurbs_top_box(), 1.0e-2);
    request.budget = TessellationBudget {
        max_faces: 1,
        ..TessellationBudget::default()
    };
    let outcome = tessellate(&request);
    let TessellationOutcome::Failed { diagnostics } = &outcome else {
        panic!("expected Failed over the budget, got {outcome:?}");
    };
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::KERNEL_BUDGET_EXCEEDED);
    assert!(outcome.mesh().is_none());
}

#[test]
fn the_registration_names_the_pinned_mesher() {
    let registration = SolidCoreTessellator::new().registration();
    assert_eq!(registration.type_key, "yacr.kernel.solidcore.tess");
    assert_eq!(registration.version, 1);
    assert_eq!(registration.priority, 10);
    assert_eq!(
        registration.entity_types,
        vec!["3DSOLID".to_string(), "BODY".to_string()]
    );
    assert!(registration
        .capabilities
        .iter()
        .any(|capability| capability == "nurbs-faces"));
    assert!(registration
        .capabilities
        .iter()
        .any(|capability| capability == "analytic-faces"));
}

#[test]
fn a_cancelled_request_is_an_error_without_a_mesh() {
    let flag = AtomicBool::new(true);
    let request = request(nurbs_top_box(), 1.0e-2);
    let error = SolidCoreTessellator::new()
        .tessellate(&request, &|| flag.load(Ordering::Relaxed))
        .expect_err("a cancelled request has no result");
    assert_eq!(error, cad_domain::CadError::Cancelled);
}

#[test]
fn a_bad_tolerance_is_an_invalid_input() {
    let mut request = request(nurbs_top_box(), 1.0e-2);
    request.tolerance.linear_deflection = 0.0;
    let error = SolidCoreTessellator::new()
        .tessellate(&request, &|| false)
        .expect_err("a non-positive chord is rejected before any work");
    assert!(
        matches!(error, cad_domain::CadError::InvalidInput(_)),
        "{error:?}"
    );
}
