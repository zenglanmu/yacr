use super::*;

/// `MODEL` is the payload-free spelling of `SPACE MODEL`: it switches the
/// session's active space and touches nothing else.
#[test]
fn model_switches_to_model_space_without_writing_the_drawing() {
    let (mut app, mut session) = application_with_document();
    let document = DocumentId(1);
    let revision = app.workspace.documents[&document].drawing.revision();
    assert_eq!(session.active_space, SpaceId::Model);
    assert!(!app.can_undo(&document));

    let outcome = app
        .execute(
            &mut session,
            command(CommandId::ModelSpace, CommandPayload::None),
        )
        .expect("MODEL is admitted on model space");

    assert_eq!(session.active_space, SpaceId::Model);
    // Session state only: no database write, no ChangeSet and no undo entry.
    assert!(outcome.changes.is_none());
    assert!(outcome.objects.is_empty());
    assert_eq!(
        app.workspace.documents[&document].drawing.revision(),
        revision
    );
    assert!(!app.can_undo(&document), "MODEL records no undo step");
}

#[test]
fn model_refuses_a_payload_explicitly() {
    let (mut app, mut session) = application_with_document();
    let document = DocumentId(1);
    let revision = app.workspace.documents[&document].drawing.revision();

    let error = app
        .execute(
            &mut session,
            command(CommandId::ModelSpace, CommandPayload::Space(SpaceId::Model)),
        )
        .unwrap_err();

    match error {
        CadError::InvalidInput(message) => assert!(
            message.contains("MODEL needs no payload"),
            "the refusal names the rule: {message}"
        ),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert_eq!(session.active_space, SpaceId::Model);
    assert_eq!(
        app.workspace.documents[&document].drawing.revision(),
        revision
    );
}

#[test]
fn model_on_a_stale_document_is_refused() {
    let (mut app, mut session) = application_with_document();
    let mut cmd = command(CommandId::ModelSpace, CommandPayload::None);
    cmd.document = DocumentId(99);

    assert_eq!(
        app.execute(&mut session, cmd).unwrap_err(),
        CadError::StaleResult
    );
    assert_eq!(session.active_space, SpaceId::Model);
}
