//! `SHOWOBJECTS`: per-selection reveal of session-hidden objects.
//!
//! Split out of `select_filter.rs` for the code-size gate; the fixtures and
//! command helper are re-used from that module.

use super::select_filter::{
    filter_app, model_ref, revision, run, undo_depth, CIRCLE, LINE_PLAIN, TEXT,
};
use super::*;

#[test]
fn show_objects_reveals_only_the_selection_and_is_viewer_legal() {
    let (mut app, mut session) = filter_app();
    let revision_before = revision(&app);
    session
        .selection
        .replace([model_ref(LINE_PLAIN), model_ref(CIRCLE)]);
    run(
        &mut app,
        &mut session,
        CommandId::HideObjects,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(session.hidden_entities.len(), 2);

    session.selection.replace([model_ref(LINE_PLAIN)]);
    let outcome = run(
        &mut app,
        &mut session,
        CommandId::ShowObjects,
        CommandPayload::None,
    )
    .unwrap();
    assert!(!session.hidden_entities.contains(&model_ref(LINE_PLAIN)));
    assert!(
        session.hidden_entities.contains(&model_ref(CIRCLE)),
        "only the selection is revealed"
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "showobjects.shown"));
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), 0);

    // A non-hidden selection is a reported no-op.
    session.selection.replace([model_ref(TEXT)]);
    let outcome = run(
        &mut app,
        &mut session,
        CommandId::ShowObjects,
        CommandPayload::None,
    )
    .unwrap();
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "showobjects.unchanged"));

    session.switch_mode(AppMode::Viewer).expect("Viewer");
    run(
        &mut app,
        &mut session,
        CommandId::ShowObjects,
        CommandPayload::None,
    )
    .expect("revealing session state is Viewer-legal");
}

#[test]
fn show_objects_refuses_an_empty_selection() {
    let (mut app, mut session) = filter_app();
    let error = run(
        &mut app,
        &mut session,
        CommandId::ShowObjects,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CadError::InvalidInput(ref m) if m.contains("showobjects.selection_required")
    ));
}
