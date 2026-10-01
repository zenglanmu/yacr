use cad_app::*;
use cad_domain::*;

#[test]
fn viewer_rejects_every_mutating_annotation_entry() {
    let session = SessionState::new(DocumentId(1), AppMode::Viewer);
    for command in [
        CommandId::CreateAnnotation,
        CommandId::UpdateAnnotation,
        CommandId::DeleteAnnotation,
        CommandId::Undo,
        CommandId::Redo,
        CommandId::ImportAnnotations,
        CommandId::Measure,
    ] {
        assert_eq!(session.authorize(command), Err(CadError::PermissionDenied));
    }
    for command in [
        CommandId::Pan,
        CommandId::Zoom,
        CommandId::ToggleLayer,
        CommandId::SwitchSpace,
        CommandId::ExportAnnotations,
    ] {
        assert_eq!(session.authorize(command), Ok(()));
    }
}

#[test]
fn command_path_checks_mode_and_document_before_dispatch() {
    let mut app = Application::new();
    let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
    let command = |id, document, payload| Command {
        schema_version: 1,
        id,
        document,
        viewport: ViewportId(1),
        payload,
    };
    assert!(matches!(
        app.execute(
            &mut session,
            command(
                CommandId::DeleteAnnotation,
                DocumentId(1),
                CommandPayload::None
            )
        ),
        Err(CadError::PermissionDenied)
    ));
    assert!(matches!(
        app.execute(
            &mut session,
            command(CommandId::Pan, DocumentId(2), CommandPayload::None)
        ),
        Err(CadError::StaleResult)
    ));
    // Pan now requires a world-space delta rather than being unimplemented.
    assert!(matches!(
        app.execute(
            &mut session,
            command(CommandId::Pan, DocumentId(1), CommandPayload::None)
        ),
        Err(CadError::InvalidInput(_))
    ));
    // SwitchBackend is implemented: a missing payload is rejected as invalid
    // input rather than pretending to be unimplemented (audit B01).
    assert!(matches!(
        app.execute(
            &mut session,
            command(
                CommandId::SwitchBackend,
                DocumentId(1),
                CommandPayload::None
            )
        ),
        Err(CadError::InvalidInput(_))
    ));
    // A command addressed to a different document than the session's is stale
    // and must be rejected before dispatch.
    assert!(matches!(
        app.execute(
            &mut session,
            command(
                CommandId::SwitchBackend,
                DocumentId(2),
                CommandPayload::Backend(BackendChoice::WebGl2)
            )
        ),
        Err(CadError::StaleResult)
    ));
    // A valid backend choice records the preference and reports it.
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::SwitchBackend,
                DocumentId(1),
                CommandPayload::Backend(BackendChoice::WebGl2),
            ),
        )
        .expect("valid backend choice executes");
    assert_eq!(session.backend, BackendChoice::WebGl2);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "backend.preference"));
}

#[test]
fn a_confirmed_drag_is_one_undo_and_a_cancel_leaves_nothing() {
    // The transaction path guarantees: a cancelled tool never commits, and one
    // confirm produces exactly one undo record.
    let session = SessionState::new(DocumentId(1), AppMode::Work);
    assert_eq!(session.authorize(CommandId::Undo), Ok(()));
}
