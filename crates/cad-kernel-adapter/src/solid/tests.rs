//! Contract tests for the kernel-independent solid modelling seam.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cad_domain::{
    BrepCurve, BrepData, BrepFace, BrepLoop, BrepShell, BrepSurface, Diagnostic, DocumentId,
    Point3, TaskStamp, Transform3,
};

use super::*;
use crate::codes;

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn square_face(id: u32, normal: [f64; 3], corners: [[f64; 3]; 4]) -> BrepFace {
    // The frame only has to be non-parallel to the normal; the toy B-rep is
    // never tessellated in these tests.
    let u_dir = if normal[0].abs() > 0.5 {
        p(0.0, 1.0, 0.0)
    } else {
        p(1.0, 0.0, 0.0)
    };
    BrepFace {
        id,
        surface: BrepSurface::Plane {
            origin: p(corners[0][0], corners[0][1], corners[0][2]),
            normal: p(normal[0], normal[1], normal[2]),
            u_dir,
        },
        reversed: false,
        loops: vec![BrepLoop {
            edges: (0..4)
                .map(|i| {
                    let a = corners[i];
                    let b = corners[(i + 1) % 4];
                    BrepCurve::Line {
                        start: p(a[0], a[1], a[2]),
                        end: p(b[0], b[1], b[2]),
                    }
                })
                .collect(),
        }],
    }
}

/// A closed unit cube with outward-wound planar faces, as an exact operand.
fn cube_brep() -> BrepData {
    let faces = vec![
        square_face(
            0,
            [0.0, 0.0, -1.0],
            [
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
            ],
        ),
        square_face(
            1,
            [0.0, 0.0, 1.0],
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
        ),
        square_face(
            2,
            [-1.0, 0.0, 0.0],
            [
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [0.0, 1.0, 0.0],
            ],
        ),
        square_face(
            3,
            [1.0, 0.0, 0.0],
            [
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [1.0, 1.0, 1.0],
                [1.0, 0.0, 1.0],
            ],
        ),
        square_face(
            4,
            [0.0, -1.0, 0.0],
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
            ],
        ),
        square_face(
            5,
            [0.0, 1.0, 0.0],
            [
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, 1.0, 0.0],
            ],
        ),
    ];
    BrepData {
        shells: vec![BrepShell { id: 0, faces }],
        placement: None,
    }
}

fn stamp() -> TaskStamp {
    TaskStamp::new(DocumentId(1), 1)
}

fn request<'a>(operation: SolidOperation<'a>) -> SolidRequest<'a> {
    SolidRequest {
        stamp: stamp(),
        operation,
    }
}

fn box_spec() -> PrimitiveSpec {
    PrimitiveSpec::Box {
        origin: p(0.0, 0.0, 0.0),
        length: 1.0,
        width: 2.0,
        height: 3.0,
    }
}

fn cylinder_spec() -> PrimitiveSpec {
    PrimitiveSpec::Cylinder {
        origin: p(0.0, 0.0, 0.0),
        radius: 1.0,
        height: 2.0,
    }
}

fn assert_dimension_error(spec: PrimitiveSpec) {
    let error = spec
        .validate()
        .expect_err("degenerate primitive must be rejected");
    assert_eq!(
        error.code,
        codes::SOLID_INVALID_DIMENSION,
        "unexpected code for {spec:?}: {error}"
    );
}

// ---- Primitive validation ----

#[test]
fn primitive_validation_rejects_zero_negative_and_non_finite() {
    let o = p(0.0, 0.0, 0.0);
    assert_dimension_error(PrimitiveSpec::Box {
        origin: o,
        length: 0.0,
        width: 1.0,
        height: 1.0,
    });
    assert_dimension_error(PrimitiveSpec::Box {
        origin: o,
        length: -1.0,
        width: 1.0,
        height: 1.0,
    });
    assert_dimension_error(PrimitiveSpec::Box {
        origin: o,
        length: 1.0,
        width: 1.0,
        height: f64::NAN,
    });
    assert_dimension_error(PrimitiveSpec::Wedge {
        origin: o,
        length: 1.0,
        width: f64::INFINITY,
        height: 1.0,
    });
    assert_dimension_error(PrimitiveSpec::Pyramid {
        base_center: o,
        length: 0.0,
        height: 1.0,
    });
    assert_dimension_error(PrimitiveSpec::Cylinder {
        origin: o,
        radius: -2.0,
        height: 1.0,
    });
    assert_dimension_error(PrimitiveSpec::Sphere {
        origin: o,
        radius: f64::NAN,
    });
    assert_dimension_error(PrimitiveSpec::Cone {
        origin: o,
        radius_bottom: 0.0,
        radius_top: 0.0,
        height: 1.0,
    });
    assert_dimension_error(PrimitiveSpec::Cone {
        origin: o,
        radius_bottom: 1.0,
        radius_top: -1.0,
        height: 1.0,
    });
    assert_dimension_error(PrimitiveSpec::Torus {
        origin: o,
        major_radius: 1.0,
        minor_radius: 0.0,
    });
}

#[test]
fn primitive_validation_rejects_non_finite_origin() {
    let error = PrimitiveSpec::Sphere {
        origin: p(f64::NAN, 0.0, 0.0),
        radius: 1.0,
    }
    .validate()
    .expect_err("non-finite origin must be rejected");
    assert_eq!(error.code, codes::SOLID_INVALID_POINT);
    assert!(error.message.contains("origin.x"));
}

#[test]
fn primitive_validation_accepts_well_formed_specs() {
    let o = p(0.0, 0.0, 0.0);
    let specs = [
        box_spec(),
        PrimitiveSpec::Wedge {
            origin: o,
            length: 1.0,
            width: 1.0,
            height: 1.0,
        },
        PrimitiveSpec::Pyramid {
            base_center: o,
            length: 1.0,
            height: 1.0,
        },
        cylinder_spec(),
        PrimitiveSpec::Sphere {
            origin: o,
            radius: 1.0,
        },
        // A full cone (apex) is valid with `radius_top == 0`.
        PrimitiveSpec::Cone {
            origin: o,
            radius_bottom: 1.0,
            radius_top: 0.0,
            height: 1.0,
        },
        PrimitiveSpec::Torus {
            origin: o,
            major_radius: 2.0,
            minor_radius: 0.5,
        },
    ];
    for spec in specs {
        spec.validate().unwrap_or_else(|e| panic!("{spec:?}: {e}"));
    }
}

// ---- Request validation ----

#[test]
fn request_validation_rejects_bad_density() {
    let brep = cube_brep();
    for density in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let req = request(SolidOperation::MassProperties {
            brep: &brep,
            density,
        });
        let error = req
            .validate()
            .expect_err("non-positive density must be rejected");
        assert_eq!(error.code, codes::SOLID_INVALID_DENSITY);
    }
    request(SolidOperation::MassProperties {
        brep: &brep,
        density: 7.85,
    })
    .validate()
    .expect("a positive density is valid");
}

#[test]
fn request_validation_rejects_empty_operands() {
    let empty = BrepData::default();
    let solid = cube_brep();
    for req in [
        request(SolidOperation::Boolean {
            op: BooleanOp::Union,
            left: &empty,
            right: &solid,
        }),
        request(SolidOperation::Boolean {
            op: BooleanOp::Subtract,
            left: &solid,
            right: &empty,
        }),
        request(SolidOperation::Transform {
            brep: &empty,
            transform: &Transform3::identity(),
        }),
        request(SolidOperation::MassProperties {
            brep: &empty,
            density: 1.0,
        }),
    ] {
        let error = req.validate().expect_err("empty operand must be rejected");
        assert_eq!(error.code, codes::SOLID_EMPTY_OPERAND);
    }
}

#[test]
fn request_validation_rejects_invalid_transforms() {
    let brep = cube_brep();
    let identity = Transform3::identity();

    let mut non_finite = identity.matrix;
    non_finite[0][0] = f64::NAN;
    let mut non_affine = identity.matrix;
    non_affine[3][3] = 2.0;

    for transform in [
        Transform3 { matrix: non_finite },
        Transform3 { matrix: non_affine },
        // Singular: a zero uniform scale collapses the body.
        Transform3::scale(0.0),
    ] {
        let req = request(SolidOperation::Transform {
            brep: &brep,
            transform: &transform,
        });
        let error = req
            .validate()
            .expect_err("invalid transform must be rejected");
        assert_eq!(error.code, codes::SOLID_INVALID_TRANSFORM);
    }

    request(SolidOperation::Transform {
        brep: &brep,
        transform: &Transform3::translation(p(10.0, 0.0, 0.0)),
    })
    .validate()
    .expect("an affine translation is valid");
}

// ---- Capability probing ----

#[test]
fn capability_probe_reports_per_family_flags() {
    let brep = cube_brep();
    let transform = Transform3::identity();
    let box_op = SolidOperation::Primitive { spec: box_spec() };
    let cylinder_op = SolidOperation::Primitive {
        spec: cylinder_spec(),
    };
    let boolean_op = SolidOperation::Boolean {
        op: BooleanOp::Union,
        left: &brep,
        right: &brep,
    };
    let transform_op = SolidOperation::Transform {
        brep: &brep,
        transform: &transform,
    };
    let mass_op = SolidOperation::MassProperties {
        brep: &brep,
        density: 1.0,
    };

    let mut caps = ModelerCapabilities::default();
    assert!(!caps.supports(&box_op));
    assert!(!caps.supports(&cylinder_op));
    assert!(!caps.supports(&boolean_op));
    assert!(!caps.supports(&mass_op));
    // A similarity transform is the seam's baseline, not a declared capability.
    assert!(caps.supports(&transform_op));

    caps.primitives.box_primitive = true;
    assert!(caps.supports(&box_op));
    assert!(!caps.supports(&cylinder_op));

    caps.primitives = ModelerPrimitives::all();
    assert!(caps.supports(&cylinder_op));
    assert_eq!(box_op.primitive_kind(), Some(PrimitiveKind::Box));
    assert_eq!(boolean_op.primitive_kind(), None);

    caps.booleans = true;
    assert!(caps.supports(&boolean_op));
    caps.mass_properties = true;
    assert!(caps.supports(&mass_op));

    // Family tags are stable contract values.
    assert_eq!(box_op.family().as_str(), "primitive");
    assert_eq!(boolean_op.family(), SolidOperationFamily::Boolean);
    assert_eq!(transform_op.family().as_str(), "transform");
    assert_eq!(mass_op.family().as_str(), "mass_properties");
}

// ---- Outcome accessors ----

#[test]
fn outcome_accessors_separate_body_and_mass() {
    let note = Diagnostic {
        object: None,
        code: "test.note".into(),
        message: "non-fatal note".into(),
    };

    let body = SolidOutcome::Success(SolidProduct::Body {
        brep: cube_brep(),
        diagnostics: vec![note.clone()],
    });
    assert!(body.is_success());
    assert!(body.product().is_some());
    assert_eq!(body.body().map(BrepData::face_count), Some(6));
    assert!(body.mass().is_none());
    assert_eq!(body.diagnostics().len(), 1);
    assert!(body.unsupported().is_none());

    let mass = SolidOutcome::Success(SolidProduct::Mass(SolidMassProperties {
        volume: 1.0,
        surface_area: 6.0,
        centroid: p(0.5, 0.5, 0.5),
        inertia: None,
        density: 2.5,
        accuracy: MassAccuracy::Requested { relative: 1e-6 },
    }));
    assert!(mass.is_success());
    assert!(mass.body().is_none());
    assert_eq!(mass.mass().expect("mass product").density, 2.5);
    assert!(mass.diagnostics().is_empty());

    let unsupported = SolidOutcome::Unsupported {
        reason: SolidUnsupportedReason {
            code: codes::SOLID_UNSUPPORTED,
            operation: SolidOperationFamily::Boolean,
            detail: "no boolean kernel".into(),
        },
    };
    assert!(!unsupported.is_success());
    assert!(unsupported.product().is_none());
    assert!(unsupported.body().is_none());
    assert!(unsupported.mass().is_none());
    assert!(unsupported.diagnostics().is_empty());
    assert_eq!(
        unsupported.unsupported().map(|r| r.code),
        Some(codes::SOLID_UNSUPPORTED)
    );

    let failed = SolidOutcome::Failed {
        diagnostics: vec![Diagnostic {
            object: None,
            code: codes::SOLID_FAILED.into(),
            message: "empty boolean result".into(),
        }],
    };
    assert!(!failed.is_success());
    assert!(failed.product().is_none());
    assert_eq!(failed.diagnostics().len(), 1);
    assert!(failed.unsupported().is_none());
}

// ---- A toy modeler: object safety and Send + Sync ----

struct ToyModeler {
    caps: ModelerCapabilities,
}

impl ToyModeler {
    fn new() -> Self {
        let primitives = ModelerPrimitives {
            box_primitive: true,
            ..ModelerPrimitives::default()
        };
        ToyModeler {
            caps: ModelerCapabilities {
                primitives,
                booleans: false,
                mass_properties: true,
                fillet: false,
                shell: false,
                nurbs: false,
            },
        }
    }
}

impl SolidModeler for ToyModeler {
    fn registration(&self) -> cad_domain::Registration {
        cad_domain::Registration {
            type_key: "test.toy.solid-modeler".into(),
            version: 1,
            priority: 0,
            entity_types: vec!["3DSOLID".into(), "BODY".into()],
            capabilities: vec!["primitives.box".into()],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        self.caps
    }

    fn execute(
        &self,
        request: &SolidRequest<'_>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> cad_domain::CadResult<SolidResult> {
        // Pre-check: cancellation wins over validation and kernel work.
        if cancelled() {
            return Err(cad_domain::CadError::Cancelled);
        }
        request
            .validate()
            .map_err(|e| cad_domain::CadError::InvalidInput(e.to_string()))?;

        let outcome = if !self.caps.supports(&request.operation) {
            SolidOutcome::Unsupported {
                reason: SolidUnsupportedReason {
                    code: codes::SOLID_UNSUPPORTED,
                    operation: request.operation.family(),
                    detail: "toy modeler does not claim this capability".into(),
                },
            }
        } else {
            match &request.operation {
                SolidOperation::Primitive { .. } => SolidOutcome::Success(SolidProduct::Body {
                    brep: cube_brep(),
                    diagnostics: Vec::new(),
                }),
                SolidOperation::MassProperties { density, .. } => {
                    SolidOutcome::Success(SolidProduct::Mass(SolidMassProperties {
                        volume: 1.0,
                        surface_area: 6.0,
                        centroid: p(0.5, 0.5, 0.5),
                        inertia: Some([[0.0; 3]; 3]),
                        density: *density,
                        accuracy: MassAccuracy::Requested { relative: 1e-6 },
                    }))
                }
                other => SolidOutcome::Failed {
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: codes::SOLID_FAILED.into(),
                        message: format!("toy modeler has no {} path", other.family()),
                    }],
                },
            }
        };

        // Post-check: a result that arrived after cancellation is dropped.
        if cancelled() {
            return Err(cad_domain::CadError::Cancelled);
        }
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}

fn assert_send_sync<T: Send + Sync + ?Sized>() {}

#[test]
fn toy_modeler_is_object_safe_and_shareable_across_threads() {
    // Compile-time: the trait is object safe and its trait object is Send + Sync.
    assert_send_sync::<dyn SolidModeler>();

    let modeler: Arc<dyn SolidModeler> = Arc::new(ToyModeler::new());
    assert_eq!(modeler.registration().type_key, "test.toy.solid-modeler");

    let worker = {
        let modeler = Arc::clone(&modeler);
        std::thread::spawn(move || {
            let req = request(SolidOperation::Primitive { spec: box_spec() });
            let result = execute_solid(modeler.as_ref(), &req, &|| false).expect("box operation");
            assert_eq!(result.stamp, req.stamp);
            result.outcome.body().map(BrepData::face_count)
        })
    };
    assert_eq!(worker.join().expect("worker thread"), Some(6));

    // The same shared modeler answers mass properties from the main thread.
    let brep = cube_brep();
    let req = request(SolidOperation::MassProperties {
        brep: &brep,
        density: 7.85,
    });
    let result = execute_solid(modeler.as_ref(), &req, &|| false).expect("mass operation");
    assert_eq!(result.outcome.mass().map(|m| m.density), Some(7.85));
}

#[test]
fn toy_modeler_refuses_undeclared_capability() {
    let modeler = ToyModeler::new();
    let brep = cube_brep();
    let req = request(SolidOperation::Boolean {
        op: BooleanOp::Union,
        left: &brep,
        right: &brep,
    });
    let result = modeler.execute(&req, &|| false).expect("unsupported is Ok");
    let SolidOutcome::Unsupported { reason } = &result.outcome else {
        panic!("expected Unsupported, got {:?}", result.outcome);
    };
    assert_eq!(reason.code, codes::SOLID_UNSUPPORTED);
    assert_eq!(reason.operation, SolidOperationFamily::Boolean);
    assert!(!reason.detail.is_empty());
    assert!(result.outcome.diagnostics().is_empty());
    assert!(result.outcome.body().is_none());
}

#[test]
fn toy_modeler_maps_bad_input_to_invalid_input() {
    let modeler = ToyModeler::new();
    let req = request(SolidOperation::Primitive {
        spec: PrimitiveSpec::Box {
            origin: p(0.0, 0.0, 0.0),
            length: 0.0,
            width: 1.0,
            height: 1.0,
        },
    });
    match modeler
        .execute(&req, &|| false)
        .expect_err("zero length must be InvalidInput")
    {
        cad_domain::CadError::InvalidInput(message) => {
            assert!(
                message.contains(codes::SOLID_INVALID_DIMENSION),
                "{message}"
            );
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn toy_modeler_honours_pre_and_post_cancellation() {
    let modeler = ToyModeler::new();
    let req = request(SolidOperation::Primitive { spec: box_spec() });

    // Pre-cancel wins even when the request is also invalid.
    let invalid = request(SolidOperation::Primitive {
        spec: PrimitiveSpec::Sphere {
            origin: p(0.0, 0.0, 0.0),
            radius: f64::NAN,
        },
    });
    assert_eq!(
        modeler.execute(&invalid, &|| true).unwrap_err(),
        cad_domain::CadError::Cancelled
    );

    // Post-cancel: the kernel call happened, the result is dropped.
    let calls = AtomicUsize::new(0);
    let cancel_after_start = || calls.fetch_add(1, Ordering::SeqCst) >= 1;
    assert_eq!(
        modeler.execute(&req, &cancel_after_start).unwrap_err(),
        cad_domain::CadError::Cancelled
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "pre and post checks both ran"
    );
}
