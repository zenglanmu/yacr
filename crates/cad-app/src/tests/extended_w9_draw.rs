//! Wave 9 real-edit contracts: `BLEND`, `BREAKLINE` and `ARCTEXT`, plus the
//! `ARRAY` router report and the stable named refusals.
//!
//! A real edit must create exactly the declared entity and commit exactly one
//! transaction / one undo step. A refusal must write nothing, bump no revision
//! and record no history.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(id: u128, start: Point3, end: Point3) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line { start, end },
        draw_order: id as i64,
    }
}

/// One `ARC` on the `+Z` plane, in radians.
fn arc(id: u128, center: Point3, normal: Point3, radius: f64, start: f64, sweep: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbArc".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        },
        draw_order: id as i64,
    }
}

/// One `TEXT` row at `position` with height `2`.
fn text_at(id: u128, content: &str, position: Point3) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbText".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(3),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: content.into(),
            position,
            style: StyleId(0),
            height: 2.0,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
            width_factor: 1.0,
        },
        draw_order: id as i64,
    }
}

/// Replace the default document with one holding the given entities.
fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn model_entities(app: &Application) -> Vec<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .into_iter()
        .cloned()
        .collect()
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::Extended(ext), payload))
}

#[test]
fn breakline_creates_one_open_polyline_with_the_declared_symbol() {
    let (mut app, mut session) = app_with(Vec::new());
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::BreakLine,
        CommandPayload::Points(vec![p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)]),
    )
    .expect("BREAKLINE is admitted");
    assert!(
        outcome.changes.is_some(),
        "BREAKLINE writes one transaction"
    );
    let entities = model_entities(&app);
    assert_eq!(entities.len(), 1);
    match &entities[0].geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } => {
            assert_eq!(points.len(), 7, "start, five symbol points, end");
            assert!(bulges.is_empty());
            assert!(!closed, "BREAKLINE creates an open polyline");
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    assert_eq!(
        revision(&app),
        Revision(before.0 + 1),
        "exactly one revision bump"
    );
    assert_eq!(undo_depth(&app), 1, "exactly one undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "breakline.partial"));
}

#[test]
fn breakline_refuses_coincident_points_without_change() {
    let (mut app, mut session) = app_with(Vec::new());
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::BreakLine,
        CommandPayload::Points(vec![p3(1.0, 1.0, 0.0), p3(1.0, 1.0, 0.0)]),
    )
    .expect_err("coincident points refuse");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert!(model_entities(&app).is_empty());
}

#[test]
fn blend_bridges_two_lines_with_one_cubic_spline() {
    let (mut app, mut session) = app_with(vec![
        line(1, p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)),
        line(2, p3(6.0, 2.0, 0.0), p3(10.0, 2.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::Blend,
        CommandPayload::None,
    )
    .expect("BLEND is admitted");
    assert!(outcome.changes.is_some());
    let entities = model_entities(&app);
    assert_eq!(entities.len(), 3, "two sources plus one spline");
    let spline = entities
        .iter()
        .find(|entity| matches!(entity.geometry, SemanticGeometry::Spline { .. }))
        .expect("BLEND creates a spline");
    match &spline.geometry {
        SemanticGeometry::Spline {
            degree,
            control_points,
            weights,
            ..
        } => {
            assert_eq!(*degree, 3);
            assert_eq!(control_points.len(), 4);
            assert_eq!(weights.len(), 4);
        }
        _ => unreachable!(),
    }
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "blend.partial"));
}

#[test]
fn blend_requires_exactly_two_open_curves() {
    let (mut app, mut session) = app_with(vec![line(1, p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0))]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::Blend,
        CommandPayload::None,
    )
    .expect_err("one curve is not enough");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn array_without_a_payload_reports_the_two_planners_and_writes_nothing() {
    let (mut app, mut session) = app_with(Vec::new());
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::Array,
        CommandPayload::None,
    )
    .expect("the router report is admitted");
    assert!(outcome.changes.is_none());
    let row = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "array.needs_payload")
        .expect("the router names the missing payload");
    assert!(row.message.contains("ARRAYRECT"));
    assert!(row.message.contains("ARRAYPOLAR"));
    assert_eq!(revision(&app), before);
}

// `MLEADERALIGN`/`MLEADEREDIT`/`MLEADERCOLLECT`/`TOLERANCE` graduated to real
// entity writes on 2026-11-12 (`app_drawing::mleader_edit`,
// `app_drawing::tolerance`); their contracts live in `tests::extended_w9_mleader`.
const REFUSALS: [(ExtendedCommandId, &str); 1] = [(
    ExtendedCommandId::DimArc,
    "dimarc.unsupported: the dimension domain has no arc-length kind",
)];

#[test]
fn wave9_draw_refusals_answer_with_their_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = execute(&mut app, &mut session, ext, CommandPayload::None)
            .expect_err("a refusal never succeeds");
        match error {
            CadError::Unsupported(reason) => assert_eq!(reason, expected),
            other => panic!("{ext:?} must be Unsupported, got {other:?}"),
        }
        assert_eq!(revision(&app), before);
        assert_eq!(undo_depth(&app), 0);
        assert!(!ext.requires_work_mode(), "{ext:?} stays Viewer-legal");
    }
}

#[test]
fn blend_and_breakline_are_work_only() {
    for ext in [ExtendedCommandId::Blend, ExtendedCommandId::BreakLine] {
        assert!(ext.requires_work_mode(), "{ext:?} writes entities");
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is a legal mode");
        let error = execute(&mut app, &mut session, ext, CommandPayload::None)
            .expect_err("Viewer cannot edit");
        assert!(matches!(error, CadError::PermissionDenied));
    }
}

// ---------------------------------------------------------------------------
// ARCTEXT
// ---------------------------------------------------------------------------

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn approx_point(a: Point3, b: Point3) -> bool {
    approx(a.x, b.x) && approx(a.y, b.y) && approx(a.z, b.z)
}

#[test]
fn arctext_places_one_text_per_character_at_equal_angular_steps() {
    let (mut app, mut session) = app_with(vec![
        arc(
            1,
            p3(0.0, 0.0, 0.0),
            p3(0.0, 0.0, 1.0),
            5.0,
            0.0,
            std::f64::consts::PI,
        ),
        text_at(2, "AB", p3(0.0, 6.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::ArcText,
        CommandPayload::None,
    )
    .expect("ARCTEXT is admitted");
    assert!(outcome.changes.is_some(), "ARCTEXT writes one transaction");

    let rows = model_entities(&app);
    assert_eq!(rows.len(), 3, "the arc plus one entity per character");
    let arc_kept = rows
        .iter()
        .any(|row| matches!(row.geometry, SemanticGeometry::Arc { .. }));
    assert!(arc_kept, "ARCTEXT keeps the arc it attached text to");
    let mut placed: Vec<(Point3, f64, String)> = Vec::new();
    for row in &rows {
        let SemanticGeometry::Text {
            text,
            position,
            height,
            rotation,
            ..
        } = &row.geometry
        else {
            continue;
        };
        assert_eq!(*height, 2.0, "the source height is kept");
        assert_eq!(row.layer, LayerId(3), "the text layer is kept");
        assert_eq!(row.object.type_key, "AcDbText");
        placed.push((*position, *rotation, text.clone()));
    }
    placed.sort_by(|a, b| a.2.cmp(&b.2));
    assert_eq!(
        placed.iter().map(|row| row.2.as_str()).collect::<Vec<_>>(),
        vec!["A", "B"],
        "one TEXT entity per character"
    );
    // Equal angular steps of sweep / 2 = 90 deg, first character centred in its
    // step, so the two positions sit at 45 and 135 degrees on the 5-radius arc.
    let quarter = std::f64::consts::FRAC_PI_4;
    let expected = [
        p3(5.0 * quarter.cos(), 5.0 * quarter.sin(), 0.0),
        p3(
            5.0 * (3.0 * quarter).cos(),
            5.0 * (3.0 * quarter).sin(),
            0.0,
        ),
    ];
    for (index, (position, rotation, label)) in placed.iter().enumerate() {
        assert!(
            approx_point(*position, expected[index]),
            "{label}: position {position:?} is not the expected angular step {:?}",
            expected[index]
        );
        // Tangential and upright: the reading direction never points into -X.
        assert!(
            rotation.cos() >= -1e-9,
            "{label}: rotation {rotation} would read upside down"
        );
        assert!(
            approx(rotation.sin().abs(), quarter.sin()),
            "{label}: rotation {rotation} is not the arc tangent (45 degrees)"
        );
    }
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "arctext.partial"));

    // One undo restores the text source and removes the placed characters.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    let rows = model_entities(&app);
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|row| matches!(row.geometry, SemanticGeometry::Text { .. })));
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn arctext_refusals_are_named_and_write_nothing() {
    let cases: Vec<(&str, Vec<DbEntity>, Vec<u128>)> = vec![
        ("arctext.selection", Vec::new(), Vec::new()),
        (
            "arctext.no_arc",
            vec![text_at(2, "AB", p3(0.0, 6.0, 0.0))],
            vec![2],
        ),
        (
            "arctext.no_text",
            vec![arc(
                1,
                p3(0.0, 0.0, 0.0),
                p3(0.0, 0.0, 1.0),
                5.0,
                0.0,
                std::f64::consts::PI,
            )],
            vec![1],
        ),
        (
            "arctext.multiple_arcs",
            vec![
                arc(1, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0, 0.0, 1.0),
                arc(2, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 6.0, 0.0, 1.0),
                text_at(3, "AB", p3(0.0, 6.0, 0.0)),
            ],
            vec![1, 2, 3],
        ),
        (
            "arctext.unsupported_selection",
            vec![
                arc(1, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0, 0.0, 1.0),
                text_at(2, "AB", p3(0.0, 6.0, 0.0)),
                line(3, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
            ],
            vec![1, 2, 3],
        ),
        (
            "arctext.plane",
            vec![
                arc(1, p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0), 5.0, 0.0, 1.0),
                text_at(2, "AB", p3(0.0, 6.0, 0.0)),
            ],
            vec![1, 2],
        ),
        (
            "arctext.multiline",
            vec![
                arc(1, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0, 0.0, 1.0),
                text_at(2, "A\\PB", p3(0.0, 6.0, 0.0)),
            ],
            vec![1, 2],
        ),
        (
            "arctext.text_empty",
            vec![
                arc(1, p3(0.0, 0.0, 0.0), p3(0.0, 0.0, 1.0), 5.0, 0.0, 1.0),
                text_at(2, "   ", p3(0.0, 6.0, 0.0)),
            ],
            vec![1, 2],
        ),
    ];
    for (code, rows, selection) in cases {
        let (mut app, mut session) = app_with(rows);
        select(&mut session, &selection);
        let before = revision(&app);
        let error = execute(
            &mut app,
            &mut session,
            ExtendedCommandId::ArcText,
            CommandPayload::None,
        )
        .expect_err("every refusal fails the command");
        let message = match error {
            CadError::Unsupported(message) | CadError::InvalidInput(message) => message,
            other => panic!("{code}: expected a named refusal, got {other:?}"),
        };
        assert!(message.contains(code), "{code}: {message}");
        assert_eq!(revision(&app), before, "{code} writes nothing");
        assert_eq!(undo_depth(&app), 0, "{code} records no undo");
    }
}

#[test]
fn arctext_is_work_only() {
    // The catalogue flag still calls ARCTEXT Viewer-legal (it used to be a named
    // refusal); the handler owns the refusal because the command now writes.
    let (mut app, mut session) = application_with_document();
    session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is a legal mode");
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::ArcText,
        CommandPayload::None,
    )
    .expect_err("Viewer cannot place text");
    assert!(matches!(error, CadError::PermissionDenied));
    assert_eq!(undo_depth(&app), 0);
}
