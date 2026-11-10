//! Waves 17-24 contracts: the 200 data-driven named refusals.
//!
//! Every selector in [`RefusalId::ALL`] must reach its stable
//! `<selector>.unsupported: ...` reason, write nothing, bump no revision,
//! record no undo entry and stay Viewer-legal. One test covers the whole
//! registry so adding a name can never silently become a no-op or a write.

use super::*;

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave17_refusal_refuses_with_its_stable_reason() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    assert_eq!(RefusalId::ALL.len(), 200, "the batch is 200 names");
    for &name in RefusalId::ALL {
        let ext = ExtendedCommandId::Refusal(name);
        let selector = name.selector();
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .unwrap_err();
        match error {
            CadError::Unsupported(reason) => {
                assert_eq!(reason, name.reason(), "{selector}");
                let prefix = format!("{}.unsupported: ", selector.to_ascii_lowercase());
                assert!(reason.starts_with(&prefix), "{selector}: {reason}");
            }
            other => panic!("{selector} must be Unsupported, got {other:?}"),
        }
        assert_eq!(revision(&app), before, "{selector} must not write");
        assert!(!app.can_undo(&DocumentId(1)), "{selector} must not undo");
        assert!(!ext.requires_work_mode(), "{selector} stays Viewer-legal");
    }
}

#[test]
fn wave17_refusals_are_viewer_legal() {
    let (mut app, mut session) = application_with_document();
    session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is legal");
    for &name in RefusalId::ALL {
        let error = app
            .execute(
                &mut session,
                command(
                    CommandId::Extended(ExtendedCommandId::Refusal(name)),
                    CommandPayload::None,
                ),
            )
            .unwrap_err();
        assert!(
            matches!(error, CadError::Unsupported(_)),
            "{}",
            name.selector()
        );
    }
}

#[test]
fn a_refusal_rejects_a_payload_before_the_feature_reason() {
    let (mut app, mut session) = application_with_document();
    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::Refusal(RefusalId::Actbasepoint)),
                CommandPayload::Points(vec![Point3::default()]),
            ),
        )
        .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("ACTBASEPOINT"), "{text}");
    assert!(!text.contains(".unsupported:"), "{text}");
}
