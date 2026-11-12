//! `TEXTFIT` verification for the two modelled text kinds.
//!
//! `TEXTFIT` is implemented in the annotation batch
//! (`app_drawing::annot_batch::textfit`); this file pins the parts of its
//! contract the 2026-11 text lane audited: `AcDbText` and `AcDbMText` share
//! `SemanticGeometry::Text`, so both are fitted, and **only** the horizontal
//! width factor (DXF TEXT group 41) changes — the string, position, rotation,
//! style, font, both alignments, the height, the MTEXT wrap boundary, the layer,
//! the draw order and the entity id survive.
//!
//! The declared limit is explicit and is not papered over: the factor is
//! `target_width / estimated_unstretched_width`, where the estimate is the
//! batch's `height × char_count`. The rendered width is exact only to that
//! estimate (not to real glyph metrics), and every written entity carries a
//! `textfit.estimated_width` diagnostic naming the estimate and factor.

use super::*;

// --- TEXTFIT -----------------------------------------------------------------
//
// `TEXTFIT` lives in the annotation batch (`app_drawing::annot_batch::textfit`);
// this lane verifies the parts its own audit covers: both modelled text kinds
// (`AcDbText` and `AcDbMText` share `SemanticGeometry::Text`), the exact
// preservation of every field other than the width factor, and the named
// refusals.

fn textfit_payload(target_width: f64) -> CommandPayload {
    CommandPayload::Annotation(Box::new(AnnotationBatchPayload::TextFit(TextFitCommand {
        target_width,
    })))
}

#[test]
fn textfit_stretches_mtext_by_width_factor_and_preserves_every_other_field() {
    let mut source = text_entity(1, "abcde", p3(1.0, 2.0, 0.0), 0.25, 4, "AcDbMText");
    if let SemanticGeometry::Text { wrap_width, .. } = &mut source.geometry {
        *wrap_width = Some(7.5);
    }
    let (mut app, mut session) = app_with(vec![source]);
    select(&mut session, &[1]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextFit,
        textfit_payload(20.0),
    )
    .expect("TEXTFIT an MTEXT");
    assert_eq!(outcome.objects, vec![ObjectId(1)], "same entity id");
    let fitted = entity(&app, 1).expect("entity exists");
    assert_eq!(fitted.object.type_key, "AcDbMText", "the kind is kept");
    let SemanticGeometry::Text {
        text,
        position,
        height,
        rotation,
        h_align,
        v_align,
        wrap_width,
        width_factor,
        ..
    } = &fitted.geometry
    else {
        panic!("expected Text");
    };
    // Unstretched estimate = height 2 × 5 chars = 10; 20 / 10 = 2.
    assert_eq!(*width_factor, 2.0, "the width factor is target / estimate");
    assert_eq!(*height, 2.0, "the glyph height is untouched");
    assert_eq!(text, "abcde", "the string is untouched");
    assert_eq!(*position, p3(1.0, 2.0, 0.0));
    assert_eq!(*rotation, 0.25);
    assert_eq!(*h_align, TextAlignH::Center);
    assert_eq!(*v_align, TextAlignV::Middle);
    assert_eq!(*wrap_width, Some(7.5), "the wrap boundary is untouched");
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");

    // The approximation is reported, not silent: the estimate and the written
    // factor are named on this entity.
    let estimate = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "textfit.estimated_width")
        .expect("the estimate diagnostic is emitted");
    assert_eq!(estimate.object, Some(ObjectId(1)));
    assert!(
        estimate.message.contains("10") && estimate.message.contains("2"),
        "the diagnostic names the estimate and factor: {}",
        estimate.message
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "textfit.applied"));

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    let SemanticGeometry::Text {
        height,
        width_factor,
        ..
    } = &entity(&app, 1).unwrap().geometry
    else {
        panic!("expected Text");
    };
    assert_eq!(*height, 2.0, "undo restores the stored height");
    assert_eq!(*width_factor, 1.0, "undo restores the stored width factor");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn textfit_target_width_round_trips_the_estimated_width() {
    // A positive non-1 factor round-trips through the same estimate the batch
    // uses (`height × char_count × factor`).
    let (mut app, mut session) = app_with(vec![text_entity(
        1,
        "abcd",
        p3(0.0, 0.0, 0.0),
        0.0,
        0,
        "AcDbText",
    )]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TextFit,
        textfit_payload(3.5),
    )
    .expect("TEXTFIT a TEXT");
    let fitted = entity(&app, 1).unwrap();
    let estimated = crate::app_drawing::text_batch::estimate_text_bounds(&fitted, "test")
        .expect("the fitted text has finite estimated bounds");
    assert!(
        (estimated.width() - 3.5).abs() < 1e-12,
        "estimated width {} should equal the requested 3.5",
        estimated.width()
    );
}

#[test]
fn textfit_refuses_a_non_positive_or_non_finite_target() {
    let (mut app, mut session) = app_with(vec![text_entity(
        1,
        "abcde",
        p3(0.0, 0.0, 0.0),
        0.0,
        0,
        "AcDbText",
    )]);
    select(&mut session, &[1]);
    let before = revision(&app);
    for target in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::TextFit,
            textfit_payload(target),
        )
        .expect_err("a degenerate target is refused");
        assert_invalid(error, "textfit.bad_width");
    }
    assert_eq!(revision(&app), before, "every refusal writes nothing");
    assert_eq!(undo_depth(&app), 0);
    assert!(
        matches!(
            &entity(&app, 1).unwrap().geometry,
            SemanticGeometry::Text { width_factor, .. } if *width_factor == 1.0
        ),
        "the stored factor is untouched"
    );
}

#[test]
fn textfit_refuses_an_empty_string_and_reports_an_already_fitted_text() {
    let (mut app, mut session) = app_with(vec![text_entity(
        1,
        "",
        p3(0.0, 0.0, 0.0),
        0.0,
        0,
        "AcDbText",
    )]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextFit,
        textfit_payload(10.0),
    )
    .expect_err("an empty string has no estimated width");
    assert_invalid(error, "textfit.empty_text");
    assert_eq!(revision(&app), before, "the refusal writes nothing");
    assert_eq!(undo_depth(&app), 0);

    // 5 characters at height 2 are already 10 wide with factor 1: no
    // transaction at all and no invented factor.
    let (mut app, mut session) = app_with(vec![text_entity(
        1,
        "abcde",
        p3(0.0, 0.0, 0.0),
        0.0,
        0,
        "AcDbText",
    )]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextFit,
        textfit_payload(10.0),
    )
    .expect("already fitted");
    assert!(outcome.changes.is_none(), "no ChangeSet");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "textfit.unchanged"),
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn textfit_updates_a_multi_selection_in_one_transaction_and_one_undo() {
    let (mut app, mut session) = app_with(vec![
        text_entity(1, "ab", p3(0.0, 0.0, 0.0), 0.0, 0, "AcDbText"),
        text_entity(2, "cdef", p3(5.0, 0.0, 0.0), 0.0, 1, "AcDbText"),
    ]);
    select(&mut session, &[1, 2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextFit,
        textfit_payload(6.0),
    )
    .expect("TEXTFIT two texts");
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(undo_depth(&app), 1, "one undo step for the whole selection");
    for id in [1u128, 2] {
        let entity = entity(&app, id).unwrap();
        let estimated = crate::app_drawing::text_batch::estimate_text_bounds(&entity, "test")
            .expect("finite estimate");
        assert!(
            (estimated.width() - 6.0).abs() < 1e-12,
            "entity {id} estimated width {} should be 6",
            estimated.width()
        );
    }
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert_eq!(undo_depth(&app), 0);
    for id in [1u128, 2] {
        assert!(
            matches!(
                &entity(&app, id).unwrap().geometry,
                SemanticGeometry::Text { width_factor, .. } if *width_factor == 1.0
            ),
            "entity {id} is restored in one undo"
        );
    }
}
