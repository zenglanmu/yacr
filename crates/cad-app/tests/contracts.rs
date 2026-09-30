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
    let mut app = Application {
        workspace: Workspace::default(),
    };
    let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
    let command = |id, document| Command {
        schema_version: 1,
        id,
        document,
        viewport: ViewportId(1),
        payload: CommandPayload::None,
    };
    assert!(matches!(
        app.execute(
            &mut session,
            command(CommandId::DeleteAnnotation, DocumentId(1))
        ),
        Err(CadError::PermissionDenied)
    ));
    assert!(matches!(
        app.execute(&mut session, command(CommandId::Pan, DocumentId(2))),
        Err(CadError::StaleResult)
    ));
    assert!(matches!(
        app.execute(&mut session, command(CommandId::Pan, DocumentId(1))),
        Err(CadError::NotImplemented(_))
    ));
}
