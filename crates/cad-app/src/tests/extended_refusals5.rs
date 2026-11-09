//! Wave 5 named-refusal contracts: `ATTDEF`, `-ATTDEF`, `ATTEDIT`, `-ATTEDIT`,
//! `ATTIPEDIT`, `ATTIN`, `ATTOUT`, `ATTEXT`, `-ATTEXT`, `EATTEDIT`, `EATTEXT`,
//! `-EATTEXT`, `ATTREDEF`, `ATTSYNC`, `GATTE`, `XDATA`, `XBIND`, `-XBIND`,
//! `XATTACH`, `ATTACH`, `-ATTACH`, `XOPEN`, `CLASSICXREF`.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal must write
//! nothing, bump no revision and record no history. These names are
//! Viewer-legal (they only answer honestly).

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 23] = [
    (
        ExtendedCommandId::AttDef,
        "attdef.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::DashAttDef,
        "-attdef.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::AttEdit,
        "attedit.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::DashAttEdit,
        "-attedit.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::AttIpEdit,
        "attipedit.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::AttIn,
        "attin.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::AttOut,
        "attout.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::AttExt,
        "attext.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::DashAttExt,
        "-attext.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::EAttEdit,
        "eattedit.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::EAttExt,
        "eatttext.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::DashEAttExt,
        "-eatttext.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::AttRedef,
        "attredef.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::AttSync,
        "attsync.unsupported: this build has no block-attribute model",
    ),
    (
        ExtendedCommandId::Gatte,
        "gatte.unsupported: this build has no block-attribute model",
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
    (
        ExtendedCommandId::ClassicXRef,
        "classicxref.unsupported: there is no separate xref palette; use the RESOURCES panel",
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
                CommandId::Extended(ExtendedCommandId::AttDef),
                CommandPayload::Mode(AppMode::Work),
            ),
        )
        .expect_err("a payload is refused");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert!(error.to_string().contains("ATTDEF needs no payload"));
}
