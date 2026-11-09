//! Wave 8 named-refusal contracts: UCS/tablet/window/menu surfaces.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal must write
//! nothing, bump no revision and record no history. These names are
//! Viewer-legal (they only answer honestly).

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 8] = [
    (
        ExtendedCommandId::Ucs,
        "ucs.unsupported: this build has no UCS model",
    ),
    (
        ExtendedCommandId::UcsMan,
        "ucsman.unsupported: this build has no UCS model",
    ),
    (
        ExtendedCommandId::Tablet,
        "tablet.unsupported: no tablet/digitizer model",
    ),
    (
        ExtendedCommandId::TaskBar,
        "taskbar.unsupported: application window chrome is host-owned",
    ),
    (
        ExtendedCommandId::Reinit,
        "reinit.unsupported: no device/digitizer initialization model",
    ),
    (
        ExtendedCommandId::SysWindows,
        "syswindows.unsupported: application window layout is host-owned",
    ),
    (
        ExtendedCommandId::Menu,
        "menu.unsupported: no menu-file model; the UI is fixed",
    ),
    (
        ExtendedCommandId::Ribbon,
        "ribbon.unsupported: ribbon visibility is host-owned chrome",
    ),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave8_refusal_answers_with_its_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a Wave 8 refusal never succeeds");
        assert_eq!(
            error,
            CadError::Unsupported(expected.to_string()),
            "{}",
            ext.selector()
        );
        assert_eq!(revision(&app), before, "{} writes nothing", ext.selector());
        assert!(
            !app.can_undo(&DocumentId(1)),
            "{} records no history",
            ext.selector()
        );
        assert!(
            !ext.requires_work_mode(),
            "{} stays Viewer-legal",
            ext.selector()
        );
    }
}

#[test]
fn wave8_refusals_are_also_available_in_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).unwrap();
    for (ext, expected) in REFUSALS {
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a Wave 8 refusal never succeeds");
        assert_eq!(
            error,
            CadError::Unsupported(expected.to_string()),
            "{}",
            ext.selector()
        );
    }
}

#[test]
fn a_caller_payload_is_refused_before_the_feature_reason() {
    let (mut app, mut session) = application_with_document();
    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::Ucs),
                CommandPayload::Mode(AppMode::Work),
            ),
        )
        .expect_err("a payload is refused");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert!(error.to_string().contains("UCS needs no payload"));
}
