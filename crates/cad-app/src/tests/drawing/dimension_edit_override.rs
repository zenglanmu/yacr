//! Contract tests for the dimension **override** and no-op half of the
//! 2026-11-07 batch: `DIMOVERRIDE`, `DIMREGEN`, `DIMDISASSOCIATE`, `DIMJOGLINE`,
//! `DIMBREAK` and the Viewer-mode refusal.
//!
//! Split out of [`super::dimension_edit`] to keep that file inside the code-size
//! limit; the fixture (the drawing, the selection helpers) stays there.

use super::dimension_edit::{
    approx, dimension_edit_app, edit_payload, select, stored, text_edit_payload, undo_depth, DIM_A,
    DIM_B,
};
use super::*;

fn override_payload(variable: &str, value: &str) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::Override(
        DimOverrideCommand {
            variable: variable.to_string(),
            value: value.to_string(),
        },
    )))
}

fn jog_payload(command: DimJogLineCommand) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::JogLine(command)))
}

#[test]
fn dimoverride_applies_the_closed_snapshot_set_and_refuses_the_rest() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A, DIM_B]);
    let before = revision(&app);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimOverride,
        override_payload("DIMTXT", "5"),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 2);
    assert!(approx(stored(&app, DIM_A).style().text_height, 5.0));
    assert!(approx(stored(&app, DIM_B).style().text_height, 5.0));
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::DimOverride,
        override_payload("DIMDEC", "3"),
    )
    .unwrap();
    assert_eq!(stored(&app, DIM_A).style().decimals, 3);
    assert_eq!(undo_depth(&app), 2);

    // The DIMSTYLE table itself is untouched: the change is the entity snapshot.
    let table_height = app.workspace.documents[&DocumentId(1)]
        .drawing
        .dim_style_by_name("Standard")
        .expect("Standard")
        .text_height;
    assert!(approx(table_height, 2.5));

    let checks = [
        ("DIMFOO", "1", "dimoverride.unknown_variable"),
        ("DIMCLRD", "1", "dimoverride.unsupported_variable"),
        ("DIMTXT", "0", "dimoverride.bad_value"),
        ("DIMLUNIT", "4", "dimoverride.unsupported_value"),
        ("DIMZIN", "3", "dimoverride.unsupported_value"),
    ];
    for (variable, value, code) in checks {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::DimOverride,
            override_payload(variable, value),
        )
        .unwrap_err();
        assert!(format!("{error}").contains(code), "{variable}: {error}");
    }
    assert_eq!(revision(&app).0, before.0 + 2);
    assert_eq!(undo_depth(&app), 2);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimOverride,
        override_payload("DIMTXT", "5"),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "dimoverride.unchanged");
    assert!(out.changes.is_none());
    assert_eq!(undo_depth(&app), 2);
}

#[test]
fn dimoverride_dimtxsty_swaps_the_text_style_from_the_style_table() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A]);

    execute(
        &mut app,
        &mut session,
        CommandId::DimOverride,
        override_payload("DIMTXSTY", "Note"),
    )
    .unwrap();
    let dimension = stored(&app, DIM_A);
    assert_eq!(dimension.style().text_style_name, "Note");
    assert_eq!(dimension.style().text_style, StyleId(4));
    assert_eq!(dimension.style().text_font.as_deref(), Some("arial.ttf"));
    // The measurement text child carries the resolved style and font, so the
    // override is honoured by the display rather than only stored.
    let (text_style, text_font) = dimension
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Text { style, font, .. } => Some((*style, font.clone())),
            _ => None,
        })
        .expect("the dimension has a text child");
    assert_eq!(text_style, StyleId(4));
    assert_eq!(text_font.as_deref(), Some("arial.ttf"));

    // An unknown style name is a value refusal, never a silent fallback.
    let revision_before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimOverride,
        override_payload("DIMTXSTY", "Nope"),
    )
    .unwrap_err();
    let text = format!("{error}");
    assert!(text.contains("dimoverride.bad_value"), "{text}");
    assert!(text.contains("no text style named 'Nope'"), "{text}");
    assert_eq!(revision(&app), revision_before);
    assert_eq!(stored(&app, DIM_A).style().text_style_name, "Note");
}

#[test]
fn dimregen_and_dimdisassociate_are_named_noops() {
    let (mut app, mut session) = dimension_edit_app();
    let before = revision(&app);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimRegen,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "dimregen.non_associative");
    assert!(out.changes.is_none());
    assert!(out.objects.is_empty());

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimDisassociate,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        out.diagnostics[0].code,
        "dimdisassociate.already_non_associative"
    );
    assert!(out.changes.is_none());

    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimRegen,
        CommandPayload::Points(Vec::new()),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("dimregen.payload"), "{error}");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimDisassociate,
        CommandPayload::Points(Vec::new()),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimdisassociate.payload"),
        "{error}"
    );
}

#[test]
fn dimjogline_and_dimbreak_name_the_missing_domain_field() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A]);
    let before = revision(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimJogLine,
        jog_payload(DimJogLineCommand::Add { x: 1.0, y: 2.0 }),
    )
    .unwrap_err();
    let text = format!("{error}");
    assert!(text.contains("dimjogline.unsupported"), "{text}");
    assert!(text.contains("no jog field"), "{text}");

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimJogLine,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimjogline.needs_option"),
        "{error}"
    );

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimBreak,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimbreak.unsupported"),
        "{error}"
    );

    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    // The dimension is still exactly where it was: DIMBREAK never trims it.
    let dimension = stored(&app, DIM_A);
    match &dimension.definition().points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => {
            assert_eq!(*first, point(0.0, 0.0));
            assert_eq!(*second, point(10.0, 0.0));
            assert_eq!(*dimension_line, point(5.0, -5.0));
        }
        other => panic!("expected Linear points, got {other:?}"),
    }
}

#[test]
fn viewer_mode_refuses_the_work_only_dimension_edits() {
    let (mut app, mut session) = dimension_edit_app();
    session.switch_mode(AppMode::Viewer).expect("Viewer mode");
    select(&mut session, &[DIM_A]);

    for (id, payload) in [
        (
            CommandId::DimTextEdit,
            text_edit_payload(DimTextEditCommand::Home),
        ),
        (CommandId::DimEdit, edit_payload(DimEditCommand::Reset)),
        (CommandId::DimOverride, override_payload("DIMTXT", "5")),
    ] {
        let error = execute(&mut app, &mut session, id, payload).unwrap_err();
        assert!(
            matches!(error, CadError::PermissionDenied),
            "{id:?}: {error:?}"
        );
    }
    let error = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
}
