//! Panel-action refusals: `MATCHPHOTO` and `OVERLAYS` have no model.
//!
//! Both write nothing and report a stable marker; the panels themselves are
//! host-read-backed. Contract level: no GUI, no GPU.

use super::*;

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map_or(0, cad_history::History::undo_depth)
}

#[test]
fn match_photo_is_a_named_refusal_that_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let error = app
        .execute(
            &mut session,
            command(CommandId::MatchPhoto, CommandPayload::None),
        )
        .expect_err("photo matching is not modeled");
    let text = match error {
        CadError::Unsupported(text) => text,
        other => panic!("expected Unsupported, got {other:?}"),
    };
    assert!(text.contains("matchphoto.not_implemented"), "{text}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn overlays_is_a_named_refusal_that_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let error = app
        .execute(
            &mut session,
            command(CommandId::Overlays, CommandPayload::None),
        )
        .expect_err("there is no user-managed overlay model");
    let text = match error {
        CadError::Unsupported(text) => text,
        other => panic!("expected Unsupported, got {other:?}"),
    };
    assert!(text.contains("overlays.not_implemented"), "{text}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn both_panel_refusals_are_legal_in_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    for id in [CommandId::MatchPhoto, CommandId::Overlays] {
        let error = app
            .execute(&mut session, command(id, CommandPayload::None))
            .expect_err("a refusal, not a write");
        assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    }
}
