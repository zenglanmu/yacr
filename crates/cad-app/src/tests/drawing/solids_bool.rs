//! ADR-0036 phase C2: boolean **command** contracts (validation + job start).
//!
//! These use a fake [`SolidModeler`] (a hand-built planar unit box) exactly like
//! the C1 primitive contracts: the kernel is not linked into `cad-app`. The
//! worker itself is real (a thread + `CancellationToken`); its commit path is
//! exercised by `host::tests::solids`.
//!
//! Every refusal here happens **before** the job starts, so the drawing, its
//! revision, the id allocator and history stay untouched.

use super::*;
use crate::tasks::SolidBooleanTake;
use cad_kernel_adapter::{
    BooleanOp, ModelerCapabilities, ModelerPrimitives, SolidModeler, SolidOperation, SolidOutcome,
    SolidProduct, SolidRequest, SolidResult,
};
use std::sync::{Arc, Mutex};

struct FakeModeler {
    ops: Arc<Mutex<Vec<BooleanOp>>>,
}

impl SolidModeler for FakeModeler {
    fn registration(&self) -> cad_domain::Registration {
        cad_domain::Registration {
            type_key: "test.fake-solid".into(),
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
            offset_face: false,
            extrude_profile: false,
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
                SolidOutcome::Success(SolidProduct::Body {
                    brep: unit_box(),
                    diagnostics: Vec::new(),
                })
            }
            other => panic!("a boolean command must not send {other:?}"),
        };
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}

fn app_with_modeler() -> (Application, SessionState, Arc<Mutex<Vec<BooleanOp>>>) {
    let (mut app, session) = application_with_document();
    let ops = Arc::new(Mutex::new(Vec::new()));
    app.set_solid_modeler(Some(Arc::new(FakeModeler { ops: ops.clone() })));
    (app, session, ops)
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .count()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map_or(0, |h| h.undo_depth())
}

/// Create one fake body through `BOX` and return its entity id.
fn create_box(app: &mut Application, session: &mut SessionState) -> EntityId {
    let outcome = execute(
        app,
        session,
        CommandId::Extended(ExtendedCommandId::Box),
        CommandPayload::Points(vec![p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0)]),
    )
    .expect("BOX creates a fake body");
    EntityId(outcome.objects[0].0)
}

fn select(session: &mut SessionState, ids: &[EntityId]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: *id,
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn invalid_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) => text,
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

// ---- fixture -------------------------------------------------------------

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
        tolerance: None,
        domain: None,
    }
}

/// A closed planar unit box `[0,1]³`.
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

// ---- contracts ------------------------------------------------------------

#[test]
fn a_boolean_starts_a_job_and_writes_nothing_until_it_settles() {
    for ext in [
        ExtendedCommandId::Union,
        ExtendedCommandId::Subtract,
        ExtendedCommandId::Intersect,
    ] {
        let (mut app, mut session, ops) = app_with_modeler();
        let first = create_box(&mut app, &mut session);
        let second = create_box(&mut app, &mut session);
        select(&mut session, &[first, second]);
        let before = revision(&app);
        let before_count = entity_count(&app);
        let before_undo = undo_depth(&app);

        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ext),
            CommandPayload::None,
        )
        .unwrap_or_else(|error| panic!("{ext:?} starts: {error:?}"));
        assert!(outcome.objects.is_empty(), "{ext:?}: no object exists yet");
        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|d| d.code == "solid.boolean_started"),
            "{ext:?}"
        );
        // The command never touches the database.
        assert_eq!(revision(&app), before, "{ext:?}");
        assert_eq!(entity_count(&app), before_count, "{ext:?}");
        assert_eq!(undo_depth(&app), before_undo, "{ext:?}");

        // Drain the worker so the test does not leave a detached thread.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match app.poll_solid_boolean_take() {
                SolidBooleanTake::Running => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "{ext:?} worker did not settle"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                SolidBooleanTake::Done { .. } => break,
                _ => panic!("{ext:?} fake body should succeed"),
            }
        }
        // A poll alone never commits: publishing is the host's half.
        assert_eq!(revision(&app), before, "{ext:?}");
        assert_eq!(entity_count(&app), before_count, "{ext:?}");
        assert_eq!(ops.lock().unwrap().len(), 1, "{ext:?}");
    }
}

#[test]
fn fewer_than_two_distinct_bodies_is_refused_by_name_and_starts_nothing() {
    let (mut app, mut session, ops) = app_with_modeler();
    let first = create_box(&mut app, &mut session);
    let before = revision(&app);

    // No selection at all.
    session.selection = SelectionSet::new();
    let text = invalid_text(
        execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::Union),
            CommandPayload::None,
        )
        .expect_err("UNION needs two bodies"),
    );
    assert!(text.starts_with("boolean.needs_two:"), "{text}");

    // One body, and the same body repeated: both collapse to one operand.
    select(&mut session, &[first, first]);
    let text = invalid_text(
        execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::Intersect),
            CommandPayload::None,
        )
        .expect_err("one distinct body is not a boolean"),
    );
    assert!(text.starts_with("boolean.needs_two:"), "{text}");

    assert!(ops.lock().unwrap().is_empty(), "the modeler never ran");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 1);
}

#[test]
fn a_non_body_operand_is_refused_by_name() {
    let (mut app, mut session, ops) = app_with_modeler();
    let body = create_box(&mut app, &mut session);
    let line = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(10.0, 0.0)]),
    )
    .expect("LINE");
    let line_id = EntityId(line.objects[0].0);
    select(&mut session, &[body, line_id]);
    let before = revision(&app);

    let text = invalid_text(
        execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::Subtract),
            CommandPayload::None,
        )
        .expect_err("a LINE is not a body"),
    );
    assert!(text.starts_with("boolean.non_solid_operand:"), "{text}");
    assert!(
        text.contains("AcDbLine") || text.contains("not a body"),
        "{text}"
    );
    assert!(ops.lock().unwrap().is_empty(), "the modeler never ran");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
}

#[test]
fn a_sub_element_reference_is_refused_by_name() {
    let (mut app, mut session, ops) = app_with_modeler();
    let first = create_box(&mut app, &mut session);
    let second = create_box(&mut app, &mut session);
    session.selection = SelectionSet::from_refs([
        SelectionRef {
            document: DocumentId(1),
            entity: first,
            instance: InstancePath::default(),
            sub_element: None,
        },
        SelectionRef {
            document: DocumentId(1),
            entity: second,
            instance: InstancePath::default(),
            sub_element: Some(SubElementId {
                source_key: "face:0".into(),
                topology_revision: Revision(0),
            }),
        },
    ]);
    let before = revision(&app);
    let text = invalid_text(
        execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::Union),
            CommandPayload::None,
        )
        .expect_err("a face is not a body operand"),
    );
    assert!(
        text.starts_with("boolean.sub_element_not_supported:"),
        "{text}"
    );
    assert!(ops.lock().unwrap().is_empty(), "the modeler never ran");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
}

#[test]
fn a_non_operand_is_described_by_its_stored_type_not_a_broken_article() {
    let (mut app, mut session, _) = app_with_modeler();
    let body = create_box(&mut app, &mut session);
    let line = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(10.0, 0.0)]),
    )
    .expect("LINE");
    let line_id = EntityId(line.objects[0].0);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let line_row = drawing.entity(line_id).expect("line").clone();
    let text = crate::app_drawing::solids::describe_operand(&line_row);
    assert!(text.contains("AcDbLine"), "{text}");
    assert!(text.contains("not a body"), "{text}");
    assert!(!text.contains("unknown"), "{text}");

    // An exact solid stored under a non-operand key is named accurately: only
    // `AcDb3dSolid` rows are operands, so the sentence must not imply the body is
    // malformed.
    let mut misplaced = drawing.entity(body).expect("body").clone();
    misplaced.object.type_key = "AcDbBody".into();
    let text = crate::app_drawing::solids::describe_operand(&misplaced);
    assert!(text.contains("AcDbBody"), "{text}");
    assert!(text.contains("exact solid body"), "{text}");
    assert!(
        text.contains("only AcDb3dSolid rows are operands"),
        "{text}"
    );

    // The refusal itself uses the same description (no broken "a unknown").
    select(&mut session, &[body, line_id]);
    let message = invalid_text(
        execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::Union),
            CommandPayload::None,
        )
        .expect_err("a LINE is not an operand"),
    );
    assert!(
        message.starts_with("boolean.non_solid_operand:"),
        "{message}"
    );
    assert!(message.contains("AcDbLine"), "{message}");
    assert!(!message.contains("a unknown"), "{message}");
}

#[test]
fn operands_must_share_one_layer() {
    let (mut app, mut session, ops) = app_with_modeler();
    let first = create_box(&mut app, &mut session);
    session.active_layer = LayerId(3);
    let second = create_box(&mut app, &mut session);
    session.active_layer = LayerId(0);
    select(&mut session, &[first, second]);
    let before = revision(&app);

    let text = invalid_text(
        execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::Union),
            CommandPayload::None,
        )
        .expect_err("a boolean never merges layers"),
    );
    assert!(text.starts_with("boolean.layer_mismatch:"), "{text}");
    assert!(ops.lock().unwrap().is_empty(), "the modeler never ran");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
}

#[test]
fn a_carrier_without_a_modeler_refuses_every_boolean_before_it_starts() {
    for ext in [
        ExtendedCommandId::Union,
        ExtendedCommandId::Subtract,
        ExtendedCommandId::Intersect,
    ] {
        let (mut app, mut session) = application_with_document();
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ext),
            CommandPayload::None,
        )
        .expect_err("no modeler, no boolean");
        assert_eq!(
            error,
            CadError::Unsupported("unsupported-runtime: solid-modeler".into()),
            "{ext:?}"
        );
        assert!(matches!(
            app.poll_solid_boolean_take(),
            SolidBooleanTake::Idle
        ));
        assert_eq!(revision(&app), Revision(0));
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn a_multi_shell_or_empty_boolean_result_is_refused_by_name() {
    use crate::app_drawing::solids::{validate_boolean_result, BOOLEAN_RESULT_NOT_SINGLE_SHELL};

    // One closed shell is the representable shape.
    validate_boolean_result(&unit_box()).expect("a unit box is one connected shell");

    // Two shells = a multi-lump body (a disjoint UNION). This build cannot store
    // it faithfully, so the gate refuses before the host can stage anything.
    let mut multi = unit_box();
    let mut second = multi.shells[0].clone();
    second.id = 1;
    multi.shells.push(second);
    let error = validate_boolean_result(&multi).expect_err("two shells are refused");
    match error {
        CadError::Unsupported(reason) => {
            assert!(
                reason.starts_with(BOOLEAN_RESULT_NOT_SINGLE_SHELL),
                "{reason}"
            );
            assert!(reason.contains("2 shell(s)"), "{reason}");
            assert!(reason.contains("nothing was written"), "{reason}");
        }
        other => panic!("a representability refusal is Unsupported, got {other:?}"),
    }

    // A shell with no faces is not a body either.
    let mut empty = unit_box();
    empty.shells[0].faces.clear();
    let error = validate_boolean_result(&empty).expect_err("an empty shell is refused");
    assert!(
        matches!(error, CadError::Unsupported(ref reason) if reason.starts_with(BOOLEAN_RESULT_NOT_SINGLE_SHELL))
    );
}

#[test]
fn a_boolean_takes_no_payload() {
    let (mut app, mut session, ops) = app_with_modeler();
    let first = create_box(&mut app, &mut session);
    let second = create_box(&mut app, &mut session);
    select(&mut session, &[first, second]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Union),
        CommandPayload::Points(vec![p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0)]),
    )
    .expect_err("UNION takes no payload");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(ops.lock().unwrap().is_empty());
}
