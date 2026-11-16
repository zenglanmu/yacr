//! ADR-0036 phase C2: the background boolean's host-side commit.
//!
//! These contracts drive the **real** worker (thread + cancellation token) with
//! a gated fake modeler, then commit through the real `HostController` poll path:
//! one transaction, one undo step, operand display states preserved. The kernel
//! itself is exercised by `cad-cli-tools/tests/solid_command.rs`.

use super::*;
use crate::host::SolidBooleanPoll;
use crate::{CommandId, CommandPayload, ExtendedCommandId, SelectionSet};
use cad_kernel_adapter::{
    BooleanOp, ModelerCapabilities, ModelerPrimitives, SolidModeler, SolidOperation, SolidOutcome,
    SolidProduct, SolidRequest, SolidResult,
};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

// ---- a gated fake modeler -------------------------------------------------

#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    ready: Condvar,
}

impl Gate {
    fn wait(&self) {
        let mut open = self.open.lock().expect("gate");
        while !*open {
            open = self.ready.wait(open).expect("gate wait");
        }
    }

    fn release(&self) {
        *self.open.lock().expect("gate") = true;
        self.ready.notify_all();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Body,
    Unsupported,
}

struct GatedModeler {
    gate: Arc<Gate>,
    answer: Answer,
    ops: Arc<Mutex<Vec<BooleanOp>>>,
}

impl SolidModeler for GatedModeler {
    fn registration(&self) -> cad_domain::Registration {
        cad_domain::Registration {
            type_key: "test.gated-solid".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["3DSOLID".into()],
            capabilities: vec!["booleans".into()],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        ModelerCapabilities {
            primitives: ModelerPrimitives::all(),
            booleans: true,
            mass_properties: false,
            chamfer: false,
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
        let outcome = match request.operation {
            SolidOperation::Primitive { .. } => SolidOutcome::Success(SolidProduct::Body {
                brep: unit_box(),
                diagnostics: Vec::new(),
            }),
            SolidOperation::Boolean { op, .. } => {
                self.ops.lock().expect("op log").push(op);
                // Block until the test has observed `Running`.
                self.gate.wait();
                match self.answer {
                    Answer::Body => SolidOutcome::Success(SolidProduct::Body {
                        brep: unit_box(),
                        diagnostics: vec![Diagnostic {
                            object: None,
                            code: "solid.test_note".into(),
                            message: "the fake kernel folded the operands".into(),
                        }],
                    }),
                    Answer::Unsupported => SolidOutcome::Unsupported {
                        reason: cad_kernel_adapter::SolidUnsupportedReason {
                            code: "solid.lift_requires_planar",
                            operation: request.operation.family(),
                            detail: "the fake kernel only folds planar bodies".into(),
                        },
                    },
                }
            }
            other => panic!("a boolean must not send {other:?}"),
        };
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}

/// A modeler whose boolean call panics immediately: the task seam must report a
/// failure, never a cancel, and never publish a body.
struct PanicModeler;

impl SolidModeler for PanicModeler {
    fn registration(&self) -> cad_domain::Registration {
        cad_domain::Registration {
            type_key: "test.panic-solid".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["3DSOLID".into()],
            capabilities: vec!["booleans".into()],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        ModelerCapabilities {
            primitives: ModelerPrimitives::all(),
            booleans: true,
            mass_properties: false,
            chamfer: false,
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
        match request.operation {
            SolidOperation::Primitive { .. } => Ok(SolidResult {
                stamp: request.stamp.clone(),
                outcome: SolidOutcome::Success(SolidProduct::Body {
                    brep: unit_box(),
                    diagnostics: Vec::new(),
                }),
            }),
            SolidOperation::Boolean { .. } => panic!("the fake kernel exploded"),
            other => panic!("a boolean must not send {other:?}"),
        }
    }
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn controller_with(modeler: Arc<dyn SolidModeler>) -> HostController {
    let mut controller =
        HostController::with_blank_document([800.0, 600.0]).expect("blank document");
    controller.application.set_solid_modeler(Some(modeler));
    controller
}

fn controller(answer: Answer) -> (HostController, Arc<Gate>, Arc<Mutex<Vec<BooleanOp>>>) {
    let gate = Arc::new(Gate::default());
    let ops = Arc::new(Mutex::new(Vec::new()));
    let modeler: Arc<dyn SolidModeler> = Arc::new(GatedModeler {
        gate: gate.clone(),
        answer,
        ops: ops.clone(),
    });
    (controller_with(modeler), gate, ops)
}

/// `COLOR <aci>`: the session token the create funnel stamps on new rows.
fn set_color(controller: &mut HostController, aci: i16) {
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::Extended(ExtendedCommandId::Color),
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::CurrentColor(aci),
        })
        .unwrap_or_else(|error| panic!("COLOR {aci} must store: {error:?}"));
}

fn box_command(controller: &HostController, x: f64) -> Command {
    Command {
        schema_version: 1,
        id: CommandId::Extended(ExtendedCommandId::Box),
        document: controller.document_id,
        viewport: controller.viewport_id,
        payload: CommandPayload::Points(vec![p3(x, 0.0, 0.0), p3(x + 1.0, 1.0, 1.0)]),
    }
}

fn create_box(controller: &mut HostController, x: f64) -> EntityId {
    let outcome = controller.execute(box_command(controller, x)).expect("BOX");
    EntityId(outcome.objects[0].0)
}

fn select(controller: &mut HostController, ids: &[EntityId]) {
    controller.session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: controller.document_id,
        entity: *id,
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn boolean(controller: &HostController, ext: ExtendedCommandId) -> Command {
    Command {
        schema_version: 1,
        id: CommandId::Extended(ext),
        document: controller.document_id,
        viewport: controller.viewport_id,
        payload: CommandPayload::None,
    }
}

fn entity_count(controller: &HostController) -> usize {
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entities()
        .count()
}

fn revision(controller: &HostController) -> Revision {
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .revision()
}

fn undo_depth(controller: &HostController) -> usize {
    controller
        .application
        .history
        .get(&controller.document_id)
        .map_or(0, |history| history.undo_depth())
}

fn is_solid(controller: &HostController, id: EntityId) -> bool {
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entity(id)
        .is_some_and(|entity| {
            entity.object.type_key == "AcDb3dSolid"
                && matches!(
                    &entity.geometry,
                    SemanticGeometry::Body3d(body)
                        if body.kind == BodyKind::Solid
                            && matches!(body.exact, BodyExact::Exact(_))
                )
        })
}

/// Poll until the job leaves `Running` (the gate must already be released).
fn wait_settled(controller: &mut HostController) -> SolidBooleanPoll {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match controller.poll_solid_boolean() {
            SolidBooleanPoll::Running => {
                assert!(Instant::now() < deadline, "the boolean did not settle");
                std::thread::sleep(Duration::from_millis(2));
            }
            poll => return poll,
        }
    }
}

// ---- fixture: a closed planar unit box ------------------------------------

fn vsub(a: Point3, b: Point3) -> Point3 {
    p3(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn vcross(a: Point3, b: Point3) -> Point3 {
    p3(
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
        p3(v.x / length, v.y / length, v.z / length)
    } else {
        p3(0.0, 0.0, 0.0)
    }
}

fn plane_face(id: u32, cycle: &[Point3]) -> BrepFace {
    let origin = cycle[0];
    let u_dir = cycle
        .iter()
        .map(|corner| unit(vsub(*corner, origin)))
        .find(|direction| vlen(*direction) > 0.0)
        .unwrap_or(p3(1.0, 0.0, 0.0));
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

/// A closed planar unit box `[0,1]³`.
fn unit_box() -> BrepData {
    let corners = [
        p3(0.0, 0.0, 0.0),
        p3(1.0, 0.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(0.0, 1.0, 0.0),
        p3(0.0, 0.0, 1.0),
        p3(1.0, 0.0, 1.0),
        p3(1.0, 1.0, 1.0),
        p3(0.0, 1.0, 1.0),
    ];
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

// ---- commit contracts -----------------------------------------------------

#[test]
fn union_commits_one_body_deletes_the_operands_and_one_undo_restores_them() {
    let (mut controller, gate, ops) = controller(Answer::Body);
    let first = create_box(&mut controller, 0.0);
    let second = create_box(&mut controller, 0.5);
    select(&mut controller, &[first, second]);
    let revision_before = revision(&controller);
    let undo_before = undo_depth(&controller);

    controller
        .execute(boolean(&controller, ExtendedCommandId::Union))
        .expect("UNION starts a job");
    // The command wrote nothing yet.
    assert_eq!(entity_count(&controller), 2);
    assert_eq!(revision(&controller), revision_before);
    assert_eq!(undo_depth(&controller), undo_before);
    assert!(
        matches!(controller.poll_solid_boolean(), SolidBooleanPoll::Running),
        "the gated worker is still running"
    );

    gate.release();
    let poll = wait_settled(&mut controller);
    let SolidBooleanPoll::Done {
        object,
        diagnostics,
    } = poll
    else {
        panic!("UNION should commit, got {poll:?}");
    };
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "solid.test_note"),
        "the kernel notes are carried through: {diagnostics:?}"
    );
    assert_eq!(entity_count(&controller), 1, "the operands are gone");
    assert!(is_solid(&controller, EntityId(object.0)));
    assert!(
        !is_solid(&controller, first),
        "the first operand was deleted"
    );
    assert!(!is_solid(&controller, second));
    assert_eq!(revision(&controller), Revision(revision_before.0 + 1));
    assert_eq!(undo_depth(&controller), undo_before + 1, "one undo step");
    assert_eq!(*ops.lock().unwrap(), vec![BooleanOp::Union]);

    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::Undo,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("undo");
    assert_eq!(entity_count(&controller), 2, "one undo restores the group");
    assert!(is_solid(&controller, first));
    assert!(is_solid(&controller, second));
    assert!(
        !is_solid(&controller, EntityId(object.0)),
        "the combined body is gone"
    );

    // Redo replays the same one-step record: one body again, operands gone.
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::Redo,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("redo");
    assert_eq!(entity_count(&controller), 1, "redo restores the result");
    assert!(is_solid(&controller, EntityId(object.0)));
    assert!(!is_solid(&controller, first));
    assert!(!is_solid(&controller, second));
}

#[test]
fn subtract_and_intersect_commit_one_body_each() {
    for (ext, expected) in [
        (ExtendedCommandId::Subtract, BooleanOp::Subtract),
        (ExtendedCommandId::Intersect, BooleanOp::Intersect),
    ] {
        let (mut controller, gate, ops) = controller(Answer::Body);
        let first = create_box(&mut controller, 0.0);
        let second = create_box(&mut controller, 0.5);
        select(&mut controller, &[first, second]);
        controller
            .execute(boolean(&controller, ext))
            .unwrap_or_else(|error| panic!("{ext:?} starts: {error:?}"));
        assert!(
            matches!(controller.poll_solid_boolean(), SolidBooleanPoll::Running),
            "{ext:?} runs on a worker, not on the command thread"
        );
        gate.release();
        let poll = wait_settled(&mut controller);
        assert!(
            matches!(poll, SolidBooleanPoll::Done { .. }),
            "{ext:?}: {poll:?}"
        );
        assert_eq!(entity_count(&controller), 1, "{ext:?}");
        assert_eq!(*ops.lock().unwrap(), vec![expected], "{ext:?}");
    }
}

#[test]
fn an_unsupported_kernel_answer_is_reported_and_writes_nothing() {
    let (mut controller, gate, _) = controller(Answer::Unsupported);
    let first = create_box(&mut controller, 0.0);
    let second = create_box(&mut controller, 0.5);
    select(&mut controller, &[first, second]);
    let revision_before = revision(&controller);
    let undo_before = undo_depth(&controller);
    controller
        .execute(boolean(&controller, ExtendedCommandId::Union))
        .expect("starts");

    gate.release();
    match wait_settled(&mut controller) {
        SolidBooleanPoll::Failed { error } => assert_eq!(
            error,
            CadError::Unsupported(
                "solid.lift_requires_planar: the fake kernel only folds planar bodies".into()
            )
        ),
        poll => panic!("an unsupported operand must fail, got {poll:?}"),
    }
    assert_eq!(entity_count(&controller), 2);
    assert_eq!(revision(&controller), revision_before);
    assert_eq!(undo_depth(&controller), undo_before);
}

#[test]
fn an_operand_edited_while_the_job_ran_discards_the_result_as_stale() {
    let (mut controller, gate, _) = controller(Answer::Body);
    let first = create_box(&mut controller, 0.0);
    let second = create_box(&mut controller, 0.5);
    select(&mut controller, &[first, second]);
    controller
        .execute(boolean(&controller, ExtendedCommandId::Union))
        .expect("starts");
    assert!(matches!(
        controller.poll_solid_boolean(),
        SolidBooleanPoll::Running
    ));

    // Edit one operand *after* the snapshot: the worker's result is now based on
    // geometry the drawing no longer has.
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::MoveEntities,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::Move {
                refs: vec![SelectionRef {
                    document: controller.document_id,
                    entity: second,
                    instance: InstancePath::default(),
                    sub_element: None,
                }],
                delta: p3(5.0, 0.0, 0.0),
            },
        })
        .expect("MOVE");
    let revision_after_move = revision(&controller);
    let undo_after_move = undo_depth(&controller);

    gate.release();
    let poll = wait_settled(&mut controller);
    assert!(
        matches!(poll, SolidBooleanPoll::Stale),
        "a changed operand discards the result, got {poll:?}"
    );
    assert_eq!(entity_count(&controller), 2, "the edit survives");
    assert_eq!(revision(&controller), revision_after_move);
    assert_eq!(undo_depth(&controller), undo_after_move);
    assert!(is_solid(&controller, first));
    assert!(is_solid(&controller, second));
}

#[test]
fn cancelling_the_job_never_publishes() {
    let (mut controller, gate, _) = controller(Answer::Body);
    let first = create_box(&mut controller, 0.0);
    let second = create_box(&mut controller, 0.5);
    select(&mut controller, &[first, second]);
    let revision_before = revision(&controller);
    let undo_before = undo_depth(&controller);
    controller
        .execute(boolean(&controller, ExtendedCommandId::Subtract))
        .expect("starts");
    assert!(matches!(
        controller.poll_solid_boolean(),
        SolidBooleanPoll::Running
    ));
    controller.cancel_solid_boolean();
    gate.release();
    let poll = wait_settled(&mut controller);
    assert!(
        matches!(poll, SolidBooleanPoll::Cancelled),
        "a cancelled job reports Cancelled, got {poll:?}"
    );
    assert_eq!(entity_count(&controller), 2);
    assert_eq!(revision(&controller), revision_before);
    assert_eq!(undo_depth(&controller), undo_before);
    // The slot is released; a further poll is genuinely idle.
    assert!(matches!(
        controller.poll_solid_boolean(),
        SolidBooleanPoll::Idle
    ));
}

#[test]
fn replacing_the_document_cancels_the_job_before_it_can_publish() {
    let (mut controller, gate, _) = controller(Answer::Body);
    let first = create_box(&mut controller, 0.0);
    let second = create_box(&mut controller, 0.5);
    select(&mut controller, &[first, second]);
    controller
        .execute(boolean(&controller, ExtendedCommandId::Union))
        .expect("starts");
    assert!(matches!(
        controller.poll_solid_boolean(),
        SolidBooleanPoll::Running
    ));
    controller
        .new_blank_document()
        .expect("New supersedes the job");
    gate.release();
    assert!(matches!(
        controller.poll_solid_boolean(),
        SolidBooleanPoll::Idle | SolidBooleanPoll::Cancelled
    ));
    assert_eq!(entity_count(&controller), 0, "the fresh document is empty");
}

#[test]
fn a_second_boolean_supersedes_the_first_and_the_old_result_never_publishes() {
    let (mut controller, gate, _) = controller(Answer::Body);
    let first = create_box(&mut controller, 0.0);
    let second = create_box(&mut controller, 0.5);
    select(&mut controller, &[first, second]);

    controller
        .execute(boolean(&controller, ExtendedCommandId::Union))
        .expect("the first boolean starts");
    assert!(matches!(
        controller.poll_solid_boolean(),
        SolidBooleanPoll::Running
    ));
    // Supersede while the first worker is still inside the kernel call.
    controller
        .execute(boolean(&controller, ExtendedCommandId::Intersect))
        .expect("the second boolean supersedes the first");
    gate.release();

    let poll = wait_settled(&mut controller);
    assert!(
        matches!(poll, SolidBooleanPoll::Done { .. }),
        "only the current job publishes, got {poll:?}"
    );
    assert_eq!(entity_count(&controller), 1, "exactly one combined body");
    assert_eq!(
        undo_depth(&controller),
        3,
        "two BOX creates + exactly one boolean record"
    );
    assert!(!is_solid(&controller, first));
    assert!(!is_solid(&controller, second));
    assert!(matches!(
        controller.poll_solid_boolean(),
        SolidBooleanPoll::Idle
    ));
}

#[test]
fn a_panicking_worker_is_failed_never_cancelled() {
    let mut controller = controller_with(Arc::new(PanicModeler));
    let first = create_box(&mut controller, 0.0);
    let second = create_box(&mut controller, 0.5);
    select(&mut controller, &[first, second]);
    let undo_before = undo_depth(&controller);
    controller
        .execute(boolean(&controller, ExtendedCommandId::Union))
        .expect("starts");

    match wait_settled(&mut controller) {
        SolidBooleanPoll::Failed { error } => match error {
            CadError::Invariant(reason) => assert!(
                reason.starts_with(crate::tasks::SOLID_BOOLEAN_WORKER_PANICKED),
                "{reason}"
            ),
            other => panic!("a panic must be Invariant, got {other:?}"),
        },
        SolidBooleanPoll::Cancelled => panic!("a panic must never be reported as a cancel"),
        poll => panic!("a panic must fail, got {poll:?}"),
    }
    assert_eq!(entity_count(&controller), 2, "nothing published");
    assert_eq!(undo_depth(&controller), undo_before, "no undo step");
    assert!(is_solid(&controller, first));
    assert!(is_solid(&controller, second));
}

#[test]
fn the_result_inherits_the_first_operand_layer_space_and_draw_order() {
    let (mut controller, gate, _) = controller(Answer::Body);
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::CreateLayer,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::CreateLayer("WALLS".into()),
        })
        .expect("layer create");
    let document = controller.document_id;
    let layer = controller.application.workspace.documents[&document]
        .drawing
        .layers()
        .find(|layer| layer.name == "WALLS")
        .expect("WALLS exists")
        .id;
    controller.session.active_layer = layer;
    let first = create_box(&mut controller, 0.0);
    let second = create_box(&mut controller, 0.5);
    let (first_order, second_order) = {
        let drawing = &controller.application.workspace.documents[&document].drawing;
        (
            drawing.entity(first).expect("first").draw_order,
            drawing.entity(second).expect("second").draw_order,
        )
    };
    assert!(second_order > first_order, "the second body is later");

    select(&mut controller, &[first, second]);
    controller
        .execute(boolean(&controller, ExtendedCommandId::Union))
        .expect("starts");
    gate.release();
    let poll = wait_settled(&mut controller);
    let SolidBooleanPoll::Done { object, .. } = poll else {
        panic!("UNION should commit, got {poll:?}");
    };
    let drawing = &controller.application.workspace.documents[&document].drawing;
    let entity = drawing
        .entity(EntityId(object.0))
        .expect("the combined row exists");
    assert_eq!(entity.layer, layer, "first operand's layer");
    assert_eq!(entity.space, SpaceId::Model, "first operand's space");
    assert_eq!(entity.draw_order, first_order, "first operand's draw order");
}

#[test]
fn the_result_inherits_the_first_operand_explicit_display_state() {
    let (mut controller, gate, _) = controller(Answer::Body);
    set_color(&mut controller, 5);
    let first = create_box(&mut controller, 0.0);
    set_color(&mut controller, 256);
    let second = create_box(&mut controller, 0.5);
    let rgb = crate::property_value::rgb_for_aci(5).expect("ACI 5 is a real colour");
    let explicit = cad_db::EntityColor::Explicit(rgb);
    select(&mut controller, &[first, second]);
    controller
        .execute(boolean(&controller, ExtendedCommandId::Union))
        .expect("starts");
    gate.release();
    let poll = wait_settled(&mut controller);
    let SolidBooleanPoll::Done { object, .. } = poll else {
        panic!("UNION should commit, got {poll:?}");
    };
    let combined = EntityId(object.0);
    let document = controller.document_id;
    {
        let drawing = &controller.application.workspace.documents[&document].drawing;
        assert_eq!(
            drawing.entity_render_attributes(combined).color,
            explicit,
            "the result keeps the first operand's colour"
        );
    }

    // One undo restores both operands with their own states.
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::Undo,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("undo");
    {
        let drawing = &controller.application.workspace.documents[&document].drawing;
        assert_eq!(
            drawing.entity_render_attributes(first).color,
            explicit,
            "the first operand is restored with its colour"
        );
        assert_eq!(
            drawing.entity_render_attributes(second).color,
            cad_db::EntityColor::ByLayer,
            "the second operand is restored ByLayer"
        );
    }

    // Redo replays the insert patch: the result comes back with the override.
    controller
        .execute(Command {
            schema_version: 1,
            id: CommandId::Redo,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: CommandPayload::None,
        })
        .expect("redo");
    let drawing = &controller.application.workspace.documents[&document].drawing;
    assert_eq!(
        drawing.entity_render_attributes(combined).color,
        explicit,
        "redo restores the result's inherited display state"
    );
}

#[test]
fn a_primitive_while_a_boolean_runs_is_refused_by_name_before_any_write() {
    let (mut controller, gate, _) = controller(Answer::Body);
    let first = create_box(&mut controller, 0.0);
    let second = create_box(&mut controller, 0.5);
    select(&mut controller, &[first, second]);
    controller
        .execute(boolean(&controller, ExtendedCommandId::Union))
        .expect("starts");
    assert!(matches!(
        controller.poll_solid_boolean(),
        SolidBooleanPoll::Running
    ));
    let count_before = entity_count(&controller);
    let undo_before = undo_depth(&controller);

    let error = controller
        .execute(box_command(&controller, 9.0))
        .expect_err("the shared kernel mutex is busy");
    assert_eq!(
        error,
        CadError::Unsupported(
            "solid.boolean_busy: a boolean is running; wait for it to settle before creating another solid"
                .into()
        )
    );
    assert_eq!(
        entity_count(&controller),
        count_before,
        "nothing was written"
    );
    assert_eq!(undo_depth(&controller), undo_before, "no undo step");

    // Settle the job so the test leaves no blocked worker behind.
    controller.cancel_solid_boolean();
    gate.release();
    let _ = wait_settled(&mut controller);
}
