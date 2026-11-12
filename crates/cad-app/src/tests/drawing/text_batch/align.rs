//! `TEXTALIGN` contract tests for **all fifteen** alignment modes this build
//! models: `LEFT`/`RIGHT`/`CENTER` (horizontal), `TOP`/`MIDDLE`/`BOTTOM`
//! (vertical) and the nine combined AutoCAD 2026 modes (`TOPLEFT` … `BOTTOMRIGHT`),
//! plus `DISTRIBUTE [X|Y]` gap distribution and the three named spacing refusals.
//!
//! Every case pins the declared estimated extents (width = height × character
//! count, height = height, anchored by the stored alignment, rotation ignored),
//! the union-box reference frame, one transaction / one undo step and the named
//! refusals.
//!
//! AutoCAD's interactive base-object + second-point alignment vector is not
//! modelled: the union box is the deterministic non-interactive substitute.
//! `SETSPACING`/`CURRENTVERTICAL`/`CURRENTHORIZONTAL` are parsed and refused by
//! name (never faked); `OBLIQUE` is not in the grammar at all.

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
fn textalign_is_one_transaction_and_one_undo_step_for_every_mode() {
    for edge in [
        TextAlignEdge::Left,
        TextAlignEdge::Right,
        TextAlignEdge::Center,
        TextAlignEdge::Top,
        TextAlignEdge::Middle,
        TextAlignEdge::Bottom,
        TextAlignEdge::TopLeft,
        TextAlignEdge::TopCenter,
        TextAlignEdge::TopRight,
        TextAlignEdge::MiddleLeft,
        TextAlignEdge::MiddleCenter,
        TextAlignEdge::MiddleRight,
        TextAlignEdge::BottomLeft,
        TextAlignEdge::BottomCenter,
        TextAlignEdge::BottomRight,
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

/// The combined nine modes over the shared fixture: the union box is
/// `x ∈ [0, 26]`, `y ∈ [0, 22]`, so left/right/centre are `0/26/13` and
/// bottom/middle/top are `0/11/22` on the reference frame.
#[test]
fn textalign_combined_modes_align_both_axes_to_the_union_box() {
    let expected: &[(TextAlignEdge, [(f64, f64); 3])] = &[
        (
            TextAlignEdge::TopLeft,
            [(0.0, 20.0), (0.0, 20.0), (0.0, 20.0)],
        ),
        (
            TextAlignEdge::TopCenter,
            [(11.0, 20.0), (9.0, 20.0), (10.0, 20.0)],
        ),
        (
            TextAlignEdge::TopRight,
            [(22.0, 20.0), (18.0, 20.0), (20.0, 20.0)],
        ),
        (
            TextAlignEdge::MiddleLeft,
            [(0.0, 10.0), (0.0, 10.0), (0.0, 10.0)],
        ),
        (
            TextAlignEdge::MiddleCenter,
            [(11.0, 10.0), (9.0, 10.0), (10.0, 10.0)],
        ),
        (
            TextAlignEdge::MiddleRight,
            [(22.0, 10.0), (18.0, 10.0), (20.0, 10.0)],
        ),
        (
            TextAlignEdge::BottomLeft,
            [(0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
        ),
        (
            TextAlignEdge::BottomCenter,
            [(11.0, 0.0), (9.0, 0.0), (10.0, 0.0)],
        ),
        (
            TextAlignEdge::BottomRight,
            [(22.0, 0.0), (18.0, 0.0), (20.0, 0.0)],
        ),
    ];
    for (edge, positions) in expected {
        let (app, _) = aligned(*edge);
        for (id, (x, y)) in [1u128, 2, 3].iter().zip(positions) {
            assert_position(&app, *id, *x, *y);
        }
    }
}

/// Moving reference points must never touch content, height, rotation or the
/// stored alignment (AutoCAD's "moves reference points only" rule).
#[test]
fn textalign_never_changes_text_height_rotation_or_justification() {
    let (app, _) = aligned(TextAlignEdge::MiddleCenter);
    for id in [1u128, 2, 3] {
        let SemanticGeometry::Text {
            text,
            height,
            rotation,
            h_align,
            v_align,
            ..
        } = &entity(&app, id).expect("entity exists").geometry
        else {
            panic!("expected Text");
        };
        assert_eq!(
            (text.as_str(), *height, *rotation, *h_align, *v_align),
            (
                match id {
                    1 => "ab",
                    2 => "abcd",
                    _ => "abc",
                },
                2.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
            ),
            "entity {id} kept its style"
        );
    }
}

/// Four texts of different heights, already at equal 4-unit gaps on Y:
/// tops 20/15/9/1 and bottoms 19/13/5/0.
fn four_distributed_texts() -> (Application, SessionState) {
    let (app, mut session) = app_with(vec![
        text_entity(
            1,
            "aa",
            p3(0.0, 19.0, 0.0),
            1.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            0,
            "AcDbText",
        ),
        text_entity(
            2,
            "ab",
            p3(10.0, 13.0, 0.0),
            2.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            1,
            "AcDbText",
        ),
        text_entity(
            3,
            "abcd",
            p3(5.0, 5.0, 0.0),
            4.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            2,
            "AcDbText",
        ),
        text_entity(
            4,
            "a",
            p3(0.0, 0.0, 0.0),
            1.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            3,
            "AcDbText",
        ),
    ]);
    select(&mut session, &[1, 2, 3, 4]);
    (app, session)
}

#[test]
fn textalign_distribute_y_equalises_the_gaps_and_keeps_the_ends_fixed() {
    // Tops 20/12/6/1, heights 1/2/4/1: free span 12 over three gaps ⇒ gap 4.
    let (mut app, mut session) = app_with(vec![
        text_entity(
            1,
            "aa",
            p3(0.0, 19.0, 0.0),
            1.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            0,
            "AcDbText",
        ),
        text_entity(
            2,
            "ab",
            p3(10.0, 10.0, 0.0),
            2.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            1,
            "AcDbText",
        ),
        text_entity(
            3,
            "abcd",
            p3(5.0, 2.0, 0.0),
            4.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            2,
            "AcDbText",
        ),
        text_entity(
            4,
            "a",
            p3(0.0, 0.0, 0.0),
            1.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            3,
            "AcDbText",
        ),
    ]);
    select(&mut session, &[1, 2, 3, 4]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextAlignEdges,
        CommandPayload::TextAlignSpacing(TextAlignSpacingCommand {
            spacing: TextAlignSpacing::Distribute(TextAlignAxis::Y),
        }),
    )
    .expect("TEXTALIGN DISTRIBUTE Y");
    // First/last fixed; the two interiors land so every gap is 4 units.
    assert_position(&app, 1, 0.0, 19.0);
    assert_position(&app, 2, 10.0, 13.0);
    assert_position(&app, 3, 5.0, 5.0);
    assert_position(&app, 4, 0.0, 0.0);
    // X never moves in a Y distribution.
    assert_eq!(outcome.objects.len(), 2, "only the two interiors moved");
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert_position(&app, 2, 10.0, 10.0);
    assert_position(&app, 3, 5.0, 2.0);
}

#[test]
fn textalign_distribute_x_equalises_width_gaps_in_reading_order() {
    // Widths 4/2/6/1 at left edges 0/10/14/21: free span 9 ⇒ gap 3.
    let (mut app, mut session) = app_with(vec![
        text_entity(
            1,
            "abcd",
            p3(0.0, 0.0, 0.0),
            1.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            0,
            "AcDbText",
        ),
        text_entity(
            2,
            "ab",
            p3(10.0, 1.0, 0.0),
            1.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            1,
            "AcDbText",
        ),
        text_entity(
            3,
            "abcdef",
            p3(14.0, 2.0, 0.0),
            1.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            2,
            "AcDbText",
        ),
        text_entity(
            4,
            "a",
            p3(21.0, 3.0, 0.0),
            1.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            3,
            "AcDbText",
        ),
    ]);
    select(&mut session, &[1, 2, 3, 4]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextAlignEdges,
        CommandPayload::TextAlignSpacing(TextAlignSpacingCommand {
            spacing: TextAlignSpacing::Distribute(TextAlignAxis::X),
        }),
    )
    .expect("TEXTALIGN DISTRIBUTE X");
    assert_position(&app, 1, 0.0, 0.0);
    assert_position(&app, 2, 7.0, 1.0);
    assert_position(&app, 3, 12.0, 2.0);
    assert_position(&app, 4, 21.0, 3.0);
    assert_eq!(outcome.objects.len(), 2, "only the two interiors moved");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn textalign_distribute_refuses_two_targets_and_reports_already_even_as_unchanged() {
    // Two targets: no interior text exists.
    let (mut app, mut session) = app_with(vec![
        simple_text(1, "a", p3(0.0, 0.0, 0.0), 0),
        simple_text(2, "a", p3(0.0, 10.0, 0.0), 1),
    ]);
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextAlignEdges,
        CommandPayload::TextAlignSpacing(TextAlignSpacingCommand {
            spacing: TextAlignSpacing::Distribute(TextAlignAxis::Y),
        }),
    )
    .expect_err("two targets cannot be distributed");
    assert_invalid(error, "textalign.distribute_needs_three");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    // Four already-even texts: no write, no transaction, no undo entry.
    let (mut app, mut session) = four_distributed_texts();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextAlignEdges,
        CommandPayload::TextAlignSpacing(TextAlignSpacingCommand {
            spacing: TextAlignSpacing::Distribute(TextAlignAxis::Y),
        }),
    )
    .expect("already even");
    assert!(outcome.changes.is_none(), "no ChangeSet");
    assert!(outcome.objects.is_empty(), "nothing was rewritten");
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
fn textalign_spacing_options_outside_distribute_refuse_by_name_and_write_nothing() {
    for (spacing, code) in [
        (
            TextAlignSpacing::SetSpacing,
            "textalign.set_spacing_unsupported",
        ),
        (
            TextAlignSpacing::CurrentVertical,
            "textalign.current_vertical_unsupported",
        ),
        (
            TextAlignSpacing::CurrentHorizontal,
            "textalign.current_horizontal_unsupported",
        ),
    ] {
        let (mut app, mut session) = three_texts();
        let before = revision(&app);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::TextAlignEdges,
            CommandPayload::TextAlignSpacing(TextAlignSpacingCommand { spacing }),
        )
        .expect_err("the spacing option is refused");
        assert_invalid(error, code);
        assert_eq!(revision(&app), before, "{code}: writes nothing");
        assert_eq!(undo_depth(&app), 0, "{code}: no undo entry");
        assert_position(&app, 1, 0.0, 0.0);
    }
}
