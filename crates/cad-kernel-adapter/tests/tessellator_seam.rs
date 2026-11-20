//! The adapter's seam entry (`tessellate_exchange_with`): a NURBS-bearing body is
//! handed to an injected `SolidTessellator`, everything else stays on the neutral
//! path, and a seam refusal is never dropped.
//!
//! The stub here is a **test double**: it proves the delegation rule and the
//! fallback plumbing, not any mesher. The real mesher is exercised by
//! `cad-kernel-solidcore`'s `tests/tess.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cad_domain::{
    BrepCurve, BrepData, BrepFace, BrepLoop, BrepShell, BrepSurface, CadResult, Diagnostic, Mesh,
    Point3, Precision, Registration, Revision, SubElementId,
};
use cad_kernel_adapter::{
    codes, tessellate_exchange, tessellate_exchange_with, ExchangeKind, SolidExchange,
    SolidTessellator, TessellationBudget, TessellationMesh, TessellationOutcome,
    TessellationRequest, TessellationResult, TessellationTolerance, UnsupportedReason,
};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    p(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn cross(a: Point3, b: Point3) -> Point3 {
    p(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

fn unit(v: Point3) -> Point3 {
    let length = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if length > 1e-12 {
        p(v.x / length, v.y / length, v.z / length)
    } else {
        p(0.0, 0.0, 0.0)
    }
}

fn plane_face(id: u32, cycle: &[Point3]) -> BrepFace {
    let origin = cycle[0];
    let u_dir = cycle
        .iter()
        .map(|corner| unit(sub(*corner, origin)))
        .find(|direction| direction.x != 0.0 || direction.y != 0.0 || direction.z != 0.0)
        .unwrap_or(p(1.0, 0.0, 0.0));
    let normal = unit(cross(sub(cycle[1], origin), sub(cycle[2], origin)));
    let edges = (0..cycle.len())
        .map(|i| BrepCurve::Line {
            start: cycle[i],
            end: cycle[(i + 1) % cycle.len()],
        })
        .collect();
    BrepFace {
        id,
        surface: BrepSurface::Plane {
            origin,
            normal,
            u_dir,
        },
        reversed: false,
        loops: vec![BrepLoop { edges }],
        domain: None,
        tolerance: None,
    }
}

/// A closed planar unit box `[0,1]^3`.
fn unit_box() -> BrepData {
    let corners = [
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
        p(0.0, 0.0, 1.0),
        p(1.0, 0.0, 1.0),
        p(1.0, 1.0, 1.0),
        p(0.0, 1.0, 1.0),
    ];
    // Outward-wound cycles: -Z, +Z, -Y, +X, +Y, -X.
    let cycles: [&[usize]; 6] = [
        &[0, 3, 2, 1],
        &[4, 5, 6, 7],
        &[0, 1, 5, 4],
        &[1, 2, 6, 5],
        &[2, 3, 7, 6],
        &[3, 0, 4, 7],
    ];
    let faces = cycles
        .iter()
        .enumerate()
        .map(|(id, cycle)| {
            let points: Vec<Point3> = cycle.iter().map(|&index| corners[index]).collect();
            plane_face(id as u32, &points)
        })
        .collect();
    BrepData {
        shells: vec![BrepShell { id: 0, faces }],
        placement: None,
        tolerance: None,
    }
}

/// The same box whose `+Z` face is a degree-1x1 NURBS plane over its own `(u, v)`
/// box — the face the neutral mesher refuses and a NURBS mesher can draw.
fn nurbs_top_box() -> BrepData {
    let mut brep = unit_box();
    let top = &mut brep.shells[0].faces[1];
    top.surface = BrepSurface::Nurbs {
        u_knots: vec![0.0, 0.0, 1.0, 1.0],
        v_knots: vec![0.0, 0.0, 1.0, 1.0],
        points: vec![
            vec![[0.0, 0.0, 1.0, 1.0], [0.0, 1.0, 1.0, 1.0]],
            vec![[1.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
        ],
    };
    top.domain = Some([[0.0, 1.0], [0.0, 1.0]]);
    brep
}

/// A one-triangle facet set, so a delegating run can be told apart from the
/// neutral one by its mesh alone.
fn stub_mesh() -> TessellationMesh {
    TessellationMesh {
        mesh: Mesh {
            vertices: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
            triangles: vec![[0, 1, 2]],
            normals: vec![p(0.0, 0.0, 1.0); 3],
            face_sources: vec![Some(SubElementId {
                source_key: "brep:face:0/1".into(),
                topology_revision: Revision(0),
            })],
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
        },
        edges: vec![vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)]],
        precision: Precision::Approximate {
            error_bound: Some(1.0e-2),
        },
    }
}

/// A stub tessellator: counts its calls and answers with a fixed outcome.
struct StubTessellator {
    calls: Arc<AtomicUsize>,
    outcome: TessellationOutcome,
    /// The face sources of the last request it was handed, to prove the synthetic
    /// identity is what the seam builds.
    last_exchange: std::sync::Mutex<Option<ExchangeKind>>,
}

impl StubTessellator {
    fn new(outcome: TessellationOutcome) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            StubTessellator {
                calls: calls.clone(),
                outcome,
                last_exchange: std::sync::Mutex::new(None),
            },
            calls,
        )
    }
}

impl SolidTessellator for StubTessellator {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "test.stub-tessellator".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["3DSOLID".into(), "BODY".into()],
            capabilities: vec!["nurbs-faces".into()],
        }
    }

    fn tessellate(
        &self,
        request: &TessellationRequest,
        _cancelled: &dyn Fn() -> bool,
    ) -> CadResult<TessellationResult> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.last_exchange.lock().expect("stub state") = Some(request.exchange.kind());
        Ok(TessellationResult {
            stamp: request.stamp.clone(),
            outcome: self.outcome.clone(),
        })
    }
}

fn tolerance() -> TessellationTolerance {
    TessellationTolerance::default()
}

fn neutral(brep: &BrepData) -> TessellationOutcome {
    tessellate_exchange(
        &SolidExchange::Brep(brep.clone()),
        tolerance(),
        TessellationBudget::default(),
    )
}

#[test]
fn a_nurbs_bearing_brep_is_delegated_and_its_outcome_returned() {
    let body = nurbs_top_box();
    let (stub, calls) = StubTessellator::new(TessellationOutcome::Success {
        geometry: stub_mesh(),
        diagnostics: Vec::new(),
    });
    let outcome = tessellate_exchange_with(
        &SolidExchange::Brep(body),
        tolerance(),
        TessellationBudget::default(),
        Some(&stub),
        &|| false,
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1, "the seam is consulted");
    assert!(outcome.is_success(), "{outcome:?}");
    let mesh = outcome.mesh().expect("facets");
    assert_eq!(mesh.mesh.triangles, vec![[0, 1, 2]]);
    assert_eq!(mesh.mesh.vertices.len(), 3);
    assert_eq!(
        *stub.last_exchange.lock().expect("stub state"),
        Some(ExchangeKind::Brep),
        "the seam is handed the neutral B-rep, never raw bytes"
    );
}

#[test]
fn no_tessellator_keeps_the_neutral_partial_naming_the_nurbs_face() {
    let body = nurbs_top_box();
    let outcome = tessellate_exchange_with(
        &SolidExchange::Brep(body.clone()),
        tolerance(),
        TessellationBudget::default(),
        None,
        &|| false,
    );
    assert!(
        matches!(outcome, TessellationOutcome::Partial { .. }),
        "a NURBS face the neutral mesher cannot draw is a partial body: {outcome:?}"
    );
    let degradation = outcome.degradation().expect("degradation");
    let missing = degradation
        .missing_faces
        .iter()
        .find(|face| face.id == 1)
        .expect("the NURBS face (id 1) is named");
    assert!(
        missing.reason.contains("nurbs"),
        "the reason names what was missing: {}",
        missing.reason
    );
    // `None` must be exactly the public neutral entry point, not a second path.
    assert_eq!(outcome, neutral(&body));
}

#[test]
fn an_analytic_only_body_never_calls_the_seam() {
    let body = unit_box();
    let (stub, calls) = StubTessellator::new(TessellationOutcome::Success {
        geometry: stub_mesh(),
        diagnostics: Vec::new(),
    });
    let outcome = tessellate_exchange_with(
        &SolidExchange::Brep(body.clone()),
        tolerance(),
        TessellationBudget::default(),
        Some(&stub),
        &|| false,
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "analytic faces keep the neutral mesher (no display churn)"
    );
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(outcome, neutral(&body));
}

#[test]
fn a_seam_refusal_stays_visible_when_the_neutral_path_runs() {
    let body = nurbs_top_box();
    let (stub, calls) = StubTessellator::new(TessellationOutcome::Unsupported {
        reason: UnsupportedReason {
            code: "test.stub_refusal".into(),
            exchange: ExchangeKind::Brep,
            detail: "the stub declines this body".into(),
        },
    });
    let outcome = tessellate_exchange_with(
        &SolidExchange::Brep(body),
        tolerance(),
        TessellationBudget::default(),
        Some(&stub),
        &|| false,
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    // The neutral path still ran (a Partial body with the NURBS face missing), and
    // the stub's refusal is on it, not dropped.
    assert!(
        matches!(outcome, TessellationOutcome::Partial { .. }),
        "{outcome:?}"
    );
    assert!(
        outcome
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "test.stub_refusal"),
        "the seam's refusal is kept: {:?}",
        outcome.diagnostics()
    );
}

#[test]
fn a_seam_failure_keeps_both_sets_of_diagnostics() {
    let body = nurbs_top_box();
    let (stub, _calls) = StubTessellator::new(TessellationOutcome::Failed {
        diagnostics: vec![Diagnostic {
            object: None,
            code: codes::KERNEL_TOLERANCE_NOT_MET.to_string(),
            message: "the stub could not meet the chord".into(),
        }],
    });
    let outcome = tessellate_exchange_with(
        &SolidExchange::Brep(body),
        tolerance(),
        TessellationBudget::default(),
        Some(&stub),
        &|| false,
    );
    let codes_found: Vec<&str> = outcome
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(
        codes_found.contains(&codes::KERNEL_TOLERANCE_NOT_MET),
        "the seam's diagnostic survives: {codes_found:?}"
    );
    assert!(
        codes_found.contains(&codes::KERNEL_MISSING_FACE),
        "the neutral degradation is reported too: {codes_found:?}"
    );
}

#[test]
fn a_non_brep_payload_is_never_delegated() {
    let (stub, calls) = StubTessellator::new(TessellationOutcome::Success {
        geometry: stub_mesh(),
        diagnostics: Vec::new(),
    });
    let outcome = tessellate_exchange_with(
        &SolidExchange::Sat(b"ACIS ...".to_vec()),
        tolerance(),
        TessellationBudget::default(),
        Some(&stub),
        &|| false,
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    match outcome {
        TessellationOutcome::Unsupported { reason } => {
            assert_eq!(reason.code, codes::KERNEL_NO_ACIS_KERNEL)
        }
        other => panic!("expected the neutral unsupported answer, got {other:?}"),
    }
}

/// An empty `Partial` from the seam is not usable output: the neutral path runs
/// and its facets win.
#[test]
fn an_empty_seam_partial_falls_back_to_the_neutral_path() {
    let body = nurbs_top_box();
    let (stub, calls) = StubTessellator::new(TessellationOutcome::Partial {
        geometry: TessellationMesh {
            mesh: Mesh {
                vertices: Vec::new(),
                triangles: Vec::new(),
                normals: Vec::new(),
                face_sources: Vec::new(),
                colors: Vec::new(),
                uvs: Vec::new(),
                texture: None,
            },
            edges: Vec::new(),
            precision: Precision::Unknown,
        },
        degradation: Default::default(),
        diagnostics: Vec::new(),
    });
    let outcome = tessellate_exchange_with(
        &SolidExchange::Brep(body),
        tolerance(),
        TessellationBudget::default(),
        Some(&stub),
        &|| false,
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let mesh = outcome.mesh().expect("the neutral facets");
    assert!(!mesh.mesh.triangles.is_empty());
}
