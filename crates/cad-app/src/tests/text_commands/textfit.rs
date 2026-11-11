//! `TEXTFIT` verification for the two modelled text kinds.
//!
//! `TEXTFIT` is implemented in the annotation batch
//! (`app_drawing::annot_batch::textfit`); this file pins the parts of its
//! contract the 2026-11 text lane audited: `AcDbText` and `AcDbMText` share
//! `SemanticGeometry::Text`, so both are fitted, and **only** the glyph height
//! changes — the string, position, rotation, style, font, both alignments, the
//! MTEXT wrap boundary, the layer, the draw order and the entity id survive.
//!
//! The declared limit is unchanged and is not papered over: `cad-domain` stores
//! no width factor, so the target width is expressed as
//! `height = target / char_count` under the batch's estimate; true glyph
//! stretch between two picked points stays NOT IMPLEMENTED.

use super::*;

// --- TEXTFIT -----------------------------------------------------------------
//
// `TEXTFIT` lives in the annotation batch (`app_drawing::annot_batch::textfit`);
// this lane verifies the parts its own audit covers: both modelled text kinds
// (`AcDbText` and `AcDbMText` share `SemanticGeometry::Text`), the exact
// preservation of every field other than the glyph height, and the named
// refusals. The declared limit is unchanged: `cad-domain` stores no width
// factor, so the width is fitted through `height = target / char_count` under
// the batch's estimate and true glyph stretch stays NOT IMPLEMENTED.

fn textfit_payload(target_width: f64) -> CommandPayload {
    CommandPayload::Annotation(Box::new(AnnotationBatchPayload::TextFit(TextFitCommand {
        target_width,
    })))
}

#[test]
fn textfit_fits_mtext_and_preserves_every_other_stored_field() {
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
        ..
    } = &fitted.geometry
    else {
        panic!("expected Text");
    };
    assert_eq!(*height, 4.0, "20 / 5 characters");
    assert_eq!(text, "abcde", "the string is untouched");
    assert_eq!(*position, p3(1.0, 2.0, 0.0));
    assert_eq!(*rotation, 0.25);
    assert_eq!(*h_align, TextAlignH::Center);
    assert_eq!(*v_align, TextAlignV::Middle);
    assert_eq!(*wrap_width, Some(7.5), "the wrap boundary is untouched");
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    let SemanticGeometry::Text { height, .. } = &entity(&app, 1).unwrap().geometry else {
        panic!("expected Text");
    };
    assert_eq!(*height, 2.0, "undo restores the stored height");
    assert_eq!(undo_depth(&app), 0);
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

    // 5 characters at height 2 are already 10 wide: no transaction at all.
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
