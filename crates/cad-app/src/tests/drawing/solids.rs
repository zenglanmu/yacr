//! ADR-0036 phase C1: the seven primitive commands create exact solid bodies.
//!
//! The contracts here use a **fake** [`SolidModeler`]: a hand-built planar
//! unit-box `BrepData` (six plane faces with `Line` loops, outward wound) so the
//! real `cad-kernel-adapter` tessellator produces a closed facet set. The kernel
//! itself is not linked into `cad-app`; the real modeler is exercised by
//! `cad-kernel-solidcore`'s own tests and by the `cad-cli-tools` integration
//! contract (`tests/solid_command.rs`).

use super::*;
use cad_kernel_adapter::{
    ModelerCapabilities, ModelerPrimitives, PrimitiveKind, PrimitiveSpec, SolidModeler,
    SolidOperation, SolidOutcome, SolidProduct, SolidRequest, SolidResult, SolidUnsupportedReason,
};
use std::sync::{Arc, Mutex};

// ---- the fake modeler -----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Body,
    Unsupported,
    Failed,
    /// A body whose exact B-rep the tessellator cannot evaluate: it answers
    /// `TessellationOutcome::Unsupported` (a structured reason, **no**
    /// diagnostics), which the handler must report as `code: detail`.
    Untessellatable,
}

struct FakeModeler {
    answer: Answer,
    specs: Arc<Mutex<Vec<PrimitiveSpec>>>,
}

impl SolidModeler for FakeModeler {
    fn registration(&self) -> cad_domain::Registration {
        cad_domain::Registration {
            type_key: "test.fake-solid".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["3DSOLID".into()],
            capabilities: vec!["primitives.box".into()],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        ModelerCapabilities {
            primitives: ModelerPrimitives::all(),
            booleans: false,
            mass_properties: false,
            fillet: false,
            shell: false,
            nurbs: false,
        }
    }

    fn execute(
        &self,
        request: &SolidRequest<'_>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<SolidResult> {
        let SolidOperation::Primitive { spec } = request.operation else {
            panic!("a primitive command must send a Primitive operation");
        };
        self.specs.lock().expect("spec log").push(spec);
        let outcome = match self.answer {
            Answer::Body => SolidOutcome::Success(SolidProduct::Body {
                brep: unit_box(),
                diagnostics: Vec::new(),
            }),
            Answer::Unsupported => SolidOutcome::Unsupported {
                reason: SolidUnsupportedReason {
                    code: "solid.unsupported",
                    operation: request.operation.family(),
                    detail: "this fake modeler declines".into(),
                },
            },
            Answer::Untessellatable => SolidOutcome::Success(SolidProduct::Body {
                brep: unsupported_surface_body(),
                diagnostics: Vec::new(),
            }),
            Answer::Failed => SolidOutcome::Failed {
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "solid.empty_result".into(),
                    message: "the fake kernel produced nothing".into(),
                }],
            },
        };
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}

fn app_with_modeler(answer: Answer) -> (Application, SessionState, Arc<Mutex<Vec<PrimitiveSpec>>>) {
    let (mut app, session) = application_with_document();
    let specs = Arc::new(Mutex::new(Vec::new()));
    app.set_solid_modeler(Some(Arc::new(FakeModeler {
        answer,
        specs: specs.clone(),
    })));
    (app, session, specs)
}

fn solid_command(ext: ExtendedCommandId, points: Vec<Point3>) -> Command {
    command(CommandId::Extended(ext), CommandPayload::Points(points))
}

pub(super) fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// The stable `InvalidInput` text, or a panic naming the wrong variant.
fn invalid_input(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) => text,
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .count()
}

// ---- the fixture B-rep ----------------------------------------------------

fn vsub(a: Point3, b: Point3) -> Point3 {
    p(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn vcross(a: Point3, b: Point3) -> Point3 {
    p(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

fn vlen(v: Point3) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

fn unit(v: Point3) -> Point3 {
    let length = vlen(v);
    if length > 1e-12 {
        p(v.x / length, v.y / length, v.z / length)
    } else {
        p(0.0, 0.0, 0.0)
    }
}

/// One planar face from an outward-wound vertex cycle.
fn plane_face(id: u32, cycle: &[Point3]) -> BrepFace {
    let origin = cycle[0];
    let u_dir = cycle
        .iter()
        .map(|corner| unit(vsub(*corner, origin)))
        .find(|direction| vlen(*direction) > 0.0)
        .unwrap_or(p(1.0, 0.0, 0.0));
    let normal = unit(vcross(vsub(cycle[1], origin), vsub(cycle[2], origin)));
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
    }
}

/// A closed planar unit box `[0,1]³`: the minimal body the real tessellator
/// turns into a complete facet set (no missing faces, no open edges).
pub(super) fn unit_box() -> BrepData {
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
    }
}

/// A one-face body whose supporting surface this build cannot evaluate.
fn unsupported_surface_body() -> BrepData {
    BrepData {
        shells: vec![BrepShell {
            id: 0,
            faces: vec![BrepFace {
                id: 0,
                surface: BrepSurface::Unsupported {
                    type_key: "test.unsupported-surface".into(),
                },
                reversed: false,
                loops: vec![BrepLoop {
                    edges: vec![BrepCurve::Line {
                        start: p(0.0, 0.0, 0.0),
                        end: p(1.0, 0.0, 0.0),
                    }],
                }],
            }],
        }],
        placement: None,
    }
}

#[test]
fn an_untessellatable_body_reports_the_structured_tessellation_reason() {
    // `TessellationOutcome::Unsupported` carries a `reason`, not diagnostics;
    // the refusal must name `code: detail` rather than "kernel failed".
    let (mut app, mut session, _) = app_with_modeler(Answer::Untessellatable);
    let before = revision(&app);
    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Box,
                vec![p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0)],
            ),
        )
        .expect_err("the tessellator cannot evaluate that face");
    let CadError::Unsupported(reason) = error else {
        panic!("a failed tessellation is Unsupported, got {error:?}");
    };
    assert!(
        reason.starts_with("solid.tessellation_failed: kernel.unsupported_surface:"),
        "{reason}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert_eq!(entity_count(&app), 0);
}

// ---- contracts ------------------------------------------------------------

#[test]
fn box_creates_one_exact_solid_body_with_display_and_one_undo_step() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Box,
                vec![p(0.0, 0.0, 0.0), p(1.0, 2.0, 3.0)],
            ),
        )
        .expect("BOX creates a body");
    assert_eq!(specs.lock().unwrap().len(), 1);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(outcome.objects.len(), 1);

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let entity = drawing
        .entities()
        .find(|entity| entity.object.id == outcome.objects[0])
        .expect("the committed row exists");
    assert_eq!(entity.object.type_key, "AcDb3dSolid");
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("BOX must commit a Body3d");
    };
    assert_eq!(body.kind, BodyKind::Solid);
    assert!(matches!(body.exact, BodyExact::Exact(_)));
    assert!(
        !body.display.mesh.triangles.is_empty(),
        "display mesh is built"
    );
    assert!(!body.display.edges.is_empty(), "display edges are built");
    assert_eq!(body.display.completeness, Completeness::Complete);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert_eq!(entity_count(&app), 0, "one undo removes the created body");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_carrier_without_a_modeler_refuses_by_name_and_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Box,
                vec![p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0)],
            ),
        )
        .expect_err("no modeler, no body");
    assert_eq!(
        error,
        CadError::Unsupported("unsupported-runtime: solid-modeler".into())
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert_eq!(entity_count(&app), 0);
}

#[test]
fn a_wrong_point_count_or_zero_extent_is_invalid_input_and_writes_nothing() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);

    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Box,
                vec![p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0), p(2.0, 2.0, 2.0)],
            ),
        )
        .expect_err("three corners are not a box");
    let text = invalid_input(error);
    assert!(text.starts_with("box.requires_two_points:"), "{text}");

    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Box,
                vec![p(1.0, 1.0, 1.0), p(1.0, 1.0, 1.0)],
            ),
        )
        .expect_err("a zero extent is not a body");
    let text = invalid_input(error);
    assert!(text.starts_with("box.degenerate_extent:"), "{text}");

    assert!(specs.lock().unwrap().is_empty(), "no modeler call happened");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert_eq!(entity_count(&app), 0);
}

#[test]
fn an_unsupported_modeler_answer_is_a_stable_refusal_with_no_commit() {
    let (mut app, mut session, _) = app_with_modeler(Answer::Unsupported);
    let before = revision(&app);
    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Cylinder,
                vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 0.0, 2.0)],
            ),
        )
        .expect_err("the fake modeler declines");
    assert_eq!(
        error,
        CadError::Unsupported("solid.unsupported: this fake modeler declines".into())
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert_eq!(entity_count(&app), 0);
}

#[test]
fn a_failed_kernel_answer_is_a_stable_refusal_with_no_commit() {
    let (mut app, mut session, _) = app_with_modeler(Answer::Failed);
    let before = revision(&app);
    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Sphere,
                vec![p(0.0, 0.0, 0.0), p(0.0, 0.0, 2.0)],
            ),
        )
        .expect_err("the fake kernel failed");
    assert_eq!(
        error,
        // The outer prefix names the situation; the diagnostic's own code is
        // deliberately not repeated (`solid.kernel_failed: solid.kernel_failed:`).
        CadError::Unsupported("solid.kernel_failed: the fake kernel produced nothing".into())
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert_eq!(entity_count(&app), 0);
}

#[test]
fn box_takes_opposite_corners_and_normalises_to_the_min_corner() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Box),
        CommandPayload::Points(vec![p(5.0, 5.0, 5.0), p(2.0, 1.0, 0.0)]),
    )
    .expect("BOX");
    assert_eq!(
        *specs.lock().unwrap(),
        vec![PrimitiveSpec::Box {
            origin: p(2.0, 1.0, 0.0),
            length: 3.0,
            width: 4.0,
            height: 5.0,
        }]
    );
}

#[test]
fn the_seven_primitives_map_their_points_onto_the_seam_specs() {
    let cases: [(ExtendedCommandId, Vec<Point3>, PrimitiveSpec); 6] = [
        (
            ExtendedCommandId::Wedge,
            vec![p(0.0, 0.0, 0.0), p(4.0, 2.0, 0.0), p(0.0, 0.0, 3.0)],
            PrimitiveSpec::Wedge {
                origin: p(0.0, 0.0, 0.0),
                length: 4.0,
                width: 2.0,
                height: 3.0,
            },
        ),
        (
            ExtendedCommandId::Pyramid,
            vec![p(0.0, 0.0, 0.0), p(2.0, 2.0, 0.0), p(1.0, 1.0, 3.0)],
            PrimitiveSpec::Pyramid {
                base_center: p(1.0, 1.0, 0.0),
                length: 2.0,
                height: 3.0,
            },
        ),
        (
            ExtendedCommandId::Cylinder,
            vec![p(1.0, 1.0, 0.0), p(4.0, 1.0, 0.0), p(1.0, 1.0, 7.0)],
            PrimitiveSpec::Cylinder {
                origin: p(1.0, 1.0, 0.0),
                radius: 3.0,
                height: 7.0,
            },
        ),
        (
            ExtendedCommandId::Sphere,
            vec![p(0.0, 0.0, 0.0), p(0.0, 3.0, 4.0)],
            PrimitiveSpec::Sphere {
                origin: p(0.0, 0.0, 0.0),
                radius: 5.0,
            },
        ),
        (
            ExtendedCommandId::Cone,
            vec![p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(0.0, 0.0, 5.0)],
            // A top point directly over the base centre is a full cone: r_top = 0.
            PrimitiveSpec::Cone {
                origin: p(0.0, 0.0, 0.0),
                radius_bottom: 2.0,
                radius_top: 0.0,
                height: 5.0,
            },
        ),
        (
            ExtendedCommandId::Torus,
            vec![p(0.0, 0.0, 0.0), p(5.0, 0.0, 0.0), p(5.0, 0.0, 1.0)],
            PrimitiveSpec::Torus {
                origin: p(0.0, 0.0, 0.0),
                major_radius: 5.0,
                minor_radius: 1.0,
            },
        ),
    ];
    for (ext, points, expected) in cases {
        let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
        let before = revision(&app);
        execute(
            &mut app,
            &mut session,
            CommandId::Extended(ext),
            CommandPayload::Points(points),
        )
        .unwrap_or_else(|error| panic!("{ext:?} creates a body: {error:?}"));
        assert_eq!(*specs.lock().unwrap(), vec![expected], "{ext:?}");
        assert_eq!(revision(&app), Revision(before.0 + 1), "{ext:?}");
    }
}

#[test]
fn cone_takes_a_frustum_top_radius_from_the_top_point() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Cone),
        CommandPayload::Points(vec![p(0.0, 0.0, 0.0), p(3.0, 0.0, 0.0), p(1.0, 0.0, 4.0)]),
    )
    .expect("frustum");
    assert_eq!(
        *specs.lock().unwrap(),
        vec![PrimitiveSpec::Cone {
            origin: p(0.0, 0.0, 0.0),
            radius_bottom: 3.0,
            radius_top: 1.0,
            height: 4.0,
        }]
    );
}

#[test]
fn pyramid_rejects_a_rectangular_base_and_an_off_centre_apex() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);

    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Pyramid,
                vec![p(0.0, 0.0, 0.0), p(3.0, 2.0, 0.0), p(1.5, 1.0, 4.0)],
            ),
        )
        .expect_err("a rectangular base is not the seam's square pyramid");
    assert!(
        matches!(&error, CadError::InvalidInput(text) if text.starts_with("pyramid.invalid_base:")),
        "{error:?}"
    );

    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Pyramid,
                vec![p(0.0, 0.0, 0.0), p(2.0, 2.0, 0.0), p(1.0, 1.5, 4.0)],
            ),
        )
        .expect_err("the apex must be over the base centre");
    assert!(
        matches!(&error, CadError::InvalidInput(text) if text.starts_with("pyramid.invalid_base:")),
        "{error:?}"
    );
    assert!(specs.lock().unwrap().is_empty(), "no modeler call happened");
    assert_eq!(revision(&app), before);
}

#[test]
fn wedge_normalises_reversed_base_corners_to_the_same_body() {
    // The documented convention is "tall face at the minimum-x corner", so the
    // point order of the base must not change the resulting spec.
    let forward = wedge_spec(p(0.0, 0.0, 0.0), p(4.0, 2.0, 0.0));
    let reversed = wedge_spec(p(4.0, 2.0, 0.0), p(0.0, 0.0, 0.0));
    assert_eq!(forward, reversed);
    assert_eq!(
        forward,
        PrimitiveSpec::Wedge {
            origin: p(0.0, 0.0, 0.0),
            length: 4.0,
            width: 2.0,
            height: 3.0,
        }
    );
}

fn wedge_spec(p0: Point3, p1: Point3) -> PrimitiveSpec {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Wedge),
        CommandPayload::Points(vec![p0, p1, p(0.0, 0.0, 3.0)]),
    )
    .expect("WEDGE");
    let recorded = specs.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1);
    recorded[0]
}

#[test]
fn wedge_refuses_a_non_coplanar_base() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);
    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Wedge,
                vec![p(0.0, 0.0, 0.0), p(4.0, 2.0, 1.0), p(0.0, 0.0, 3.0)],
            ),
        )
        .expect_err("a tilted base has no single height");
    let text = invalid_input(error);
    assert!(text.starts_with("wedge.base_not_coplanar:"), "{text}");
    assert!(specs.lock().unwrap().is_empty());
    assert_eq!(revision(&app), before);
}

#[test]
fn pyramid_refuses_a_non_coplanar_base() {
    // The base plane is `z = p0.z = p1.z`; a tilted base must not silently take
    // the lower z (p1.z would be dropped).
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);
    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Pyramid,
                vec![p(0.0, 0.0, 0.0), p(2.0, 2.0, 1.0), p(1.0, 1.0, 4.0)],
            ),
        )
        .expect_err("a tilted base has no single plane");
    let text = invalid_input(error);
    assert!(text.starts_with("pyramid.base_not_coplanar:"), "{text}");
    assert!(specs.lock().unwrap().is_empty(), "no modeler call happened");
    assert_eq!(revision(&app), before);
}

#[test]
fn cone_refuses_equal_radii_and_points_at_cylinder() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);
    // Base radius 2 and a top point 2 away from the axis: a straight tube.
    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Cone,
                vec![p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(2.0, 0.0, 5.0)],
            ),
        )
        .expect_err("equal radii are a cylinder");
    let text = invalid_input(error);
    assert!(text.starts_with("cone.equal_radii:"), "{text}");
    assert!(text.contains("CYLINDER"), "{text}");
    assert!(specs.lock().unwrap().is_empty());
    assert_eq!(revision(&app), before);
}

#[test]
fn elev_never_translates_an_explicitly_placed_solid() {
    // `ELEV` is the session elevation for *new* 2D objects; an explicit 3D body
    // carries its own Z and must land exactly where its points say.
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    session
        .drafting_extras
        .set_elev(10.0)
        .expect("a finite elevation is legal");
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Box),
        CommandPayload::Points(vec![p(0.0, 0.0, 0.0), p(1.0, 1.0, 2.0)]),
    )
    .expect("BOX");
    assert_eq!(
        *specs.lock().unwrap(),
        vec![PrimitiveSpec::Box {
            origin: p(0.0, 0.0, 0.0),
            length: 1.0,
            width: 1.0,
            height: 2.0,
        }],
        "the modeler sees the explicit points, not the ELEV offset"
    );
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let entity = drawing.entities().next().expect("the box row");
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("BOX must commit a Body3d");
    };
    let brep = body.exact_brep().expect("exact");
    let (min, max) = cad_kernel_adapter::brep_bounds(brep).expect("bounds");
    // The fake modeler returns the unit-box fixture, so z runs 0..1; the point
    // of the assertion is that it starts at the explicit 0, not at ELEV's 10.
    assert_eq!(min.z, 0.0, "ELEV must not lift the body off z=0");
    assert_eq!(max.z, 1.0);
}

#[test]
fn torus_rejects_a_minor_radius_that_is_not_smaller_than_the_major() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);
    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Torus,
                vec![p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(2.0, 0.0, 2.0)],
            ),
        )
        .expect_err("a tube as thick as the ring is not a torus");
    assert!(
        matches!(&error, CadError::InvalidInput(text) if text.starts_with("torus.degenerate_extent:")),
        "{error:?}"
    );
    assert!(specs.lock().unwrap().is_empty());
    assert_eq!(revision(&app), before);
}

#[test]
fn primitives_with_a_non_positive_height_or_radius_are_refused() {
    let (mut app, mut session, specs) = app_with_modeler(Answer::Body);
    let before = revision(&app);
    // SPHERE with a coincident radius point.
    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Sphere,
                vec![p(0.0, 0.0, 0.0), p(0.0, 0.0, 0.0)],
            ),
        )
        .expect_err("zero radius");
    assert!(
        matches!(&error, CadError::InvalidInput(text) if text.starts_with("sphere.degenerate_extent:")),
        "{error:?}"
    );
    // CYLINDER whose top is below its base.
    let error = app
        .execute(
            &mut session,
            solid_command(
                ExtendedCommandId::Cylinder,
                vec![p(0.0, 0.0, 5.0), p(1.0, 0.0, 5.0), p(0.0, 0.0, 1.0)],
            ),
        )
        .expect_err("negative height");
    assert!(
        matches!(&error, CadError::InvalidInput(text) if text.starts_with("cylinder.degenerate_extent:")),
        "{error:?}"
    );
    assert!(specs.lock().unwrap().is_empty());
    assert_eq!(revision(&app), before);
}

/// The kind tags stay stable: they are the diagnostic prefixes above.
#[test]
fn primitive_kind_tags_are_stable() {
    assert_eq!(PrimitiveKind::Box.as_str(), "box");
    assert_eq!(PrimitiveKind::Torus.as_str(), "torus");
}
