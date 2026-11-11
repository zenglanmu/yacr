//! `TEXTALIGN` contract tests for **all six** edges/options this build models:
//! `LEFT`/`RIGHT`/`CENTER` (horizontal) and `TOP`/`MIDDLE`/`BOTTOM` (vertical).
//!
//! Every case pins the declared estimated extents (width = height × character
//! count, height = height, anchored by the stored alignment, rotation ignored),
//! the single-axis rule (only the affected coordinate moves), one transaction /
//! one undo step and the named refusals.
//!
//! AutoCAD's remaining `TEXTALIGN` options are **not** modelled and are refused by
//! the typed grammar, never faked here: `DISTRIBUTION` (horizontal/vertical even
//! spacing) and `SPACING` need another `TextAlignEdge` variant, and `OBLIQUE`
//! needs a per-object skew the text model does not store. The grammar admits only
//! the six edge words (`invalid_textalign_argument_reason` names them).

use super::*;

/// Three left/baseline-anchored texts with the estimated boxes
/// `[0,4]×[0,2]`, `[10,18]×[10,12]` and `[20,26]×[20,22]`.
fn three_texts() -> (Application, SessionState) {
    let (app, mut session) = app_with(vec![
        simple_text(1, "ab", p3(0.0, 0.0, 0.0), 0),
        simple_text(2, "abcd", p3(10.0, 10.0, 0.0), 1),
        simple_text(3, "abc", p3(20.0, 20.0, 0.0), 2),
    ]);
    select(&mut session, &[1, 2, 3]);
    (app, session)
}

fn position_of(app: &Application, id: u128) -> Point3 {
    match &entity(app, id).expect("entity exists").geometry {
        SemanticGeometry::Text { position, .. } => *position,
        other => panic!("expected Text, got {other:?}"),
    }
}

fn assert_position(app: &Application, id: u128, x: f64, y: f64) {
    let position = position_of(app, id);
    assert!(
        near(position.x, x) && near(position.y, y),
        "entity {id} is at ({}, {}), expected ({x}, {y})",
        position.x,
        position.y
    );
}

/// Run one edge over the shared three-text fixture and return the app.
fn aligned(edge: TextAlignEdge) -> (Application, SessionState) {
    let (mut app, mut session) = three_texts();
    execute(
        &mut app,
        &mut session,
        CommandId::TextAlignEdges,
        CommandPayload::TextAlignEdges(TextAlignCommand { edge }),
    )
    .expect("TEXTALIGN");
    (app, session)
}

#[test]
fn textalign_left_right_and_center_align_the_one_common_horizontal_line() {
    // Union box: x ∈ [0, 26], y ∈ [0, 22]. Only X may move.
    let (app, _) = aligned(TextAlignEdge::Left);
    assert_position(&app, 1, 0.0, 0.0);
    assert_position(&app, 2, 0.0, 10.0);
    assert_position(&app, 3, 0.0, 20.0);

    let (app, _) = aligned(TextAlignEdge::Right);
    // The right edge moves to 26: a 4-wide box anchored on its left starts at 22.
    assert_position(&app, 1, 22.0, 0.0);
    assert_position(&app, 2, 18.0, 10.0);
    assert_position(&app, 3, 20.0, 20.0);

    let (app, _) = aligned(TextAlignEdge::Center);
    // Every estimated box centre is on x = (0 + 26) / 2 = 13.
    assert_position(&app, 1, 11.0, 0.0);
    assert_position(&app, 2, 9.0, 10.0);
    assert_position(&app, 3, 10.0, 20.0);
}

#[test]
fn textalign_top_middle_and_bottom_align_the_one_common_vertical_line() {
    let (app, _) = aligned(TextAlignEdge::Top);
    assert_position(&app, 1, 0.0, 20.0);
    assert_position(&app, 2, 10.0, 20.0);
    assert_position(&app, 3, 20.0, 20.0);

    let (app, _) = aligned(TextAlignEdge::Middle);
    // Every estimated box centre is on y = (0 + 22) / 2 = 11; X never moves.
    assert_position(&app, 1, 0.0, 10.0);
    assert_position(&app, 2, 10.0, 10.0);
    assert_position(&app, 3, 20.0, 10.0);

    let (app, _) = aligned(TextAlignEdge::Bottom);
    assert_position(&app, 1, 0.0, 0.0);
    assert_position(&app, 2, 10.0, 0.0);
    assert_position(&app, 3, 20.0, 0.0);
}

#[test]
fn textalign_is_one_transaction_and_one_undo_step_for_every_edge() {
    for edge in [
        TextAlignEdge::Left,
        TextAlignEdge::Right,
        TextAlignEdge::Center,
        TextAlignEdge::Top,
        TextAlignEdge::Middle,
        TextAlignEdge::Bottom,
    ] {
        let (mut app, mut session) = three_texts();
        let before = revision(&app);
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::TextAlignEdges,
            CommandPayload::TextAlignEdges(TextAlignCommand { edge }),
        )
        .expect("TEXTALIGN");
        assert_eq!(revision(&app).0, before.0 + 1, "{edge:?}: one transaction");
        assert_eq!(undo_depth(&app), 1, "{edge:?}: one undo step");
        // Every moved entity is reported once (an already-aligned extreme is not
        // a write and not an object).
        assert!(!outcome.objects.is_empty(), "{edge:?} moved nothing");
        execute(
            &mut app,
            &mut session,
            CommandId::Undo,
            CommandPayload::None,
        )
        .expect("undo");
        assert_position(&app, 1, 0.0, 0.0);
        assert_position(&app, 2, 10.0, 10.0);
        assert_position(&app, 3, 20.0, 20.0);
        assert_eq!(undo_depth(&app), 0);
    }
}

#[test]
fn textalign_reports_unchanged_without_a_transaction_when_the_edges_already_match() {
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "ab", p3(0.0, 0.0, 0.0), 0),
        simple_text(2, "abc", p3(0.0, 40.0, 0.0), 1),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextAlignEdges,
        CommandPayload::TextAlignEdges(TextAlignCommand {
            edge: TextAlignEdge::Left,
        }),
    )
    .expect("already aligned");
    assert!(outcome.changes.is_none(), "no ChangeSet");
    assert!(outcome.objects.is_empty(), "no object was rewritten");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "textalign.unchanged"),
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn textalign_refuses_a_degenerate_height_and_a_non_text_target() {
    // A stored height of 0 has no estimated box.
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "ab", p3(0.0, 0.0, 0.0), 0),
        text_entity(
            2,
            "cd",
            p3(10.0, 0.0, 0.0),
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            1,
            "AcDbText",
        ),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextAlignEdges,
        CommandPayload::TextAlignEdges(TextAlignCommand {
            edge: TextAlignEdge::Left,
        }),
    )
    .expect_err("a degenerate height is refused");
    assert_invalid(error, "textalign.degenerate");
    assert_eq!(revision(&app), before, "the refusal writes nothing");
    assert_eq!(undo_depth(&app), 0);

    // A mixed selection refuses the whole command (never a partial alignment).
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "ab", p3(0.0, 0.0, 0.0), 0),
        line_entity(2),
    ]);
    select(&mut session, &[1, 2]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextAlignEdges,
        CommandPayload::TextAlignEdges(TextAlignCommand {
            edge: TextAlignEdge::Top,
        }),
    )
    .expect_err("a non-text target refuses the command");
    assert_unsupported(error, "textalign.not_text");
    assert_eq!(undo_depth(&app), 0);
    assert_position(&app, 1, 0.0, 0.0);
}
