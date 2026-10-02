//! Unit tests.

use super::*;

fn stamp() -> TaskStamp {
    TaskStamp::new(DocumentId(1), 1)
}

fn request(exchange: SolidExchange) -> TessellationRequest {
    TessellationRequest {
        geometry: GeometryHandle::Resolved(ObjectId(7)),
        exchange,
        tolerance: TessellationTolerance::default(),
        budget: TessellationBudget::default(),
        stamp: stamp(),
    }
}

fn no_cancel() -> bool {
    false
}

#[test]
fn non_empty_sat_is_unsupported_not_empty_success() {
    // A non-empty ACIS payload must report `Unsupported`, never `Ok(empty)`.
    let req = request(SolidExchange::Sat(b"ACIS ...".to_vec()));
    let result = NoKernelTessellator.tessellate(&req, &no_cancel).unwrap();
    match &result.outcome {
        TessellationOutcome::Unsupported { reason } => {
            assert_eq!(reason.code, codes::KERNEL_NO_ACIS_KERNEL);
            assert_eq!(reason.exchange, ExchangeKind::Sat);
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
    assert!(result.outcome.mesh().is_none());
    assert!(!result.is_usable());
}

#[test]
fn empty_payload_is_failed_not_success() {
    let req = request(SolidExchange::Sab(Vec::new()));
    let result = NoKernelTessellator.tessellate(&req, &no_cancel).unwrap();
    let TessellationOutcome::Failed { diagnostics } = result.outcome else {
        panic!("expected Failed for an empty payload");
    };
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::KERNEL_EMPTY_GEOMETRY);
}

#[test]
fn missing_handle_is_reported() {
    let mut req = request(SolidExchange::Sat(b"data".to_vec()));
    req.geometry = GeometryHandle::Missing {
        key: "acis:42".into(),
    };
    let result = NoKernelTessellator.tessellate(&req, &no_cancel).unwrap();
    let TessellationOutcome::Failed { diagnostics } = result.outcome else {
        panic!("expected Failed for an unresolved handle");
    };
    assert_eq!(diagnostics[0].code, codes::KERNEL_MISSING_HANDLE);
}

#[test]
fn unknown_exchange_is_unsupported_exchange() {
    let req = request(SolidExchange::Unsupported {
        type_key: "vendor.cfd".into(),
        data: vec![1, 2, 3],
    });
    let result = NoKernelTessellator.tessellate(&req, &no_cancel).unwrap();
    let TessellationOutcome::Unsupported { reason } = result.outcome else {
        panic!("expected Unsupported");
    };
    assert_eq!(reason.code, codes::KERNEL_UNSUPPORTED_EXCHANGE);
}

#[test]
fn bad_tolerance_is_rejected() {
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut req = request(SolidExchange::Sat(b"data".to_vec()));
        req.tolerance.linear_deflection = bad;
        let err = NoKernelTessellator
            .tessellate(&req, &no_cancel)
            .expect_err("bad tolerance must be rejected");
        match err {
            CadError::InvalidInput(message) => {
                assert!(message.contains(codes::KERNEL_INVALID_TOLERANCE));
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }
}

#[test]
fn zero_budget_is_rejected() {
    let mut req = request(SolidExchange::Sat(b"data".to_vec()));
    req.budget.max_faces = 0;
    let err = NoKernelTessellator
        .tessellate(&req, &no_cancel)
        .expect_err("zero budget must be rejected");
    match err {
        CadError::InvalidInput(message) => {
            assert!(message.contains(codes::KERNEL_INVALID_BUDGET));
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn cancellation_wins_before_validation() {
    let mut req = request(SolidExchange::Sat(b"data".to_vec()));
    req.tolerance.linear_deflection = f64::NAN;
    let err = NoKernelTessellator
        .tessellate(&req, &|| true)
        .expect_err("cancelled request must error");
    assert_eq!(err, CadError::Cancelled);
}

#[test]
fn budget_exceeded_is_reported_with_stable_code() {
    let mesh = TessellationMesh {
        mesh: Mesh {
            vertices: vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            ],
            triangles: vec![[0, 1, 2]],
            normals: vec![],
            face_sources: vec![None],
            colors: Vec::new(),
        },
        edges: vec![vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ]],
        precision: Precision::Approximate {
            error_bound: Some(0.01),
        },
    };
    // `validate()` rejects zero, but `check` is the post-hoc enforcement a
    // real kernel uses; exercise it directly with a tiny non-zero cap.
    let budget = TessellationBudget {
        max_faces: 1,
        max_vertices: 1,
        max_edges: 1,
    };
    let diagnostic = budget
        .check(&mesh)
        .expect_err("1 face / 3 vertices must exceed a 1/1/1 budget");
    assert_eq!(diagnostic.code, codes::KERNEL_BUDGET_EXCEEDED);

    let generous = TessellationBudget::default();
    assert!(generous.check(&mesh).is_ok());
}

#[test]
fn degradation_reports_stable_codes() {
    let degradation = TessellationDegradation {
        missing_faces: vec![FaceRef {
            id: 3,
            reason: "self-intersecting loop".into(),
        }],
        open_edges: vec![EdgeRef {
            id: 9,
            reason: "vertices not shared".into(),
        }],
        dropped_shells: vec![ShellRef {
            id: 1,
            reason: "contained a missing face".into(),
        }],
    };
    assert!(!degradation.is_empty());
    let codes: Vec<String> = degradation
        .diagnostics()
        .into_iter()
        .map(|d| d.code)
        .collect();
    assert!(codes.contains(&codes::KERNEL_MISSING_FACE.to_string()));
    assert!(codes.contains(&codes::KERNEL_OPEN_EDGE.to_string()));
    assert!(codes.contains(&codes::KERNEL_DROPPED_SHELL.to_string()));
    assert_eq!(codes.len(), 3);
}

#[test]
fn partial_outcome_exposes_mesh_and_degradation() {
    let outcome = TessellationOutcome::Partial {
        geometry: TessellationMesh {
            mesh: Mesh::default(),
            edges: vec![],
            precision: Precision::Approximate { error_bound: None },
        },
        degradation: TessellationDegradation {
            missing_faces: vec![FaceRef {
                id: 0,
                reason: "unsupported surface".into(),
            }],
            ..TessellationDegradation::default()
        },
        diagnostics: vec![],
    };
    assert!(outcome.mesh().is_some());
    assert!(outcome.degradation().unwrap().missing_faces.len() == 1);
    assert!(!outcome.is_success());
}

#[test]
fn entry_point_routes_through_the_seam() {
    let req = request(SolidExchange::Sat(b"ACIS".to_vec()));
    let result = tessellate_solid(&NoKernelTessellator, &req, &no_cancel).unwrap();
    assert_eq!(result.stamp, req.stamp);
    assert!(matches!(
        result.outcome,
        TessellationOutcome::Unsupported { .. }
    ));
}

#[test]
fn default_registration_claims_no_capability() {
    let registration = NoKernelTessellator.registration();
    assert_eq!(registration.type_key, "yacr.kernel.unsupported");
    assert!(registration.capabilities.is_empty());
}

// ---- Neutral B-rep tessellation (real mesh subset) ----

fn pt(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line_loop(pts: &[[f64; 3]]) -> BrepLoop {
    BrepLoop {
        edges: (0..pts.len())
            .map(|i| {
                let a = pts[i];
                let b = pts[(i + 1) % pts.len()];
                BrepCurve::Line {
                    start: pt(a[0], a[1], a[2]),
                    end: pt(b[0], b[1], b[2]),
                }
            })
            .collect(),
    }
}

fn plane_face(id: u32, origin: [f64; 3], normal: [f64; 3], loop_pts: &[[f64; 3]]) -> BrepFace {
    BrepFace {
        id,
        surface: BrepSurface::Plane {
            origin: pt(origin[0], origin[1], origin[2]),
            normal: pt(normal[0], normal[1], normal[2]),
            u_dir: pt(1.0, 0.0, 0.0),
        },
        reversed: false,
        loops: vec![line_loop(loop_pts)],
    }
}

fn cube_brep(size: f64) -> BrepData {
    let s = size;
    let faces = vec![
        plane_face(
            0,
            [0.0, 0.0, 0.0],
            [0.0, 0.0, -1.0],
            &[[0.0, 0.0, 0.0], [0.0, s, 0.0], [s, s, 0.0], [s, 0.0, 0.0]],
        ),
        plane_face(
            1,
            [0.0, 0.0, s],
            [0.0, 0.0, 1.0],
            &[[0.0, 0.0, s], [s, 0.0, s], [s, s, s], [0.0, s, s]],
        ),
        plane_face(
            2,
            [0.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            &[[0.0, 0.0, 0.0], [0.0, 0.0, s], [0.0, s, s], [0.0, s, 0.0]],
        ),
        plane_face(
            3,
            [s, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            &[[s, 0.0, 0.0], [s, s, 0.0], [s, s, s], [s, 0.0, s]],
        ),
        plane_face(
            4,
            [0.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            &[[0.0, 0.0, 0.0], [s, 0.0, 0.0], [s, 0.0, s], [0.0, 0.0, s]],
        ),
        plane_face(
            5,
            [0.0, s, 0.0],
            [0.0, 1.0, 0.0],
            &[[0.0, s, 0.0], [0.0, s, s], [s, s, s], [s, s, 0.0]],
        ),
    ];
    BrepData {
        shells: vec![BrepShell { id: 0, faces }],
        placement: None,
    }
}

fn brep_request(brep: BrepData) -> TessellationRequest {
    request(SolidExchange::Brep(brep))
}

#[test]
fn brep_cube_is_a_closed_success() {
    let req = brep_request(cube_brep(1.0));
    let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
    let mesh = result.outcome.mesh().expect("cube must produce a mesh");
    assert_eq!(mesh.triangle_count(), 12, "cube face count / triangles");
    assert_eq!(mesh.precision, Precision::Analytic);
    let area = brep::mesh_area(&mesh.mesh);
    assert!((area - 6.0).abs() < 1e-9, "cube area {area}");
    assert!(matches!(
        result.outcome,
        TessellationOutcome::Success { .. }
    ));
}

#[test]
fn brep_cube_placement_is_applied() {
    let mut brep = cube_brep(1.0);
    brep.placement = Some(BrepPlacement {
        matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: pt(10.0, 0.0, 0.0),
        scale: 1.0,
    });
    let req = brep_request(brep);
    let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
    let mesh = result.outcome.mesh().unwrap();
    assert!(mesh.mesh.vertices.iter().all(|p| p.x >= 10.0 - 1e-9));
}

#[test]
fn brep_sphere_is_closed_and_monotone_in_tolerance() {
    let sphere = |tolerance: f64| {
        let brep = BrepData {
            shells: vec![BrepShell {
                id: 0,
                faces: vec![BrepFace {
                    id: 0,
                    surface: BrepSurface::Sphere {
                        center: pt(0.0, 0.0, 0.0),
                        radius: 1.0,
                        u_dir: pt(1.0, 0.0, 0.0),
                        pole: pt(0.0, 0.0, 1.0),
                    },
                    reversed: false,
                    loops: Vec::new(),
                }],
            }],
            placement: None,
        };
        let mut req = brep_request(brep);
        req.tolerance.linear_deflection = tolerance;
        let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
        assert!(
            matches!(result.outcome, TessellationOutcome::Success { .. }),
            "sphere must be a closed success: {:?}",
            result.outcome
        );
        let mesh = result.outcome.mesh().unwrap();
        assert!(mesh.triangle_count() > 0);
        let err = match mesh.precision {
            Precision::Approximate {
                error_bound: Some(e),
            } => e,
            ref other => panic!("sphere must report a chordal bound, got {other:?}"),
        };
        (mesh.triangle_count(), err)
    };
    let (coarse_tris, coarse_err) = sphere(0.5);
    let (fine_tris, fine_err) = sphere(0.05);
    assert!(
        fine_tris >= coarse_tris,
        "finer tolerance must not coarsen: {fine_tris} < {coarse_tris}"
    );
    assert!(
        fine_err <= coarse_err + 1e-12,
        "finer tolerance must not increase error: {fine_err} > {coarse_err}"
    );
}

#[test]
fn brep_budget_exceeded_is_reported_not_truncated() {
    let mut brep = BrepData::default();
    brep.shells.push(BrepShell {
        id: 0,
        faces: vec![BrepFace {
            id: 0,
            surface: BrepSurface::Sphere {
                center: pt(0.0, 0.0, 0.0),
                radius: 1.0,
                u_dir: pt(1.0, 0.0, 0.0),
                pole: pt(0.0, 0.0, 1.0),
            },
            reversed: false,
            loops: Vec::new(),
        }],
    });
    let mut req = brep_request(brep);
    req.budget.max_faces = 2;
    let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
    let TessellationOutcome::Failed { diagnostics } = result.outcome else {
        panic!("over-budget sphere must fail, not truncate");
    };
    assert_eq!(diagnostics[0].code, codes::KERNEL_BUDGET_EXCEEDED);
}

#[test]
fn brep_partial_reports_missing_face_and_open_edge() {
    // A lone planar square plus one unsupported face: usable facets, with
    // the unsupported face and the square's open boundary both explicit.
    let brep = BrepData {
        shells: vec![BrepShell {
            id: 7,
            faces: vec![
                plane_face(
                    0,
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    &[
                        [0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0],
                        [1.0, 1.0, 0.0],
                        [0.0, 1.0, 0.0],
                    ],
                ),
                BrepFace {
                    id: 5,
                    surface: BrepSurface::Unsupported {
                        type_key: "nurbs-surface".into(),
                    },
                    reversed: false,
                    loops: Vec::new(),
                },
            ],
        }],
        placement: None,
    };
    let req = brep_request(brep);
    let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
    let TessellationOutcome::Partial {
        geometry,
        degradation,
        diagnostics,
    } = result.outcome
    else {
        panic!("expected Partial, got a different outcome");
    };
    assert_eq!(geometry.triangle_count(), 2);
    assert_eq!(degradation.missing_faces.len(), 1);
    assert_eq!(degradation.missing_faces[0].id, 5);
    assert!(!degradation.open_edges.is_empty());
    assert!(diagnostics
        .iter()
        .any(|d| d.code == codes::KERNEL_MISSING_FACE));
    assert!(diagnostics
        .iter()
        .any(|d| d.code == codes::KERNEL_OPEN_EDGE));
}

#[test]
fn brep_all_unsupported_is_unsupported_not_empty_success() {
    let brep = BrepData {
        shells: vec![BrepShell {
            id: 0,
            faces: vec![BrepFace {
                id: 0,
                surface: BrepSurface::Unsupported {
                    type_key: "spline-surface".into(),
                },
                reversed: false,
                loops: Vec::new(),
            }],
        }],
        placement: None,
    };
    let req = brep_request(brep);
    let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
    let TessellationOutcome::Unsupported { reason } = result.outcome else {
        panic!("all-unsupported B-rep must be Unsupported");
    };
    assert_eq!(reason.code, codes::KERNEL_UNSUPPORTED_SURFACE);
    assert!(!reason.detail.is_empty());
}

#[test]
fn empty_brep_is_failed_not_success() {
    let req = brep_request(BrepData::default());
    let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
    let TessellationOutcome::Failed { diagnostics } = result.outcome else {
        panic!("empty B-rep must fail");
    };
    assert_eq!(diagnostics[0].code, codes::KERNEL_EMPTY_GEOMETRY);
}

#[test]
fn brep_tessellator_never_parses_raw_bytes() {
    let req = request(SolidExchange::Sat(b"ACIS ...".to_vec()));
    let result = BrepTessellator.tessellate(&req, &no_cancel).unwrap();
    let TessellationOutcome::Unsupported { reason } = result.outcome else {
        panic!("raw SAT must stay unsupported");
    };
    assert_eq!(reason.code, codes::KERNEL_NO_ACIS_KERNEL);
    assert_eq!(reason.exchange, ExchangeKind::Sat);
}

#[test]
fn brep_tessellator_honours_cancellation() {
    let req = brep_request(cube_brep(1.0));
    assert_eq!(
        BrepTessellator.tessellate(&req, &|| true).unwrap_err(),
        CadError::Cancelled
    );
}
