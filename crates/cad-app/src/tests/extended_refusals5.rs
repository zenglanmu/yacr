//! Wave 5 named-refusal contracts after the block-attribute command family
//! graduated (ADR-0055 M3): `ATTIN`, `ATTOUT`, `ATTEXT`, `-ATTEXT`, `EATTEXT`,
//! `-EATTEXT`, `XDATA`, `XBIND`, `-XBIND`, `XATTACH`, `ATTACH`, `-ATTACH`,
//! `XOPEN`.
//!
//! The in-drawing definition/value commands (`ATTDEF`, `-ATTDEF`, `ATTEDIT`,
//! `-ATTEDIT`, `ATTIPEDIT`, `EATTEDIT`, `ATTREDEF`, `ATTSYNC`, `GATTE` and
//! `BURST`) are real commands now; their contracts live in
//! [`crate::tests::attribute_commands`]. The six file lanes above remain a
//! separate implementation slice and are still named refusals.

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 13] = [
    (
        ExtendedCommandId::AttIn,
        "attin.unsupported: importing attributes from an external file is not implemented",
    ),
    (
        ExtendedCommandId::AttOut,
        "attout.unsupported: exporting attributes to an external file is not implemented",
    ),
    (
        ExtendedCommandId::AttExt,
        "attext.unsupported: attribute extraction to a template/file is not implemented",
    ),
    (
        ExtendedCommandId::DashAttExt,
        "-attext.unsupported: attribute extraction to a template/file is not implemented",
    ),
    (
        ExtendedCommandId::EAttExt,
        "eatttext.unsupported: attribute extraction to a template/file is not implemented",
    ),
    (
        ExtendedCommandId::DashEAttExt,
        "-eatttext.unsupported: attribute extraction to a template/file is not implemented",
    ),
    (
        ExtendedCommandId::XData,
        "xdata.unsupported: this database does not retain entity XDATA",
    ),
    (
        ExtendedCommandId::XBind,
        "xbind.unsupported: this build has no xref bind/insert model",
    ),
    (
        ExtendedCommandId::DashXBind,
        "-xbind.unsupported: this build has no xref bind/insert model",
    ),
    (
        ExtendedCommandId::XAttach,
        "xattach.unsupported: this build does not attach external drawings as xrefs",
    ),
    (
        ExtendedCommandId::Attach,
        "attach.unsupported: this build does not attach external references or underlays",
    ),
    (
        ExtendedCommandId::DashAttach,
        "-attach.unsupported: this build does not attach external references or underlays",
    ),
    (
        ExtendedCommandId::XOpen,
        "xopen.unsupported: this build has no xref editing session",
    ),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave5_refusal_answers_with_its_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a Wave 5 refusal never succeeds");
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
fn wave5_refusals_are_also_available_in_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).unwrap();
    for (ext, expected) in REFUSALS {
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a Wave 5 refusal never succeeds");
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
                CommandId::Extended(ExtendedCommandId::AttIn),
                CommandPayload::Mode(AppMode::Work),
            ),
        )
        .expect_err("a payload is refused");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert!(error.to_string().contains("ATTIN needs no payload"));
}
