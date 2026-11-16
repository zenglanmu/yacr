//! End-to-end chain tests for the `solidcore` modeler: create → boolean →
//! mass properties, plus the refusals that keep the capability claim honest.
//!
//! These run the **real** pinned kernel (no mock, no golden): every volume below
//! is integrated by `solidcore` from the body this crate produced, so a wrong
//! conversion or a wrong winding shows up as a wrong number. Evidence boundary:
//! Linux x86-64, this repository's test profile. No GPU, no host, no other
//! platform is involved (ADR-0036).

mod common;

use std::sync::Arc;

use cad_domain::{BrepSurface, CadError, DocumentId, TaskStamp, Transform3};
use cad_kernel_adapter::{
    execute_solid, BooleanOp, PrimitiveKind, PrimitiveSpec, SolidEdgeSelector, SolidModeler,
    SolidOperation, SolidOutcome, SolidRequest, SolidResult,
};
use cad_kernel_solidcore::{
    SolidCoreModeler, CHAMFER_FAILED, EDGE_NOT_FOUND, LIFT_REQUIRES_PLANAR, LIFT_UNSUPPORTED,
    NONUNIFORM_TRANSFORM, TYPE_KEY, UPSTREAM_VERSION,
};

use common::*;

#[test]
fn registration_and_capabilities_are_honest() {
    let modeler = SolidCoreModeler::new();
    let registration = modeler.registration();
    assert_eq!(registration.type_key, TYPE_KEY);
    assert_eq!(registration.priority, 10);
    // The modeler only handles closed solids: `REGION`/`SURFACE` are sheet
    // bodies it neither creates nor consumes (P2's SURFACE family).
    assert_eq!(registration.entity_types, ["3DSOLID", "BODY"]);
    assert!(
        registration
            .capabilities
            .iter()
            .any(|entry| entry == &format!("solidcore.version={UPSTREAM_VERSION}")),
        "the pinned upstream version is advertised: {:?}",
        registration.capabilities
    );

    let capabilities = modeler.capabilities();
    for kind in [
        PrimitiveKind::Box,
        PrimitiveKind::Wedge,
        PrimitiveKind::Pyramid,
        PrimitiveKind::Cylinder,
        PrimitiveKind::Sphere,
        PrimitiveKind::Cone,
        PrimitiveKind::Torus,
    ] {
        assert!(
            capabilities.primitives.supports(kind),
            "{} must be declared",
            kind.as_str()
        );
    }
    assert!(capabilities.booleans);
    assert!(capabilities.mass_properties);
    // P2.0: exactly the straight chamfer on one edge, nothing else from P2.
    assert!(capabilities.chamfer);
    assert!(!capabilities.fillet && !capabilities.shell && !capabilities.nurbs);
    assert!(
        registration
            .capabilities
            .iter()
            .any(|entry| entry == "chamfer.sides"),
        "the chamfer capability is advertised: {:?}",
        registration.capabilities
    );
}

#[test]
fn object_safe_and_shareable_as_a_trait_object() {
    fn assert_send_sync<T: Send + Sync + ?Sized>() {}
    assert_send_sync::<dyn SolidModeler>();

    let modeler: Arc<dyn SolidModeler> = Arc::new(SolidCoreModeler::new());
    let brep = body(execute(
        modeler.as_ref(),
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 2.0, 3.0),
        },
    ));
    assert_eq!(brep.face_count(), 6);
    assert!((volume(modeler.as_ref(), &brep) - 6.0).abs() < 1e-9);
}

#[test]
fn authored_box_wedge_and_pyramid_have_their_closed_form_volumes() {
    let modeler = SolidCoreModeler::new();

    let box_brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 2.0, 3.0, 4.0),
        },
    ));
    assert_eq!(box_brep.face_count(), 6);
    assert!((volume(&modeler, &box_brep) - 24.0).abs() < 1e-9);

    let wedge_brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Wedge {
                origin: p(0.0, 0.0, 0.0),
                length: 2.0,
                width: 3.0,
                height: 4.0,
            },
        },
    ));
    assert_eq!(wedge_brep.face_count(), 5);
    assert!((volume(&modeler, &wedge_brep) - 12.0).abs() < 1e-9);

    let pyramid_brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Pyramid {
                base_center: p(0.0, 0.0, 0.0),
                length: 2.0,
                height: 3.0,
            },
        },
    ));
    assert_eq!(pyramid_brep.face_count(), 5);
    assert!((volume(&modeler, &pyramid_brep) - 4.0).abs() < 1e-9);
}

#[test]
fn subtract_corner_leaves_eleven_eighths() {
    let modeler = SolidCoreModeler::new();
    let unit = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    let corner = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.5, 0.5, 0.5), 0.5, 0.5, 0.5),
        },
    ));
    let cut = body(execute(
        &modeler,
        SolidOperation::Boolean {
            op: BooleanOp::Subtract,
            left: &unit,
            right: &corner,
        },
    ));
    // The result is still all-planar: it must lift again for mass properties.
    assert!((volume(&modeler, &cut) - 0.875).abs() < 1e-9);
}

#[test]
fn a_through_hole_keeps_its_inner_loop_and_measures_exactly() {
    let modeler = SolidCoreModeler::new();
    // A 2 x 2 x 2 cube minus a 1 x 1 column that pokes out of both faces.
    let outer = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 2.0, 2.0, 2.0),
        },
    ));
    let hole = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.5, 0.5, -0.5), 1.0, 1.0, 3.0),
        },
    ));
    let cut = body(execute(
        &modeler,
        SolidOperation::Boolean {
            op: BooleanOp::Subtract,
            left: &outer,
            right: &hole,
        },
    ));

    // The top and bottom faces each carry the hole as an inner loop. This is the
    // shape that proves the closed-shell invariant does not over-reject holes.
    let inner_loops: usize = cut
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .map(|face| face.loops.len().saturating_sub(1))
        .sum();
    assert_eq!(inner_loops, 2, "top and bottom each gain one hole loop");

    let mass = *execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &cut,
            density: 1.0,
        },
    )
    .mass()
    .expect("the through-hole body lifts again");
    // Volume 8 - 2. Surface 24 (outer) - 2 (the two 1 x 1 openings) + 8 (the
    // four 1 x 2 hole walls) = 30.
    assert!(
        (mass.volume - 6.0).abs() < 1e-6,
        "volume {} != 6",
        mass.volume
    );
    assert!(
        (mass.surface_area - 30.0).abs() < 1e-5,
        "area {} != 30",
        mass.surface_area
    );
}

#[test]
fn a_disjoint_union_is_a_body_whose_next_lift_is_refused() {
    // The asymmetry the docs describe: a multi-shell result is a legitimate
    // body, so the boolean succeeds; the refusal arrives on the *next* lift
    // (mass properties here, another boolean below), not at the boolean.
    let modeler = SolidCoreModeler::new();
    let left = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    let right = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(5.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    let union = body(execute(
        &modeler,
        SolidOperation::Boolean {
            op: BooleanOp::Union,
            left: &left,
            right: &right,
        },
    ));
    assert_eq!(
        union.shells.len(),
        2,
        "a disjoint union is returned with two closed shells"
    );

    let mass = execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &union,
            density: 1.0,
        },
    );
    assert_eq!(unsupported_code(&mass), LIFT_UNSUPPORTED);
    assert!(mass.mass().is_none(), "no summed volume");

    let again = execute(
        &modeler,
        SolidOperation::Boolean {
            op: BooleanOp::Union,
            left: &union,
            right: &left,
        },
    );
    assert_eq!(unsupported_code(&again), LIFT_UNSUPPORTED);
}

#[test]
fn a_cavity_result_is_refused_at_the_boolean_itself() {
    // The other half of the asymmetry: a *void region* (a fully enclosed cavity)
    // cannot be mapped on the way in, so the boolean that produces it refuses
    // immediately instead of handing out a body.
    let modeler = SolidCoreModeler::new();
    let shell = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 3.0, 3.0, 3.0),
        },
    ));
    let cavity = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(1.0, 1.0, 1.0), 1.0, 1.0, 1.0),
        },
    ));
    let outcome = execute(
        &modeler,
        SolidOperation::Boolean {
            op: BooleanOp::Subtract,
            left: &shell,
            right: &cavity,
        },
    );
    assert_eq!(unsupported_code(&outcome), LIFT_UNSUPPORTED);
    assert!(outcome.body().is_none(), "no body for an unmappable cavity");
}

#[test]
fn empty_intersection_is_failed_never_a_success() {
    let modeler = SolidCoreModeler::new();
    let left = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    let right = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(5.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    let outcome = execute(
        &modeler,
        SolidOperation::Boolean {
            op: BooleanOp::Intersect,
            left: &left,
            right: &right,
        },
    );
    assert!(!outcome.is_success(), "{outcome:?}");
    assert!(outcome.body().is_none());
    let SolidOutcome::Failed { diagnostics } = &outcome else {
        panic!("expected Failed, got {outcome:?}");
    };
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, cad_kernel_solidcore::EMPTY_RESULT);
}

#[test]
fn curved_primitives_map_analytic_surfaces_and_refuse_kernel_re_entry() {
    let modeler = SolidCoreModeler::new();
    let cases: [(PrimitiveSpec, &str); 4] = [
        (
            PrimitiveSpec::Cylinder {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
                height: 2.0,
            },
            "cylinder",
        ),
        (
            PrimitiveSpec::Sphere {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
            },
            "sphere",
        ),
        (
            PrimitiveSpec::Cone {
                origin: p(0.0, 0.0, 0.0),
                radius_bottom: 1.0,
                radius_top: 0.5,
                height: 2.0,
            },
            "cone",
        ),
        (
            PrimitiveSpec::Torus {
                origin: p(0.0, 0.0, 0.0),
                major_radius: 2.0,
                minor_radius: 0.5,
            },
            "torus",
        ),
    ];
    for (spec, name) in cases {
        let brep = body(execute(&modeler, SolidOperation::Primitive { spec }));
        // The curved face keeps its analytic kind: display and export get an exact
        // surface, not an unsupported placeholder.
        let analytic = brep
            .shells
            .iter()
            .flat_map(|shell| &shell.faces)
            .any(|face| {
                matches!(
                    (name, &face.surface),
                    ("cylinder", BrepSurface::Cylinder { .. })
                        | ("sphere", BrepSurface::Sphere { .. })
                        | ("cone", BrepSurface::Cone { .. })
                        | ("torus", BrepSurface::Torus { .. })
                )
            });
        assert!(analytic, "{name}: no analytic surface in the mapped body");

        // A boolean has to lift the body back into the kernel, which needs the
        // curved shell's NURBS form and pcurves the neutral model does not carry
        // yet. Refused with a stable reason, never approximated.
        let other = body(execute(
            &modeler,
            SolidOperation::Primitive {
                spec: box_spec(p(-2.0, -2.0, -2.0), 1.0, 1.0, 1.0),
            },
        ));
        let outcome = execute(
            &modeler,
            SolidOperation::Boolean {
                op: BooleanOp::Union,
                left: &brep,
                right: &other,
            },
        );
        assert_eq!(
            unsupported_code(&outcome),
            LIFT_REQUIRES_PLANAR,
            "{name}: boolean must refuse the curved lift"
        );
        assert!(outcome.body().is_none());

        // Mass properties take the same lift gate.
        let mass = execute(
            &modeler,
            SolidOperation::MassProperties {
                brep: &brep,
                density: 1.0,
            },
        );
        assert_eq!(
            unsupported_code(&mass),
            LIFT_REQUIRES_PLANAR,
            "{name}: mass must refuse the curved lift"
        );
        assert!(mass.mass().is_none());
    }
}

#[test]
fn mass_properties_of_a_unit_box_are_its_closed_forms() {
    let modeler = SolidCoreModeler::new();
    let brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    let outcome = execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &brep,
            density: 7.85,
        },
    );
    let mass = outcome.mass().expect("mass product");
    assert!((mass.volume - 1.0).abs() < 1e-9, "volume {}", mass.volume);
    assert!(
        (mass.surface_area - 6.0).abs() < 1e-6,
        "area {}",
        mass.surface_area
    );
    assert!(
        (mass.centroid.x - 0.5).abs() < 1e-9
            && (mass.centroid.y - 0.5).abs() < 1e-9
            && (mass.centroid.z - 0.5).abs() < 1e-9,
        "centroid {:?}",
        mass.centroid
    );
    assert_eq!(mass.density, 7.85);
    // Requested, never proved: the seam must not claim an analytic bound.
    assert!(matches!(
        mass.accuracy,
        cad_kernel_adapter::MassAccuracy::Requested { .. }
    ));
    // Inertia is the **mass** inertia at the requested density: the kernel's
    // unit-density tensor (diag((1+1)/12)) scaled by 7.85.
    let inertia = mass.inertia.expect("inertia tensor");
    for (axis, row) in inertia.iter().enumerate() {
        for (column, value) in row.iter().enumerate() {
            let expected = if axis == column { 7.85 / 6.0 } else { 0.0 };
            assert!(
                (value - expected).abs() < 1e-5,
                "inertia[{axis}][{column}] = {value}, expected {expected}"
            );
        }
    }
}

#[test]
fn density_scales_the_inertia_tensor_but_not_the_geometry() {
    let modeler = SolidCoreModeler::new();
    let brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    let at = |density: f64| {
        *execute(
            &modeler,
            SolidOperation::MassProperties {
                brep: &brep,
                density,
            },
        )
        .mass()
        .expect("mass product")
    };
    let unit = at(1.0);
    let doubled = at(2.0);
    assert!(
        (doubled.volume - unit.volume).abs() < 1e-12,
        "volume is geometric"
    );
    assert!(
        (doubled.surface_area - unit.surface_area).abs() < 1e-12,
        "area is geometric"
    );
    assert_eq!(doubled.centroid, unit.centroid, "centroid is geometric");
    let unit_tensor = unit.inertia.expect("unit tensor");
    let doubled_tensor = doubled.inertia.expect("doubled tensor");
    for axis in 0..3 {
        for column in 0..3 {
            assert!(
                (doubled_tensor[axis][column] - 2.0 * unit_tensor[axis][column]).abs() < 1e-9,
                "inertia[{axis}][{column}] must double at density 2"
            );
        }
    }
}

#[test]
fn similarity_transform_scales_a_body_and_non_uniform_is_refused() {
    let modeler = SolidCoreModeler::new();
    let brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    let scaled = body(execute(
        &modeler,
        SolidOperation::Transform {
            brep: &brep,
            transform: &Transform3::scale(2.0),
        },
    ));
    assert!((volume(&modeler, &scaled) - 8.0).abs() < 1e-9);

    let mut shear = Transform3::identity().matrix;
    shear[0][1] = 1.0;
    let mut non_uniform = Transform3::identity().matrix;
    non_uniform[0][0] = 2.0;
    for transform in [
        Transform3 { matrix: shear },
        Transform3 {
            matrix: non_uniform,
        },
    ] {
        let outcome = execute(
            &modeler,
            SolidOperation::Transform {
                brep: &brep,
                transform: &transform,
            },
        );
        assert_eq!(unsupported_code(&outcome), NONUNIFORM_TRANSFORM);
        assert!(outcome.body().is_none());
    }
}

#[test]
fn pre_cancelled_request_returns_cancelled() {
    let modeler = SolidCoreModeler::new();
    let primitive = request(SolidOperation::Primitive {
        spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
    });
    assert_eq!(
        execute_solid(&modeler, &primitive, &|| true).unwrap_err(),
        CadError::Cancelled
    );
    // Cancellation also wins over an invalid request.
    let invalid = request(SolidOperation::Primitive {
        spec: box_spec(p(0.0, 0.0, 0.0), 0.0, 1.0, 1.0),
    });
    assert_eq!(
        execute_solid(&modeler, &invalid, &|| true).unwrap_err(),
        CadError::Cancelled
    );
}

#[test]
fn invalid_input_is_rejected_before_kernel_work() {
    let modeler = SolidCoreModeler::new();
    let request = request(SolidOperation::Primitive {
        spec: box_spec(p(0.0, 0.0, 0.0), -1.0, 1.0, 1.0),
    });
    match execute_solid(&modeler, &request, &never()).unwrap_err() {
        CadError::InvalidInput(message) => {
            assert!(message.contains("solid.invalid_dimension"), "{message}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn two_threads_share_one_modeler_and_both_succeed() {
    let modeler: Arc<dyn SolidModeler> = Arc::new(SolidCoreModeler::new());
    let workers: Vec<_> = (0..2)
        .map(|index| {
            let modeler = Arc::clone(&modeler);
            std::thread::spawn(move || -> SolidResult {
                let request = SolidRequest {
                    stamp: TaskStamp::new(DocumentId(1), 1),
                    operation: SolidOperation::Primitive {
                        spec: box_spec(p(index as f64 * 4.0, 0.0, 0.0), 1.0, 2.0, 3.0),
                    },
                };
                execute_solid(modeler.as_ref(), &request, &never()).expect("box")
            })
        })
        .collect();
    let mut volumes = Vec::new();
    for worker in workers {
        let result = worker.join().expect("worker thread");
        let brep = result
            .outcome
            .product()
            .and_then(|product| product.body())
            .expect("body product")
            .clone();
        volumes.push(volume(modeler.as_ref(), &brep));
    }
    assert_eq!(volumes.len(), 2);
    for value in volumes {
        assert!((value - 6.0).abs() < 1e-9, "volume {value}");
    }
}

// ---- M1/M2 regressions: implausible bodies and their codes ----------------

#[test]
fn probe_a_reversed_face_is_refused_not_measured() {
    let modeler = SolidCoreModeler::new();
    let mut brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 2.0, 2.0, 2.0),
        },
    ));
    brep.shells[0].faces[0].reversed = true;
    let outcome = execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &brep,
            density: 1.0,
        },
    );
    assert_eq!(unsupported_code(&outcome), LIFT_UNSUPPORTED);
    assert!(outcome.mass().is_none(), "no number may be returned");
    assert!(outcome.body().is_none());
}

#[test]
fn probe_a_duplicated_face_is_refused_not_measured() {
    let modeler = SolidCoreModeler::new();
    let mut brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 2.0, 2.0, 2.0),
        },
    ));
    let duplicate = brep.shells[0].faces[0].clone();
    brep.shells[0].faces.push(duplicate);
    let outcome = execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &brep,
            density: 1.0,
        },
    );
    assert_eq!(unsupported_code(&outcome), LIFT_UNSUPPORTED);
    assert!(outcome.mass().is_none());
}

// ---- P2.0: straight chamfer on one planar edge ----------------------------

/// The unit cube's edge from (0, 0, 0) to (0, 1, 0): the bottom/left corner
/// along Y, shared by the `z = 0` and `x = 0` faces.
fn chamfer_selector() -> SolidEdgeSelector {
    SolidEdgeSelector::NearestToPoint(p(0.0, 0.5, 0.0))
}

#[test]
fn chamfered_unit_edge_removes_its_triangular_prism() {
    let modeler = SolidCoreModeler::new();
    let unit = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));

    let distance = 0.1;
    let chamfered = body(execute(
        &modeler,
        SolidOperation::Chamfer {
            brep: &unit,
            selector: chamfer_selector(),
            distance,
            second: None,
        },
    ));

    // P2.0's whole point: a straight chamfer between two planes produces only
    // planes, so the neutral model can carry it and it re-lifts.
    assert!(
        faces(&chamfered).all(|face| matches!(face.surface, BrepSurface::Plane { .. })),
        "a straight chamfer must stay all-planar"
    );
    assert!(
        chamfered.face_count() > unit.face_count(),
        "the chamfer must add the cut face ({} -> {})",
        unit.face_count(),
        chamfered.face_count()
    );
    assert_eq!(chamfered.shells.len(), 1, "one closed shell");

    // Closed form for a symmetric 45 degree chamfer of one box edge:
    //   V = L * W * H - (d^2 / 2) * len(edge)
    // The cross-section perpendicular to the edge is a right isosceles triangle
    // with legs d (the two set-backs), extruded along the edge.
    let edge_length = 1.0;
    let expected = 1.0 - (distance * distance / 2.0) * edge_length;
    let mass = *execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &chamfered,
            density: 1.0,
        },
    )
    .mass()
    .expect("the chamfered body lifts again for mass properties");
    assert!(
        (mass.volume - expected).abs() < 1e-6,
        "volume {} != {expected}",
        mass.volume
    );

    // Watertight through this repository's own tessellator: no missing face, no
    // open edge, no dropped shell.
    match cad_kernel_adapter::tessellate_exchange(
        &cad_kernel_adapter::SolidExchange::Brep(chamfered.clone()),
        cad_kernel_adapter::TessellationTolerance::default(),
        cad_kernel_adapter::TessellationBudget::default(),
    ) {
        cad_kernel_adapter::TessellationOutcome::Success { geometry, .. } => {
            assert!(!geometry.is_empty(), "empty chamfer mesh");
        }
        other => panic!("chamfered body is not watertight: {other:?}"),
    }
}

#[test]
fn chamfer_with_an_asymmetric_second_distance_removes_its_prism() {
    // The non-symmetric path is a real kernel path, not a silent
    // "second = distance". For set-backs `a` along the first face and `b` along
    // the second (perpendicular planes), the removed prism's cross-section is a
    // right triangle with legs a and b, so
    //   V = L·W·H − (a·b / 2) · len(edge).
    // The formula is symmetric in a and b, so it holds whichever face the kernel
    // measures first; a kernel that ignored `second` would leave 0.995, not 0.99.
    let modeler = SolidCoreModeler::new();
    let unit = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    let chamfered = body(execute(
        &modeler,
        SolidOperation::Chamfer {
            brep: &unit,
            selector: chamfer_selector(),
            distance: 0.1,
            second: Some(0.2),
        },
    ));
    assert!(
        faces(&chamfered).all(|face| matches!(face.surface, BrepSurface::Plane { .. })),
        "an asymmetric straight chamfer must stay all-planar"
    );
    assert_eq!(chamfered.shells.len(), 1, "one closed shell");

    let expected = 1.0 - (0.1 * 0.2 / 2.0) * 1.0;
    let mass = *execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &chamfered,
            density: 1.0,
        },
    )
    .mass()
    .expect("the asymmetric chamfer lifts again");
    assert!(
        (mass.volume - expected).abs() < 1e-6,
        "asymmetric volume {} != {expected}",
        mass.volume
    );
}

#[test]
fn chamfer_on_a_wedge_base_edge_removes_its_prism() {
    // A second planar solid, so the capability is not "works on a box only".
    // The wedge's bottom face (z = 0) and back face (x = 0) meet at a convex
    // 90-degree dihedral along the edge from (0, 0, 0) to (0, 3, 0).
    let modeler = SolidCoreModeler::new();
    let length = 2.0;
    let width = 3.0;
    let height = 4.0;
    let wedge = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Wedge {
                origin: p(0.0, 0.0, 0.0),
                length,
                width,
                height,
            },
        },
    ));
    let distance = 0.1;
    let chamfered = body(execute(
        &modeler,
        SolidOperation::Chamfer {
            brep: &wedge,
            selector: SolidEdgeSelector::NearestToPoint(p(0.0, width / 2.0, 0.0)),
            distance,
            second: None,
        },
    ));
    assert!(
        faces(&chamfered).all(|face| matches!(face.surface, BrepSurface::Plane { .. })),
        "a chamfered wedge must stay all-planar"
    );
    assert_eq!(chamfered.shells.len(), 1, "one closed shell");

    // The wedge is a right triangular prism, V = L·W·H/2. The symmetric chamfer
    // removes a prism whose cross-section is a right isosceles triangle of legs
    // d along the edge's full length W:
    //   V = L·W·H/2 − (d² / 2) · W.
    let expected = length * width * height / 2.0 - (distance * distance / 2.0) * width;
    let mass = *execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &chamfered,
            density: 1.0,
        },
    )
    .mass()
    .expect("the chamfered wedge lifts again");
    assert!(
        (mass.volume - expected).abs() < 1e-6,
        "wedge volume {} != {expected}",
        mass.volume
    );
}

#[test]
fn chamfer_refuses_a_point_far_from_every_edge() {
    let modeler = SolidCoreModeler::new();
    let unit = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    // The cube's centre is 0.5 from every edge: far outside the pick tolerance
    // (1e-3 of the bounding-box diagonal), so nothing may be chamfered.
    let outcome = execute(
        &modeler,
        SolidOperation::Chamfer {
            brep: &unit,
            selector: SolidEdgeSelector::NearestToPoint(p(0.5, 0.5, 0.5)),
            distance: 0.1,
            second: None,
        },
    );
    assert_eq!(unsupported_code(&outcome), EDGE_NOT_FOUND);
    let reason = outcome.unsupported().expect("a structured refusal");
    assert_eq!(reason.operation.as_str(), "chamfer");
    assert!(reason.detail.contains("no edge lies within"), "{reason:?}");
    assert!(outcome.body().is_none(), "no body may be invented");
}

#[test]
fn chamfer_refuses_bad_sizes_and_curved_bodies_and_honours_cancellation() {
    let modeler = SolidCoreModeler::new();
    let unit = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));

    // A zero or negative set-back is invalid input, rejected before the kernel.
    for distance in [0.0, -0.1] {
        let req = request(SolidOperation::Chamfer {
            brep: &unit,
            selector: chamfer_selector(),
            distance,
            second: None,
        });
        match execute_solid(&modeler, &req, &never()).unwrap_err() {
            CadError::InvalidInput(message) => {
                assert!(message.contains("solid.invalid_dimension"), "{message}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    // A curved body cannot be lifted in this cut: the same refusal as a boolean,
    // never an approximated chamfer.
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
    let outcome = execute(
        &modeler,
        SolidOperation::Chamfer {
            brep: &cylinder,
            selector: SolidEdgeSelector::NearestToPoint(p(1.0, 0.0, 0.0)),
            distance: 0.1,
            second: None,
        },
    );
    assert_eq!(unsupported_code(&outcome), LIFT_REQUIRES_PLANAR);
    assert!(outcome.body().is_none());

    // Cancellation wins before any work and leaves no product.
    let req = request(SolidOperation::Chamfer {
        brep: &unit,
        selector: chamfer_selector(),
        distance: 0.1,
        second: None,
    });
    assert_eq!(
        execute_solid(&modeler, &req, &|| true).unwrap_err(),
        CadError::Cancelled
    );
}

#[test]
fn chamfer_refusals_name_the_chamfer_family() {
    // A size the kernel cannot fit (larger than the unit box) is a geometric
    // chamfer refusal: `Unsupported` with the chamfer-specific code, never a
    // silent success or a plausible smaller cut.
    let modeler = SolidCoreModeler::new();
    let unit = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    let outcome = execute(
        &modeler,
        SolidOperation::Chamfer {
            brep: &unit,
            selector: chamfer_selector(),
            distance: 2.0,
            second: None,
        },
    );
    assert!(outcome.body().is_none(), "{outcome:?}");
    let reason = outcome.unsupported().unwrap_or_else(|| {
        panic!("a size too large for the faces must be Unsupported, got {outcome:?}")
    });
    assert_eq!(reason.code, CHAMFER_FAILED);
    assert_eq!(reason.operation.as_str(), "chamfer");
}
