//! 2026-11-12 contracts for the five former named refusals:
//! `MLEDIT`/`-MLEDIT`, `MLEADERALIGN`, `MLEADEREDIT`, `MLEADERCOLLECT` and
//! `TOLERANCE`.
//!
//! A real edit must create/rewrite exactly the declared entities and commit
//! exactly one transaction / one undo step. A refusal must write nothing, bump
//! no revision and record no history.

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

/// One straight-line MULTILEADER with text content: one root, one line with an
/// arrow vertex and a dogleg vertex, and the text at `text_position`.
fn leader(id: u128, text: &str, text_position: Point3, arrow: Point3, dogleg: Point3) -> DbEntity {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 1.0,
            landing_gap: 0.1,
            arrowhead_size: 0.5,
            text_height: 0.2,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Text,
        text: MLeaderText {
            string: text.into(),
            location: text_position,
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: text_position,
            dogleg_vector: p3(1.0, 0.0, 0.0),
            dogleg_length: 1.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![arrow, dogleg],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbMultiLeader".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::MultiLeader(Box::new(geometry)),
        draw_order: id as i64,
    }
}

/// A drawing with layers 0/WALLS and the given entities.
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

fn segment(geometry: &SemanticGeometry) -> (Point3, Point3) {
    match geometry {
        SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("expected a LINE, got {other:?}"),
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn has_segment(entities: &[DbEntity], start: Point3, end: Point3) -> bool {
    entities.iter().any(|entity| {
        let (s, e) = segment(&entity.geometry);
        approx(s.x, start.x) && approx(s.y, start.y) && approx(e.x, end.x) && approx(e.y, end.y)
    })
}

fn leader_of(entity: &DbEntity) -> &MultiLeaderGeometry {
    match &entity.geometry {
        SemanticGeometry::MultiLeader(leader) => leader,
        other => panic!("expected a MULTILEADER, got {other:?}"),
    }
}

fn mledit_payload(mode: MlEditMode, first: Point3, second: Option<Point3>) -> CommandPayload {
    CommandPayload::MlEdit(Box::new(MlEditCommand {
        mode,
        first,
        second,
    }))
}

// ---------------------------------------------------------------------------
// MLEDIT / -MLEDIT
// ---------------------------------------------------------------------------

#[test]
fn mledit_close_trims_and_extends_two_lines_to_their_intersection() {
    let (mut app, mut session) = app_with(vec![
        line(1, p3(0.0, 0.0, 0.0), p3(5.0, 0.0, 0.0)),
        line(2, p3(2.0, -2.0, 0.0), p3(2.0, -1.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MlEdit,
        mledit_payload(MlEditMode::Closed, p3(2.0, 0.0, 0.0), None),
    )
    .expect("MLEDIT CLOSED is admitted");
    assert!(outcome.changes.is_some(), "MLEDIT writes one transaction");
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mledit.applied"));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mledit.residual"));
    let entities = model_entities(&app);
    assert_eq!(entities.len(), 2);
    // Line 1 keeps its far end and its nearest end moved onto (2, 0).
    assert!(has_segment(&entities, p3(2.0, 0.0, 0.0), p3(5.0, 0.0, 0.0)));
    // Line 2 was extended to the intersection.
    assert!(has_segment(
        &entities,
        p3(2.0, -2.0, 0.0),
        p3(2.0, 0.0, 0.0)
    ));
}

#[test]
fn mledit_close_is_a_named_no_op_when_the_lines_already_meet() {
    let (mut app, mut session) = app_with(vec![
        line(1, p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 0.0)),
        line(2, p3(2.0, -1.0, 0.0), p3(2.0, 0.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MlEdit,
        mledit_payload(MlEditMode::Merge, p3(2.0, 0.0, 0.0), None),
    )
    .expect("the no-op report is admitted");
    assert!(outcome.changes.is_none(), "no transaction for a no-op");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mledit.unchanged"));
}

#[test]
fn mledit_cleanup_refuses_parallel_lines() {
    let (mut app, mut session) = app_with(vec![
        line(1, p3(0.0, 0.0, 0.0), p3(5.0, 0.0, 0.0)),
        line(2, p3(0.0, 1.0, 0.0), p3(5.0, 1.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MlEdit,
        mledit_payload(MlEditMode::Cross, p3(2.0, 0.0, 0.0), None),
    )
    .expect_err("parallel lines have no intersection");
    assert!(error.to_string().contains("mledit.parallel"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn mledit_open_splits_both_lines_at_the_crossing() {
    let (mut app, mut session) = app_with(vec![
        line(1, p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)),
        line(2, p3(2.0, -2.0, 0.0), p3(2.0, 2.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashMlEdit,
        mledit_payload(MlEditMode::Open, p3(2.0, 0.0, 0.0), None),
    )
    .expect("-MLEDIT OPEN is admitted");
    assert_eq!(outcome.objects.len(), 4);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    let entities = model_entities(&app);
    assert_eq!(entities.len(), 4);
    for (start, end) in [
        (p3(0.0, 0.0, 0.0), p3(2.0, 0.0, 0.0)),
        (p3(2.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)),
        (p3(2.0, -2.0, 0.0), p3(2.0, 0.0, 0.0)),
        (p3(2.0, 0.0, 0.0), p3(2.0, 2.0, 0.0)),
    ] {
        assert!(
            has_segment(&entities, start, end),
            "missing {start:?}->{end:?}"
        );
    }
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mledit.residual"));
}

#[test]
fn mledit_break_removes_the_band_from_two_parallel_lines() {
    let (mut app, mut session) = app_with(vec![
        line(1, p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)),
        line(2, p3(0.0, 2.0, 0.0), p3(10.0, 2.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MlEdit,
        mledit_payload(
            MlEditMode::Break,
            p3(4.0, 0.0, 0.0),
            Some(p3(6.0, 0.0, 0.0)),
        ),
    )
    .expect("MLEDIT BREAK is admitted");
    assert_eq!(outcome.objects.len(), 4);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    let entities = model_entities(&app);
    assert_eq!(entities.len(), 4);
    for (start, end) in [
        (p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0)),
        (p3(6.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)),
        (p3(0.0, 2.0, 0.0), p3(4.0, 2.0, 0.0)),
        (p3(6.0, 2.0, 0.0), p3(10.0, 2.0, 0.0)),
    ] {
        assert!(
            has_segment(&entities, start, end),
            "missing {start:?}->{end:?}"
        );
    }
}

#[test]
fn mledit_break_refuses_non_parallel_lines_and_a_band_outside_them() {
    let (mut app, mut session) = app_with(vec![
        line(1, p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)),
        line(2, p3(0.0, 2.0, 0.0), p3(10.0, 4.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MlEdit,
        mledit_payload(
            MlEditMode::Break,
            p3(4.0, 0.0, 0.0),
            Some(p3(6.0, 0.0, 0.0)),
        ),
    )
    .expect_err("non-parallel lines have no common band");
    assert!(
        error.to_string().contains("mledit.break_parallel"),
        "{error}"
    );

    let (mut app, mut session) = app_with(vec![
        line(1, p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)),
        line(2, p3(0.0, 2.0, 0.0), p3(10.0, 2.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MlEdit,
        mledit_payload(
            MlEditMode::Break,
            p3(12.0, 0.0, 0.0),
            Some(p3(14.0, 0.0, 0.0)),
        ),
    )
    .expect_err("the band is outside both lines");
    assert!(error.to_string().contains("mledit.break_range"), "{error}");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn mledit_refuses_a_selection_that_is_not_exactly_two_lines() {
    let (mut app, mut session) = app_with(vec![line(1, p3(0.0, 0.0, 0.0), p3(5.0, 0.0, 0.0))]);
    select(&mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MlEdit,
        mledit_payload(MlEditMode::Tee, p3(2.0, 0.0, 0.0), None),
    )
    .expect_err("one line is not enough");
    assert!(error.to_string().contains("mledit.selection"), "{error}");
}

#[test]
fn mledit_refuses_a_stored_acdbmline_compound() {
    let mut compound = line(9, p3(0.0, 0.0, 0.0), p3(5.0, 0.0, 0.0));
    compound.object.type_key = "AcDbMline".into();
    compound.geometry = SemanticGeometry::Compound(vec![SemanticGeometry::Line {
        start: p3(0.0, 0.5, 0.0),
        end: p3(5.0, 0.5, 0.0),
    }]);
    let (mut app, mut session) = app_with(vec![
        compound,
        line(2, p3(2.0, -2.0, 0.0), p3(2.0, 2.0, 0.0)),
    ]);
    select(&mut session, &[9, 2]);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MlEdit,
        mledit_payload(MlEditMode::Cross, p3(2.0, 0.0, 0.0), None),
    )
    .expect_err("the AcDbMline compound has no editable model");
    assert!(
        error.to_string().contains("mledit.acdbmline_model"),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// MLEADERALIGN
// ---------------------------------------------------------------------------

fn align_payload(mode: MLeaderAlignMode, spacing: Option<f64>) -> CommandPayload {
    CommandPayload::MLeaderAlign(Box::new(MLeaderAlignCommand { mode, spacing }))
}

#[test]
fn mleaderalign_left_aligns_the_texts_and_keeps_the_arrows() {
    let (mut app, mut session) = app_with(vec![
        leader(
            1,
            "A",
            p3(10.0, 5.0, 0.0),
            p3(0.0, 5.0, 0.0),
            p3(9.0, 5.0, 0.0),
        ),
        leader(
            2,
            "B",
            p3(20.0, 0.0, 0.0),
            p3(12.0, 0.0, 0.0),
            p3(19.0, 0.0, 0.0),
        ),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MLeaderAlign,
        align_payload(MLeaderAlignMode::Left, None),
    )
    .expect("MLEADERALIGN is admitted");
    // The leader already on the left column is not rewritten; only the one that
    // moved is staged (one transaction either way).
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mleaderalign.applied"));
    let entities = model_entities(&app);
    let first = leader_of(&entities[0]);
    let second = leader_of(&entities[1]);
    assert!(approx(first.text().location.x, 10.0));
    assert!(approx(first.text().location.y, 5.0));
    assert!(approx(second.text().location.x, 10.0));
    assert!(approx(second.text().location.y, 0.0));
    // The arrow (first leader vertex) is anchored; the rest followed the text.
    assert!(approx(first.lines()[0].points[0].x, 0.0));
    assert!(approx(second.lines()[0].points[0].x, 12.0));
    assert!(approx(second.lines()[0].points[1].x, 9.0));
}

#[test]
fn mleaderalign_centers_and_spaces_the_texts_top_to_bottom() {
    let (mut app, mut session) = app_with(vec![
        leader(
            1,
            "A",
            p3(10.0, 10.0, 0.0),
            p3(0.0, 10.0, 0.0),
            p3(9.0, 10.0, 0.0),
        ),
        leader(
            2,
            "B",
            p3(5.0, 2.0, 0.0),
            p3(0.0, 2.0, 0.0),
            p3(4.0, 2.0, 0.0),
        ),
    ]);
    select(&mut session, &[1, 2]);
    execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MLeaderAlign,
        align_payload(MLeaderAlignMode::Center, Some(4.0)),
    )
    .expect("MLEADERALIGN CENTER 4 is admitted");
    let entities = model_entities(&app);
    assert!(approx(leader_of(&entities[0]).text().location.x, 7.5));
    assert!(approx(leader_of(&entities[0]).text().location.y, 10.0));
    assert!(approx(leader_of(&entities[1]).text().location.x, 7.5));
    assert!(approx(leader_of(&entities[1]).text().location.y, 6.0));
}

#[test]
fn mleaderalign_refuses_a_single_leader_and_a_non_mleader_selection() {
    let (mut app, mut session) = app_with(vec![leader(
        1,
        "A",
        p3(10.0, 5.0, 0.0),
        p3(0.0, 5.0, 0.0),
        p3(9.0, 5.0, 0.0),
    )]);
    select(&mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MLeaderAlign,
        align_payload(MLeaderAlignMode::Left, None),
    )
    .expect_err("one leader cannot be aligned");
    assert!(
        error.to_string().contains("mleaderalign.selection"),
        "{error}"
    );

    let (mut app, mut session) = app_with(vec![
        leader(
            1,
            "A",
            p3(10.0, 5.0, 0.0),
            p3(0.0, 5.0, 0.0),
            p3(9.0, 5.0, 0.0),
        ),
        line(2, p3(0.0, 0.0, 0.0), p3(5.0, 0.0, 0.0)),
    ]);
    select(&mut session, &[1, 2]);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MLeaderAlign,
        align_payload(MLeaderAlignMode::Left, None),
    )
    .expect_err("a LINE is not a MULTILEADER");
    assert!(
        error.to_string().contains("mleaderalign.not_mleader"),
        "{error}"
    );
    assert_eq!(undo_depth(&app), 0);
}

// ---------------------------------------------------------------------------
// MLEADEREDIT
// ---------------------------------------------------------------------------

fn edit_payload(action: MLeaderEditAction) -> CommandPayload {
    CommandPayload::MLeaderEdit(Box::new(MLeaderEditCommand { action }))
}

#[test]
fn mleaderedit_adds_a_leader_line_at_the_typed_arrow() {
    let (mut app, mut session) = app_with(vec![leader(
        1,
        "A",
        p3(10.0, 5.0, 0.0),
        p3(0.0, 5.0, 0.0),
        p3(9.0, 5.0, 0.0),
    )]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MLeaderEdit,
        edit_payload(MLeaderEditAction::Add {
            arrow: p3(0.0, 15.0, 0.0),
        }),
    )
    .expect("MLEADEREDIT ADD is admitted");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    let entities = model_entities(&app);
    let leader = leader_of(&entities[0]);
    assert_eq!(leader.lines().len(), 2);
    let added = &leader.lines()[1];
    assert_eq!(added.root_index, 0);
    assert!(approx(added.points[0].y, 15.0));
    // The new line ends on the root's existing dogleg vertex.
    assert!(approx(added.points[1].x, 9.0));
    assert!(approx(added.points[1].y, 5.0));
}

#[test]
fn mleaderedit_removes_one_line_and_refuses_to_empty_a_root() {
    let (mut app, mut session) = app_with(vec![leader(
        1,
        "A",
        p3(10.0, 5.0, 0.0),
        p3(0.0, 5.0, 0.0),
        p3(9.0, 5.0, 0.0),
    )]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MLeaderEdit,
        edit_payload(MLeaderEditAction::Add {
            arrow: p3(0.0, 15.0, 0.0),
        }),
    )
    .expect("add one line");
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MLeaderEdit,
        edit_payload(MLeaderEditAction::Remove { line_index: 1 }),
    )
    .expect("remove the added line");
    assert_eq!(outcome.objects.len(), 1);
    let entities = model_entities(&app);
    assert_eq!(leader_of(&entities[0]).lines().len(), 1);

    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MLeaderEdit,
        edit_payload(MLeaderEditAction::Remove { line_index: 0 }),
    )
    .expect_err("the last line cannot be removed");
    assert!(
        error.to_string().contains("mleaderedit.last_line"),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// MLEADERCOLLECT
// ---------------------------------------------------------------------------

#[test]
fn mleadercollect_merges_two_leaders_into_one_annotation() {
    let (mut app, mut session) = app_with(vec![
        leader(
            1,
            "A",
            p3(10.0, 5.0, 0.0),
            p3(0.0, 5.0, 0.0),
            p3(9.0, 5.0, 0.0),
        ),
        leader(
            2,
            "B",
            p3(20.0, 0.0, 0.0),
            p3(12.0, 0.0, 0.0),
            p3(19.0, 0.0, 0.0),
        ),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MLeaderCollect,
        CommandPayload::None,
    )
    .expect("MLEADERCOLLECT is admitted");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    let entities = model_entities(&app);
    assert_eq!(entities.len(), 1, "the two sources are consumed");
    let leader = leader_of(&entities[0]);
    assert_eq!(leader.text().string, "A\\PB");
    assert_eq!(leader.roots().len(), 2);
    assert_eq!(leader.lines().len(), 2);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mleadercollect.representation"));
}

#[test]
fn mleadercollect_refuses_a_single_selection() {
    let (mut app, mut session) = app_with(vec![leader(
        1,
        "A",
        p3(10.0, 5.0, 0.0),
        p3(0.0, 5.0, 0.0),
        p3(9.0, 5.0, 0.0),
    )]);
    select(&mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::MLeaderCollect,
        CommandPayload::None,
    )
    .expect_err("one leader cannot be collected");
    assert!(
        error.to_string().contains("mleadercollect.selection"),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// TOLERANCE
// ---------------------------------------------------------------------------

fn tolerance_payload(origin: Point3, text: &str) -> CommandPayload {
    CommandPayload::Tolerance(Box::new(ToleranceCommand {
        origin,
        text: text.into(),
    }))
}

#[test]
fn tolerance_creates_a_frame_with_dividers_and_cell_texts() {
    let (mut app, mut session) = app_with(Vec::new());
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::Tolerance,
        tolerance_payload(p3(1.0, 2.0, 0.0), "0.05|A"),
    )
    .expect("TOLERANCE is admitted");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(outcome.objects.len(), 4, "border + divider + two TEXT rows");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "tolerance.representation"));
    let entities = model_entities(&app);
    let border = entities
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Polyline {
                points,
                closed,
                bulges,
                ..
            } => Some((points.clone(), *closed, bulges.clone())),
            _ => None,
        })
        .expect("a border polyline");
    assert!(border.1, "the border is a closed polyline");
    assert_eq!(border.0.len(), 4);
    assert!(border.2.iter().all(|bulge| *bulge == 0.0));
    assert!(approx(border.0[0].x, 1.0));
    assert!(approx(border.0[0].y, 2.0));
    let divider = entities
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Line { start, end } => Some((*start, *end)),
            _ => None,
        })
        .expect("one internal divider line");
    assert!(approx(divider.0.y, 2.0));
    assert!(approx(divider.1.y, divider.0.y + 2.5 * 1.6));
    let mut texts: Vec<&str> = entities
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    texts.sort_unstable();
    assert_eq!(texts, vec!["0.05", "A"]);
}

#[test]
fn tolerance_refuses_empty_or_invalid_frame_text() {
    let (mut app, mut session) = app_with(Vec::new());
    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::Tolerance,
        tolerance_payload(p3(0.0, 0.0, 0.0), "   "),
    )
    .expect_err("empty frame text is refused");
    assert!(error.to_string().contains("tolerance.text"), "{error}");

    let error = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::Tolerance,
        tolerance_payload(p3(0.0, 0.0, 0.0), "A||B"),
    )
    .expect_err("an empty compartment is refused");
    assert!(
        error.to_string().contains("tolerance.cell_empty"),
        "{error}"
    );
    assert_eq!(undo_depth(&app), 0);
}
