//! Wave 6 named-refusal contracts: `SAVEALL`, `QNEW`, `REVERT`, `CLOSEALL`,
//! `CLOSEALLOTHER`, `QQUIT`, `QUIT`, `PROPULATE`, `PLOTTERMANAGER`,
//! `STYLESMANAGER`, `EXPORTSETTINGS`, `PREVIEW`, `EXPORTLAYOUT`, `PLOTSTAMP`,
//! `-PLOTSTAMP`, `-PLOT`, `-EXPORT`, `ETRANSMIT`, `-ETRANSMIT`, `ARCHIVE`,
//! `-ARCHIVE`, `PUBLISH`, `-PUBLISH`, `AUTOPUBLISH`.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal must write
//! nothing, bump no revision and record no history. These names are
//! Viewer-legal (they only answer honestly).

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 24] = [
    (
        ExtendedCommandId::SaveAll,
        "saveall.single_document: this build has one document; use SAVE",
    ),
    (ExtendedCommandId::QNew, "qnew.single_document: use NEW"),
    (
        ExtendedCommandId::Revert,
        "revert.unsupported: no reopen-to-source path",
    ),
    (
        ExtendedCommandId::CloseAll,
        "closeall.single_document: use CLOSE",
    ),
    (
        ExtendedCommandId::CloseAllOther,
        "closeallother.single_document: only one document exists",
    ),
    (
        ExtendedCommandId::QQuit,
        "qquit.unsupported: application exit is host-owned",
    ),
    (
        ExtendedCommandId::Quit,
        "quit.unsupported: application exit is host-owned",
    ),
    (
        ExtendedCommandId::Propulate,
        "propulate.unsupported: no property-propagation model",
    ),
    (
        ExtendedCommandId::PlotterManager,
        "plottermanager.unsupported: no plotter configuration model",
    ),
    (
        ExtendedCommandId::StylesManager,
        "stylesmanager.unsupported: no CTB/STB plot-style model",
    ),
    (
        ExtendedCommandId::ExportSettings,
        "exportsettings.unsupported: no export-settings model",
    ),
    (
        ExtendedCommandId::Preview,
        "preview.unsupported: no plot-preview window",
    ),
    (
        ExtendedCommandId::ExportLayout,
        "exportlayout.unsupported: use PLOT (one layout, SVG/PDF)",
    ),
    (
        ExtendedCommandId::PlotStamp,
        "plotstamp.unsupported: no plot-stamp model",
    ),
    (
        ExtendedCommandId::DashPlotStamp,
        "-plotstamp.unsupported: no plot-stamp model",
    ),
    (
        ExtendedCommandId::DashPlot,
        "-plot.unsupported: use PLOT; this build has one vector plot path",
    ),
    (
        ExtendedCommandId::DashExport,
        "-export.unsupported: use PLOT/EXPORT; this build has one vector export path",
    ),
    (
        ExtendedCommandId::ETransmit,
        "etransmit.unsupported: no eTransmit/zip packaging model",
    ),
    (
        ExtendedCommandId::DashETransmit,
        "-etransmit.unsupported: no eTransmit/zip packaging model",
    ),
    (
        ExtendedCommandId::Archive,
        "archive.unsupported: no sheet-set archive packaging model",
    ),
    (
        ExtendedCommandId::DashArchive,
        "-archive.unsupported: no sheet-set archive packaging model",
    ),
    (
        ExtendedCommandId::Publish,
        "publish.unsupported: no batch publish model",
    ),
    (
        ExtendedCommandId::DashPublish,
        "-publish.unsupported: no batch publish model",
    ),
    (
        ExtendedCommandId::AutoPublish,
        "autopublish.unsupported: no automatic publish model",
    ),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave6_refusal_answers_with_its_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a Wave 6 refusal never succeeds");
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
fn wave6_refusals_are_also_available_in_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).unwrap();
    for (ext, expected) in REFUSALS {
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a Wave 6 refusal never succeeds");
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
                CommandId::Extended(ExtendedCommandId::SaveAll),
                CommandPayload::Mode(AppMode::Work),
            ),
        )
        .expect_err("a payload is refused");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert!(error.to_string().contains("SAVEALL needs no payload"));
}
