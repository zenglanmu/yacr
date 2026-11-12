//! Batch-edit contracts added by the R1-A audit: `MPEDIT` conversion and
//! batch options, the `MOCORO` chain, and `CHAMFER`'s angle method / No-Trim.
//!
//! They live in their own module so no test file exceeds the 1000-line ceiling;
//! the shared fixtures (`p3`, `point`, `ref_of`, `execute`, `revision`) come from
//! the parent drawing test module.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn ent(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn build(entities: Vec<DbEntity>, mode: AppMode) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), mode))
}

fn geometry(app: &Application, id: u128) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .unwrap_or_else(|| panic!("entity {id} exists"))
        .geometry
        .clone()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Remaining corner/transform contracts from the R1-A audit: MPEDIT batch
// conversion, MOCORO's validated chain, and CHAMFER's angle method with the
// No-Trim switch. Helpers (`build`, `ent`, `ref_of`, `revision`, `undo_depth`)
// are the file's own.
// ---------------------------------------------------------------------------

fn set_number(session: &mut SessionState, parameter: ToolParameter, value: f64) {
    session
        .set_tool_parameter(parameter, ToolValue::Number(value))
        .unwrap_or_else(|error| panic!("{parameter:?} = {value} refused: {error}"));
}

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p3(x0, y0, 0.0),
        end: p3(x1, y1, 0.0),
    }
}

fn open_polyline(points: Vec<(f64, f64)>) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points: points.into_iter().map(|(x, y)| p3(x, y, 0.0)).collect(),
        bulges: Vec::new(),
        closed: false,
        width: 0.0,
    }
}

fn closed_square(side: f64) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points: vec![
            p3(0.0, 0.0, 0.0),
            p3(side, 0.0, 0.0),
            p3(side, side, 0.0),
            p3(0.0, side, 0.0),
        ],
        bulges: vec![0.0; 4],
        closed: true,
        width: 0.0,
    }
}

fn select(app: &mut Application, session: &mut SessionState, ids: &[u128]) {
    let refs: Vec<SelectionRef> = ids.iter().map(|id| ref_of(*id)).collect();
    session.selection = SelectionSet::from_refs(refs);
    let _ = app;
}

fn mpedit_payload(option: PeditOption) -> CommandPayload {
    CommandPayload::Draw2(Box::new(Draw2Payload::Mpedit(option)))
}

fn pedit_edit(option: PeditEditOption, convert: bool) -> CommandPayload {
    CommandPayload::PeditEdit(PeditEditCommand { option, convert })
}

#[test]
fn mpedit_applies_close_open_reverse_to_every_selection_in_one_undo_step() {
    let (mut app, mut session) = build(
        vec![
            ent(
                1,
                open_polyline(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]),
                0,
            ),
            ent(2, closed_square(3.0), 1),
            ent(
                3,
                open_polyline(vec![(5.0, 0.0), (6.0, 0.0), (6.0, 1.0)]),
                2,
            ),
        ],
        AppMode::Work,
    );
    select(&mut app, &mut session, &[1, 2, 3]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Mpedit,
        mpedit_payload(PeditOption::Close),
    )
    .expect("MPEDIT CLOSE batches the selection");
    assert_eq!(revision(&app).0, before.0 + 1, "one revision bump");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    for id in [1u128, 3] {
        let SemanticGeometry::Polyline { closed, .. } = geometry(&app, id) else {
            panic!("entity {id} must stay a polyline");
        };
        assert!(closed, "entity {id} is closed");
    }
    assert!(
        geometry(&app, 2) == closed_square(3.0),
        "an already-closed polyline is unchanged"
    );
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "mpedit.unchanged"),
        "the no-op is reported by name"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let SemanticGeometry::Polyline { closed, .. } = geometry(&app, 1) else {
        panic!("undone entity is the original polyline");
    };
    assert!(!closed);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn mpedit_convert_turns_lines_and_arcs_into_polylines_in_one_transaction() {
    let (mut app, mut session) = build(
        vec![
            ent(1, line(0.0, 0.0, 2.0, 0.0), 0),
            ent(
                2,
                SemanticGeometry::Arc {
                    center: p3(0.0, 0.0, 0.0),
                    normal: p3(0.0, 0.0, 1.0),
                    radius: 1.0,
                    start: 0.0,
                    sweep: std::f64::consts::FRAC_PI_2,
                },
                1,
            ),
        ],
        AppMode::Work,
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Mpedit,
        pedit_edit(PeditEditOption::Reverse, true),
    )
    .expect("MPEDIT converts LINE/ARC and reverses them");
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(outcome.objects.len(), 2);

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let first = drawing.entity(EntityId(1)).unwrap();
    assert_eq!(
        first.object.type_key, "AcDbPolyline",
        "the type is re-derived"
    );
    let SemanticGeometry::Polyline { points, .. } = &first.geometry else {
        panic!("the LINE became a POLYLINE, got {:?}", first.geometry);
    };
    assert_eq!(points.len(), 2);
    assert!(
        (points[0].x - 2.0).abs() < 1e-12 && (points[1].x).abs() < 1e-12,
        "REVERSE runs after the conversion: {points:?}"
    );

    let second = drawing.entity(EntityId(2)).unwrap();
    assert_eq!(second.object.type_key, "AcDbPolyline");
    let SemanticGeometry::Polyline { points, bulges, .. } = &second.geometry else {
        panic!("the ARC became a POLYLINE, got {:?}", second.geometry);
    };
    assert_eq!(points.len(), 2);
    assert!(
        points[0].y > 0.9 && points[1].x > 0.9,
        "the reversed arc starts at its old end point: {points:?}"
    );
    assert!(
        bulges[0] < 0.0,
        "the reversed arc bulge flips sign: {bulges:?}"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(matches!(geometry(&app, 1), SemanticGeometry::Line { .. }));
    assert!(matches!(geometry(&app, 2), SemanticGeometry::Arc { .. }));
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .unwrap()
            .object
            .type_key,
        "AcDbEntity"
    );
}

#[test]
fn mpedit_without_convert_skips_non_polylines_and_writes_nothing() {
    let (mut app, mut session) = build(vec![ent(1, line(0.0, 0.0, 2.0, 0.0), 0)], AppMode::Work);
    select(&mut app, &mut session, &[1]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Mpedit,
        mpedit_payload(PeditOption::Reverse),
    )
    .expect("no convertible object is a named no-op, not an error");
    assert_eq!(revision(&app), before, "nothing was written");
    assert_eq!(undo_depth(&app), 0, "no history");
    assert!(outcome.changes.is_none());
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "mpedit.skipped"),
        "the skip is reported: {:?}",
        outcome.diagnostics
    );
}

#[test]
fn mpedit_delegates_pedit_planners_and_keeps_their_selection_rules() {
    // One selected polyline: FIT runs through PEDIT's own planner (one undo step).
    let (mut app, mut session) = build(
        vec![ent(
            1,
            open_polyline(vec![(0.0, 0.0), (1.0, 0.3), (2.0, 0.0)]),
            0,
        )],
        AppMode::Work,
    );
    select(&mut app, &mut session, &[1]);
    let before = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::Mpedit,
        pedit_edit(PeditEditOption::Fit, false),
    )
    .expect("MPEDIT delegates FIT");
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);
    let SemanticGeometry::Polyline { bulges, .. } = geometry(&app, 1) else {
        panic!("FIT keeps a polyline");
    };
    assert!(bulges.iter().any(|b| b.abs() > 1e-9), "FIT inserts arcs");

    // Two selected polylines: PEDIT's single-object rule wins by name and writes
    // nothing rather than silently editing just one of them.
    let (mut app, mut session) = build(
        vec![
            ent(
                1,
                open_polyline(vec![(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)]),
                0,
            ),
            ent(
                2,
                open_polyline(vec![(5.0, 0.0), (6.0, 1.0), (7.0, 0.0)]),
                1,
            ),
        ],
        AppMode::Work,
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Mpedit,
        pedit_edit(PeditEditOption::Fit, false),
    )
    .unwrap_err();
    assert!(error.to_string().contains("pedit.selection"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    // An unknown payload is a named refusal.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Mpedit,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("mpedit.payload"), "{error}");
    assert_eq!(revision(&app), before);
}

#[test]
fn mocoro_chain_writes_one_undo_step_per_operation_and_undo_restores_it() {
    let (mut app, mut session) = build(vec![ent(1, line(0.0, 0.0, 2.0, 0.0), 0)], AppMode::Work);
    select(&mut app, &mut session, &[1]);
    let before = revision(&app);
    let mut expected = 0u64;

    execute(
        &mut app,
        &mut session,
        CommandId::Mocoro,
        CommandPayload::Mocoro(MocoroCommand::Move {
            delta: p3(3.0, 4.0, 0.0),
        }),
    )
    .expect("MOCORO MOVE");
    expected += 1;
    assert_eq!(undo_depth(&app), expected as usize);
    let SemanticGeometry::Line { start, .. } = geometry(&app, 1) else {
        panic!("MOVE keeps a LINE");
    };
    assert!((start.x - 3.0).abs() < 1e-12 && (start.y - 4.0).abs() < 1e-12);

    execute(
        &mut app,
        &mut session,
        CommandId::Mocoro,
        CommandPayload::Mocoro(MocoroCommand::Rotate {
            base: p3(3.0, 4.0, 0.0),
            degrees: 90.0,
        }),
    )
    .expect("MOCORO ROTATE");
    expected += 1;
    assert_eq!(undo_depth(&app), expected as usize);
    let SemanticGeometry::Line { end, .. } = geometry(&app, 1) else {
        panic!("ROTATE keeps a LINE");
    };
    assert!((end.x - 3.0).abs() < 1e-9 && (end.y - 6.0).abs() < 1e-9);

    execute(
        &mut app,
        &mut session,
        CommandId::Mocoro,
        CommandPayload::Mocoro(MocoroCommand::Scale {
            base: p3(3.0, 4.0, 0.0),
            factor: 2.0,
        }),
    )
    .expect("MOCORO SCALE");
    expected += 1;
    assert_eq!(undo_depth(&app), expected as usize);
    let SemanticGeometry::Line { end, .. } = geometry(&app, 1) else {
        panic!("SCALE keeps a LINE");
    };
    assert!((end.y - 8.0).abs() < 1e-9);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Mocoro,
        CommandPayload::Mocoro(MocoroCommand::Copy {
            delta: p3(10.0, 0.0, 0.0),
        }),
    )
    .expect("MOCORO COPY");
    expected += 1;
    assert_eq!(
        outcome.diagnostics[0].code, "drawing.mocoro",
        "MOCORO attributes its single undo step"
    );
    assert_eq!(undo_depth(&app), expected as usize);
    assert_eq!(
        revision(&app).0,
        before.0 + 4,
        "each of the four operations bumps exactly one revision"
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        2,
        "COPY keeps the source"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        1,
        "one undo removes only the copy"
    );
}

#[test]
fn mocoro_refuses_bad_numbers_and_an_empty_selection_before_writing() {
    let (mut app, mut session) = build(vec![ent(1, line(0.0, 0.0, 2.0, 0.0), 0)], AppMode::Work);
    select(&mut app, &mut session, &[1]);
    let before = revision(&app);
    let cases = [
        (
            MocoroCommand::Move {
                delta: p3(f64::NAN, 0.0, 0.0),
            },
            "mocoro.invalid_delta",
        ),
        (
            MocoroCommand::Rotate {
                base: p3(f64::INFINITY, 0.0, 0.0),
                degrees: 90.0,
            },
            "mocoro.invalid_base",
        ),
        (
            MocoroCommand::Rotate {
                base: p3(0.0, 0.0, 0.0),
                degrees: f64::NAN,
            },
            "mocoro.invalid_angle",
        ),
        (
            MocoroCommand::Scale {
                base: p3(0.0, 0.0, 0.0),
                factor: 0.0,
            },
            "mocoro.invalid_factor",
        ),
    ];
    for (operation, code) in cases {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Mocoro,
            CommandPayload::Mocoro(operation),
        )
        .unwrap_err();
        assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
        assert!(error.to_string().contains(code), "{error} must name {code}");
        assert_eq!(revision(&app), before, "a refusal writes nothing");
        assert_eq!(undo_depth(&app), 0, "a refusal records no undo step");
    }

    session.selection.clear();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Mocoro,
        CommandPayload::Mocoro(MocoroCommand::Move {
            delta: p3(1.0, 0.0, 0.0),
        }),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("mocoro.selection_required"),
        "{error}"
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn chamfer_angle_zero_is_a_named_refusal_and_trim_off_creates_only_the_join() {
    let (mut app, mut session) = build(
        vec![
            ent(1, line(0.0, 0.0, 10.0, 0.0), 0),
            ent(2, line(0.0, 0.0, 0.0, 10.0), 1),
        ],
        AppMode::Work,
    );
    set_number(&mut session, ToolParameter::ChamferDistanceA, 2.0);
    session
        .drafting_extras
        .corner_modes_mut()
        .set_chamfer_method(crate::session_state::ChamferMethod::Angle);
    session
        .drafting_extras
        .corner_modes_mut()
        .set_chamfer_angle_degrees(0.0)
        .unwrap();
    let before = revision(&app);
    let payload = CommandPayload::Chamfer(Box::new(TwoLineCommand {
        first: ref_of(1),
        second: ref_of(2),
        pick_first: p3(8.0, 0.0, 0.0),
        pick_second: p3(0.0, 8.0, 0.0),
    }));
    let error = execute(&mut app, &mut session, CommandId::ChamferLines, payload).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(
        error.to_string().contains("CHAMFER.angle_zero"),
        "{error} must name CHAMFER.angle_zero"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    // A real angle with No-Trim moves neither object and creates only the LINE.
    session
        .drafting_extras
        .corner_modes_mut()
        .set_chamfer_angle_degrees(30.0)
        .unwrap();
    session.drafting_extras.corner_modes_mut().set_trim(false);
    let payload = CommandPayload::Chamfer(Box::new(TwoLineCommand {
        first: ref_of(1),
        second: ref_of(2),
        pick_first: p3(8.0, 0.0, 0.0),
        pick_second: p3(0.0, 8.0, 0.0),
    }));
    let outcome = execute(&mut app, &mut session, CommandId::ChamferLines, payload)
        .expect("the angle method with No-Trim is supported");
    assert_eq!(outcome.objects.len(), 1, "only the joining LINE");
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);
    let SemanticGeometry::Line { start, end } = geometry(&app, 1) else {
        panic!("entity 1 is untouched");
    };
    assert!((start.x).abs() < 1e-12 && (end.x - 10.0).abs() < 1e-12);
    let SemanticGeometry::Line { start, end } = geometry(&app, 2) else {
        panic!("entity 2 is untouched");
    };
    assert!((start.y).abs() < 1e-12 && (end.y - 10.0).abs() < 1e-12);
}
