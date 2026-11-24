//! ADR-0036 C1 extension: the typed **option constructions** of the seven
//! primitive commands (`Cube`/`Center`/`Length`, `Diameter`, `2P`, `Top radius`)
//! and the `ELEV` base rule for plan inputs.
//!
//! Same **fake** modeler as the parent module: a hand-built planar unit box, so
//! the real `cad-kernel-adapter` tessellator produces a closed facet set and the
//! commit path is the production one. The point of these contracts is the
//! `PrimitiveSpec` the command sends and the state it leaves behind, not the
//! kernel's body (the real kernel is covered by
//! `cad-cli-tools/tests/solid_options.rs`).

use super::super::*;
use super::{app_with_modeler, entity_count, p, Answer};
use crate::command_payload::{PlanCircle, PlanLength, SolidPrimitiveCommand};
use crate::{CadError, CadResult, CommandOutcome};
use cad_kernel_adapter::PrimitiveSpec;
use std::sync::{Arc, Mutex};

/// Run one primitive command with a typed construction payload.
fn run(
    app: &mut Application,
    session: &mut SessionState,
    kind: ExtendedCommandId,
    input: SolidPrimitiveCommand,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Extended(kind),
        CommandPayload::Solid(input),
    )
}

/// The single spec the fake modeler recorded.
fn one_spec(specs: &Arc<Mutex<Vec<PrimitiveSpec>>>) -> PrimitiveSpec {
    let recorded = specs.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1, "exactly one modeler call");
    recorded[0]
}

/// Set the session `ELEV` token (a finite value, as the setter requires).
fn set_elev(session: &mut SessionState, elevation: f64) {
    session
        .drafting_extras
        .set_elev(elevation)
        .expect("a finite elevation is legal");
}

/// Build one primed app, run `input`, and return its single spec.
fn spec_of(kind: ExtendedCommandId, input: SolidPrimitiveCommand) -> PrimitiveSpec {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    run(&mut app, &mut session, kind, input).expect("the construction creates a body");
    one_spec(&specs)
}

/// Build one primed app at `elevation`, run `input`, and return its single spec.
fn spec_at_elev(
    kind: ExtendedCommandId,
    input: SolidPrimitiveCommand,
    elevation: f64,
) -> PrimitiveSpec {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    set_elev(&mut session, elevation);
    run(&mut app, &mut session, kind, input).expect("the construction creates a body");
    one_spec(&specs)
}

// ---- BOX ----------------------------------------------------------------

#[test]
fn box_cube_uses_one_edge_from_a_plan_corner_at_the_session_elevation() {
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Box,
            SolidPrimitiveCommand::Cube {
                corner: [2.0, 3.0],
                edge: 4.0,
            },
            10.0,
        ),
        PrimitiveSpec::Box {
            origin: p(2.0, 3.0, 10.0),
            length: 4.0,
            width: 4.0,
            height: 4.0,
        }
    );
    assert_eq!(
        spec_of(
            ExtendedCommandId::Box,
            SolidPrimitiveCommand::Cube {
                corner: [0.0, 0.0],
                edge: 1.5,
            },
        ),
        PrimitiveSpec::Box {
            origin: p(0.0, 0.0, 0.0),
            length: 1.5,
            width: 1.5,
            height: 1.5,
        },
        "the default ELEV 0 keeps the base on the work plane"
    );
}

#[test]
fn box_center_with_a_corner_is_centred_on_the_3d_midpoint() {
    // AutoCAD's `Center` is centred on the point **in height too**: `corner`'s
    // mirror image about the centre is the opposite corner, so the body spans
    // `center ± (corner - center)`. The points are explicit 3D coordinates, so
    // the session ELEV is irrelevant.
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Box,
            SolidPrimitiveCommand::CenterPoints {
                center: p(10.0, 10.0, 10.0),
                corner: p(12.0, 14.0, 13.0),
            },
            99.0,
        ),
        PrimitiveSpec::Box {
            origin: p(8.0, 6.0, 7.0),
            length: 4.0,
            width: 8.0,
            height: 6.0,
        },
        "the box is centred on `center` in all three axes; ELEV does not apply"
    );
}

#[test]
fn box_center_cube_and_center_lengths_center_on_all_three_axes() {
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Box,
            SolidPrimitiveCommand::CenterCube {
                center: [0.0, 0.0],
                edge: 4.0,
            },
            5.0,
        ),
        PrimitiveSpec::Box {
            origin: p(-2.0, -2.0, 3.0),
            length: 4.0,
            width: 4.0,
            height: 4.0,
        },
        "the cube is centred on z = ELEV, so it spans ELEV ± edge/2"
    );
    assert_eq!(
        spec_of(
            ExtendedCommandId::Box,
            SolidPrimitiveCommand::CenterLengths {
                center: [10.0, 10.0],
                length: 2.0,
                width: 4.0,
                height: 6.0,
            },
        ),
        PrimitiveSpec::Box {
            origin: p(9.0, 8.0, -3.0),
            length: 2.0,
            width: 4.0,
            height: 6.0,
        },
        "a centred box spans ELEV − height/2 .. ELEV + height/2"
    );
}

#[test]
fn box_length_takes_a_plan_corner_and_three_extents() {
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Box,
            SolidPrimitiveCommand::Length {
                corner: [1.0, 2.0],
                length: 3.0,
                width: 4.0,
                height: 5.0,
            },
            -2.0,
        ),
        PrimitiveSpec::Box {
            origin: p(1.0, 2.0, -2.0),
            length: 3.0,
            width: 4.0,
            height: 5.0,
        },
        "a negative ELEV is a legal base plane"
    );
}

// ---- WEDGE --------------------------------------------------------------

#[test]
fn wedge_takes_the_same_cube_center_and_length_constructions_as_box() {
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Wedge,
            SolidPrimitiveCommand::Cube {
                corner: [0.0, 0.0],
                edge: 2.0,
            },
            1.0,
        ),
        PrimitiveSpec::Wedge {
            origin: p(0.0, 0.0, 1.0),
            length: 2.0,
            width: 2.0,
            height: 2.0,
        }
    );
    assert_eq!(
        spec_of(
            ExtendedCommandId::Wedge,
            SolidPrimitiveCommand::CenterLengths {
                center: [5.0, 5.0],
                length: 2.0,
                width: 3.0,
                height: 4.0,
            },
        ),
        PrimitiveSpec::Wedge {
            origin: p(4.0, 3.5, -2.0),
            length: 2.0,
            width: 3.0,
            height: 4.0,
        }
    );
    assert_eq!(
        spec_of(
            ExtendedCommandId::Wedge,
            SolidPrimitiveCommand::Length {
                corner: [0.0, 0.0],
                length: 2.0,
                width: 3.0,
                height: 4.0,
            },
        ),
        PrimitiveSpec::Wedge {
            origin: p(0.0, 0.0, 0.0),
            length: 2.0,
            width: 3.0,
            height: 4.0,
        }
    );
}

// ---- CYLINDER -----------------------------------------------------------

#[test]
fn cylinder_takes_radius_diameter_and_two_point_bases() {
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Cylinder,
            SolidPrimitiveCommand::CylinderPlan {
                base: PlanCircle::Radius {
                    center: [1.0, 1.0],
                    radius: 2.0,
                },
                height: 5.0,
            },
            7.0,
        ),
        PrimitiveSpec::Cylinder {
            origin: p(1.0, 1.0, 7.0),
            radius: 2.0,
            height: 5.0,
        }
    );
    assert_eq!(
        spec_of(
            ExtendedCommandId::Cylinder,
            SolidPrimitiveCommand::CylinderPlan {
                base: PlanCircle::Diameter {
                    center: [0.0, 0.0],
                    diameter: 6.0,
                },
                height: 1.0,
            },
        ),
        PrimitiveSpec::Cylinder {
            origin: p(0.0, 0.0, 0.0),
            radius: 3.0,
            height: 1.0,
        }
    );
    assert_eq!(
        spec_of(
            ExtendedCommandId::Cylinder,
            SolidPrimitiveCommand::CylinderPlan {
                base: PlanCircle::TwoPoint {
                    first: [0.0, 0.0],
                    second: [0.0, 8.0],
                },
                height: 2.0,
            },
        ),
        PrimitiveSpec::Cylinder {
            origin: p(0.0, 4.0, 0.0),
            radius: 4.0,
            height: 2.0,
        },
        "2P supplies the two ends of a base diameter"
    );
}

// ---- CONE ---------------------------------------------------------------

#[test]
fn cone_takes_diameter_and_two_point_bases() {
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Cone,
            SolidPrimitiveCommand::ConePlan {
                base: PlanCircle::Diameter {
                    center: [0.0, 0.0],
                    diameter: 4.0,
                },
                top_radius: 0.0,
                height: 5.0,
            },
            3.0,
        ),
        PrimitiveSpec::Cone {
            origin: p(0.0, 0.0, 3.0),
            radius_bottom: 2.0,
            radius_top: 0.0,
            height: 5.0,
        },
        "top radius 0 is AutoCAD's pointed cone"
    );
    assert_eq!(
        spec_of(
            ExtendedCommandId::Cone,
            SolidPrimitiveCommand::ConePlan {
                base: PlanCircle::TwoPoint {
                    first: [-2.0, 0.0],
                    second: [2.0, 0.0],
                },
                top_radius: 0.5,
                height: 3.0,
            },
        ),
        PrimitiveSpec::Cone {
            origin: p(0.0, 0.0, 0.0),
            radius_bottom: 2.0,
            radius_top: 0.5,
            height: 3.0,
        }
    );
}

#[test]
fn cone_top_radius_is_a_frustum_and_may_widen_upwards() {
    assert_eq!(
        spec_of(
            ExtendedCommandId::Cone,
            SolidPrimitiveCommand::ConePlan {
                base: PlanCircle::Radius {
                    center: [0.0, 0.0],
                    radius: 2.0,
                },
                top_radius: 1.0,
                height: 5.0,
            },
        ),
        PrimitiveSpec::Cone {
            origin: p(0.0, 0.0, 0.0),
            radius_bottom: 2.0,
            radius_top: 1.0,
            height: 5.0,
        },
        "Top radius creates a frustum"
    );
    assert_eq!(
        spec_of(
            ExtendedCommandId::Cone,
            SolidPrimitiveCommand::ConePlan {
                base: PlanCircle::Radius {
                    center: [0.0, 0.0],
                    radius: 2.0,
                },
                top_radius: 3.0,
                height: 5.0,
            },
        ),
        PrimitiveSpec::Cone {
            origin: p(0.0, 0.0, 0.0),
            radius_bottom: 2.0,
            radius_top: 3.0,
            height: 5.0,
        },
        "AutoCAD's Top radius may be wider than the base"
    );
}

#[test]
fn cone_top_radius_equal_to_the_base_radius_is_refused_as_a_cylinder() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);
    let error = run(
        &mut app,
        &mut session,
        ExtendedCommandId::Cone,
        SolidPrimitiveCommand::ConePlan {
            base: PlanCircle::Radius {
                center: [0.0, 0.0],
                radius: 2.0,
            },
            top_radius: 2.0,
            height: 5.0,
        },
    )
    .expect_err("equal radii are a straight tube");
    assert!(
        matches!(&error, CadError::InvalidInput(text) if text.starts_with("cone.equal_radii:")),
        "{error:?}"
    );
    assert!(specs.lock().unwrap().is_empty(), "no modeler call happened");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert_eq!(entity_count(&app), 0);
}

// ---- SPHERE -------------------------------------------------------------

#[test]
fn sphere_takes_a_diameter_and_the_3d_two_point_form() {
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Sphere,
            SolidPrimitiveCommand::SphereDiameter {
                center: [1.0, 2.0],
                diameter: 6.0,
            },
            3.0,
        ),
        PrimitiveSpec::Sphere {
            origin: p(1.0, 2.0, 3.0),
            radius: 3.0,
        }
    );
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Sphere,
            SolidPrimitiveCommand::SphereTwoPoint {
                first: p(0.0, 0.0, 0.0),
                second: p(0.0, 0.0, 10.0),
            },
            42.0,
        ),
        PrimitiveSpec::Sphere {
            origin: p(0.0, 0.0, 5.0),
            radius: 5.0,
        },
        "2P's points are explicitly 3D, so ELEV never shifts the sphere"
    );
}

// ---- TORUS --------------------------------------------------------------

#[test]
fn torus_takes_a_diameter_or_two_point_ring_and_a_radius_or_diameter_tube() {
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Torus,
            SolidPrimitiveCommand::TorusPlan {
                ring: PlanCircle::Diameter {
                    center: [0.0, 0.0],
                    diameter: 20.0,
                },
                tube: PlanLength::Radius(2.0),
            },
            5.0,
        ),
        PrimitiveSpec::Torus {
            origin: p(0.0, 0.0, 5.0),
            major_radius: 10.0,
            minor_radius: 2.0,
        }
    );
    assert_eq!(
        spec_of(
            ExtendedCommandId::Torus,
            SolidPrimitiveCommand::TorusPlan {
                ring: PlanCircle::TwoPoint {
                    first: [-10.0, 0.0],
                    second: [10.0, 0.0],
                },
                tube: PlanLength::Diameter(4.0),
            },
        ),
        PrimitiveSpec::Torus {
            origin: p(0.0, 0.0, 0.0),
            major_radius: 10.0,
            minor_radius: 2.0,
        }
    );
    assert_eq!(
        spec_of(
            ExtendedCommandId::Torus,
            SolidPrimitiveCommand::TorusPlan {
                ring: PlanCircle::Radius {
                    center: [0.0, 0.0],
                    radius: 9.0,
                },
                tube: PlanLength::TwoPoint {
                    first: [0.0, 0.0],
                    second: [3.0, 4.0],
                },
            },
        ),
        PrimitiveSpec::Torus {
            origin: p(0.0, 0.0, 0.0),
            major_radius: 9.0,
            minor_radius: 5.0,
        },
        "tube 2Point takes the plan distance between the two points as the tube radius"
    );
}

#[test]
fn torus_refuses_a_tube_that_does_not_fit_inside_the_ring() {
    // The modeler seam's `Torus` requires `minor < major`; AutoCAD's
    // negative-radius "apple"/"rugby ball" forms are outside it and are refused
    // by name rather than silently clamped.
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);
    let error = run(
        &mut app,
        &mut session,
        ExtendedCommandId::Torus,
        SolidPrimitiveCommand::TorusPlan {
            ring: PlanCircle::Radius {
                center: [0.0, 0.0],
                radius: 9.0,
            },
            tube: PlanLength::Radius(11.0),
        },
    )
    .expect_err("a tube wider than the ring is outside the seam");
    assert!(
        matches!(&error, CadError::InvalidInput(text) if text.starts_with("torus.degenerate_extent:")),
        "{error:?}"
    );
    assert!(specs.lock().unwrap().is_empty(), "no modeler call happened");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert_eq!(entity_count(&app), 0);
}

#[test]
fn torus_refuses_a_negative_ring_or_a_degenerate_tube() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);
    let error = run(
        &mut app,
        &mut session,
        ExtendedCommandId::Torus,
        SolidPrimitiveCommand::TorusPlan {
            ring: PlanCircle::Radius {
                center: [0.0, 0.0],
                radius: -4.0,
            },
            tube: PlanLength::Radius(1.0),
        },
    )
    .expect_err("a negative ring radius is not a body");
    assert!(
        matches!(&error, CadError::InvalidInput(text) if text.starts_with("torus.degenerate_extent:")),
        "{error:?}"
    );
    assert!(specs.lock().unwrap().is_empty(), "no modeler call happened");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert_eq!(entity_count(&app), 0);
}

// ---- ELEV ---------------------------------------------------------------

#[test]
fn elev_lifts_the_plan_forms_and_never_the_explicit_points_form() {
    // One session token, two payload shapes: the plan form's base plane is the
    // token; the explicit 3D form keeps the caller's z.
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    set_elev(&mut session, 7.0);
    run(
        &mut app,
        &mut session,
        ExtendedCommandId::Box,
        SolidPrimitiveCommand::Cube {
            corner: [0.0, 0.0],
            edge: 2.0,
        },
    )
    .expect("plan BOX");
    run(
        &mut app,
        &mut session,
        ExtendedCommandId::Box,
        SolidPrimitiveCommand::Points(vec![p(0.0, 0.0, 0.0), p(2.0, 2.0, 2.0)]),
    )
    .expect("explicit BOX");
    assert_eq!(
        *specs.lock().unwrap(),
        vec![
            PrimitiveSpec::Box {
                origin: p(0.0, 0.0, 7.0),
                length: 2.0,
                width: 2.0,
                height: 2.0,
            },
            PrimitiveSpec::Box {
                origin: p(0.0, 0.0, 0.0),
                length: 2.0,
                width: 2.0,
                height: 2.0,
            },
        ],
        "ELEV lifts the plan construction and leaves the explicit corners alone"
    );
}

// ---- refusals -----------------------------------------------------------

#[test]
fn a_construction_the_kind_does_not_take_is_refused_before_any_write() {
    let cases: [(ExtendedCommandId, SolidPrimitiveCommand, &str); 3] = [
        (
            ExtendedCommandId::Sphere,
            SolidPrimitiveCommand::Cube {
                corner: [0.0, 0.0],
                edge: 1.0,
            },
            "solid.option_not_supported: SPHERE does not take the Cube construction",
        ),
        (
            ExtendedCommandId::Pyramid,
            SolidPrimitiveCommand::Length {
                corner: [0.0, 0.0],
                length: 1.0,
                width: 1.0,
                height: 1.0,
            },
            "solid.option_not_supported: PYRAMID does not take the Length construction",
        ),
        (
            ExtendedCommandId::Cone,
            SolidPrimitiveCommand::TorusPlan {
                ring: PlanCircle::Radius {
                    center: [0.0, 0.0],
                    radius: 5.0,
                },
                tube: PlanLength::Radius(1.0),
            },
            "solid.option_not_supported: CONE does not take the plan ring and tube construction",
        ),
    ];
    for (kind, input, expected) in cases {
        let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
        let before = revision(&app);
        let error = run(&mut app, &mut session, kind, input)
            .expect_err("a foreign construction is refused");
        let CadError::InvalidInput(text) = error else {
            panic!("{kind:?} refuses with InvalidInput");
        };
        assert!(text.starts_with(expected), "{kind:?}: {text}");
        assert!(
            specs.lock().unwrap().is_empty(),
            "{kind:?}: no modeler call"
        );
        assert_eq!(revision(&app), before, "{kind:?}");
        assert!(!app.can_undo(&DocumentId(1)), "{kind:?}");
        assert_eq!(entity_count(&app), 0, "{kind:?}");
    }
}

#[test]
fn degenerate_or_non_finite_plan_input_is_refused_before_the_modeler() {
    let cases: [(ExtendedCommandId, SolidPrimitiveCommand, &str); 8] = [
        (
            ExtendedCommandId::Box,
            SolidPrimitiveCommand::Length {
                corner: [0.0, 0.0],
                length: 1.0,
                width: 1.0,
                height: 0.0,
            },
            "box.degenerate_extent:",
        ),
        (
            ExtendedCommandId::Box,
            SolidPrimitiveCommand::Cube {
                corner: [0.0, 0.0],
                edge: -1.0,
            },
            "box.degenerate_extent:",
        ),
        (
            ExtendedCommandId::Box,
            SolidPrimitiveCommand::Cube {
                corner: [f64::NAN, 0.0],
                edge: 1.0,
            },
            "box.non_finite_point:",
        ),
        (
            ExtendedCommandId::Cylinder,
            SolidPrimitiveCommand::CylinderPlan {
                base: PlanCircle::Diameter {
                    center: [0.0, 0.0],
                    diameter: 0.0,
                },
                height: 1.0,
            },
            "cylinder.degenerate_extent:",
        ),
        (
            ExtendedCommandId::Cylinder,
            SolidPrimitiveCommand::CylinderPlan {
                base: PlanCircle::TwoPoint {
                    first: [1.0, 1.0],
                    second: [1.0, 1.0],
                },
                height: 1.0,
            },
            "cylinder.degenerate_extent:",
        ),
        (
            ExtendedCommandId::Cone,
            SolidPrimitiveCommand::ConePlan {
                base: PlanCircle::Radius {
                    center: [0.0, 0.0],
                    radius: 2.0,
                },
                top_radius: -1.0,
                height: 1.0,
            },
            "cone.degenerate_extent:",
        ),
        (
            ExtendedCommandId::Sphere,
            SolidPrimitiveCommand::SphereDiameter {
                center: [0.0, 0.0],
                diameter: f64::INFINITY,
            },
            "sphere.degenerate_extent:",
        ),
        (
            ExtendedCommandId::Torus,
            SolidPrimitiveCommand::TorusPlan {
                ring: PlanCircle::Radius {
                    center: [0.0, 0.0],
                    radius: 5.0,
                },
                tube: PlanLength::Radius(0.0),
            },
            "torus.degenerate_extent:",
        ),
    ];
    for (kind, input, expected) in cases {
        let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
        let before = revision(&app);
        let error = run(&mut app, &mut session, kind, input)
            .expect_err("a degenerate construction is refused");
        let CadError::InvalidInput(text) = error else {
            panic!("{kind:?} refuses with InvalidInput");
        };
        assert!(text.starts_with(expected), "{kind:?}: {text}");
        assert!(
            specs.lock().unwrap().is_empty(),
            "{kind:?}: no modeler call"
        );
        assert_eq!(revision(&app), before, "{kind:?}");
        assert!(!app.can_undo(&DocumentId(1)), "{kind:?}");
        assert_eq!(entity_count(&app), 0, "{kind:?}");
    }
}

// ---- one transaction ----------------------------------------------------

#[test]
fn a_plan_construction_commits_one_body_in_one_undo_step() {
    let (mut app, mut session, _) = app_with_modeler(Answer::Body);
    let before = revision(&app);
    let outcome = run(
        &mut app,
        &mut session,
        ExtendedCommandId::Cylinder,
        SolidPrimitiveCommand::CylinderPlan {
            base: PlanCircle::Diameter {
                center: [5.0, 5.0],
                diameter: 4.0,
            },
            height: 10.0,
        },
    )
    .expect("CYLINDER DIAMETER creates a body");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .next()
        .expect("the committed row exists");
    assert_eq!(entity.object.type_key, "AcDb3dSolid");
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("CYLINDER must commit a Body3d");
    };
    assert_eq!(body.kind, BodyKind::Solid);
    assert!(matches!(body.exact, BodyExact::Exact(_)));

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert_eq!(entity_count(&app), 0);
    assert!(!app.can_undo(&DocumentId(1)));
}

// ---- PYRAMID: the typed regular n-gon base ------------------------------

#[test]
fn pyramid_polygon_base_is_the_polygon_ring_at_the_session_elevation() {
    // Inscribed: the first vertex lies on +X at exactly the typed radius, so the
    // seam receives that radius and rotation 0.
    assert_eq!(
        spec_at_elev(
            ExtendedCommandId::Pyramid,
            SolidPrimitiveCommand::PyramidPolygon {
                base: [2.0, 3.0],
                radius: 5.0,
                sides: 6,
                circumscribed: false,
                height: 8.0,
            },
            7.0,
        ),
        PrimitiveSpec::PyramidNgon {
            base_center: p(2.0, 3.0, 7.0),
            radius: 5.0,
            sides: 6,
            rotation: 0.0,
            height: 8.0,
        },
        "ELEV sets the base plane; the inscribed ring's vertex radius is the typed one"
    );

    // A triangle is the smallest base: the ring is still the POLYGON one.
    let spec = spec_of(
        ExtendedCommandId::Pyramid,
        SolidPrimitiveCommand::PyramidPolygon {
            base: [0.0, 0.0],
            radius: 2.0,
            sides: 3,
            circumscribed: false,
            height: 1.0,
        },
    );
    assert!(
        matches!(
            spec,
            PrimitiveSpec::PyramidNgon {
                radius,
                sides: 3,
                rotation,
                ..
            } if (radius - 2.0).abs() < 1e-12 && rotation.abs() < 1e-12
        ),
        "{spec:?}"
    );
}

#[test]
fn pyramid_polygon_circumscribed_uses_the_polygon_tangent_ring() {
    // The typed radius is the tangent circle; the ring's vertex radius is
    // `r / cos(π/n)` and the first vertex is half a step clockwise of +X —
    // exactly `POLYGON CIRCUMSCRIBED`'s construction.
    let spec = spec_of(
        ExtendedCommandId::Pyramid,
        SolidPrimitiveCommand::PyramidPolygon {
            base: [1.0, -1.0],
            radius: 5.0,
            sides: 6,
            circumscribed: true,
            height: 4.0,
        },
    );
    let PrimitiveSpec::PyramidNgon {
        base_center,
        radius,
        sides,
        rotation,
        height,
    } = spec
    else {
        panic!("an n-gon pyramid spec, got {spec:?}");
    };
    assert_eq!(base_center, p(1.0, -1.0, 0.0));
    assert_eq!(sides, 6);
    assert_eq!(height, 4.0);
    let expected_radius = 5.0 / (std::f64::consts::PI / 6.0).cos();
    assert!(
        (radius - expected_radius).abs() < 1e-12,
        "vertex radius {radius} vs {expected_radius}"
    );
    assert!(
        (rotation + std::f64::consts::PI / 6.0).abs() < 1e-12,
        "the first vertex is half a step clockwise of +X, got {rotation}"
    );
}

#[test]
fn pyramid_polygon_refuses_out_of_range_input_before_the_modeler() {
    let cases = [
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 1.0,
                sides: 2,
                circumscribed: false,
                height: 1.0,
            },
            "pyramid.sides:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 1.0,
                sides: 1025,
                circumscribed: false,
                height: 1.0,
            },
            "pyramid.sides:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 0.0,
                sides: 4,
                circumscribed: false,
                height: 1.0,
            },
            "pyramid.radius:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [f64::NAN, 0.0],
                radius: 1.0,
                sides: 4,
                circumscribed: false,
                height: 1.0,
            },
            "pyramid.invalid_point:",
        ),
        (
            SolidPrimitiveCommand::PyramidPolygon {
                base: [0.0, 0.0],
                radius: 1.0,
                sides: 4,
                circumscribed: false,
                height: 0.0,
            },
            "pyramid.degenerate_extent:",
        ),
    ];
    for (input, expected) in cases {
        let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
        let before = revision(&app);
        let error = run(&mut app, &mut session, ExtendedCommandId::Pyramid, input)
            .expect_err("an out-of-range polygon base is refused");
        let CadError::InvalidInput(text) = error else {
            panic!("a degenerate construction is InvalidInput");
        };
        assert!(text.starts_with(expected), "{text}");
        assert!(specs.lock().unwrap().is_empty(), "no modeler call");
        assert_eq!(revision(&app), before);
        assert!(!app.can_undo(&DocumentId(1)));
        assert_eq!(entity_count(&app), 0);
    }
}
