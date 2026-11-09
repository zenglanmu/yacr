//! Wave 13 contracts: page-setup/conversion reports and named refusals.
//!
//! `PAGESETUP`/`-PAGESETUP` report stored `PLOTSETTINGS` records and
//! `CONVERTPOLY` reports the single polyline representation; both write nothing.
//! The rest are stable named refusals.

use super::*;

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Extended(ext), CommandPayload::None),
    )
}

#[test]
fn pagesetup_reports_stored_records_without_writing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    for ext in [
        ExtendedCommandId::PageSetup,
        ExtendedCommandId::DashPageSetup,
    ] {
        let outcome = execute(&mut app, &mut session, ext).expect("PAGESETUP is a report");
        assert!(outcome.changes.is_none());
        assert!(outcome
            .diagnostics
            .iter()
            .any(|row| row.code == "pagesetup.report_only"));
        assert_eq!(revision(&app), before);
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn convertpoly_reports_the_single_representation() {
    let (mut app, mut session) = application_with_document();
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::ConvertPoly).expect("report");
    assert!(outcome.changes.is_none());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "convertpoly.single_representation"));
}

const REFUSALS: [(ExtendedCommandId, &str); 22] = [
    (
        ExtendedCommandId::PageSetupIn,
        "psetupin.unsupported: importing a page setup from another drawing is not implemented",
    ),
    (
        ExtendedCommandId::DashPageSetupIn,
        "-psetupin.unsupported: importing a page setup from another drawing is not implemented",
    ),
    (
        ExtendedCommandId::PlotStyleList,
        "plotstyle.unsupported: named plot styles are not implemented (no CTB/STB table)",
    ),
    (
        ExtendedCommandId::DashPlotStyleList,
        "-plotstyle.unsupported: named plot styles are not implemented (no CTB/STB table)",
    ),
    (
        ExtendedCommandId::ConvertPlotStyles,
        "convertpstyles.unsupported: plot-style conversion is not implemented (no CTB/STB model)",
    ),
    (
        ExtendedCommandId::ConvertCtb,
        "convertctb.unsupported: CTB conversion is not implemented (no CTB/STB model)",
    ),
    (
        ExtendedCommandId::ConvertGeneric,
        "convert.unsupported: generic object conversion is not implemented",
    ),
    (
        ExtendedCommandId::TableCreate,
        "table.unsupported: no table entity model is implemented",
    ),
    (
        ExtendedCommandId::DashTable,
        "-table.unsupported: no table entity model is implemented",
    ),
    (
        ExtendedCommandId::TableEdit,
        "tabledit.unsupported: no table entity model is implemented",
    ),
    (
        ExtendedCommandId::TableExport,
        "tableexport.unsupported: no table entity model is implemented",
    ),
    (
        ExtendedCommandId::TableStyleList,
        "tablestyle.unsupported: no table-style model is implemented",
    ),
    (
        ExtendedCommandId::TableInsert,
        "tinsert.unsupported: no table entity model is implemented",
    ),
    (
        ExtendedCommandId::MatchCell,
        "matchcell.unsupported: no table entity model is implemented",
    ),
    (
        ExtendedCommandId::Field,
        "field.unsupported: fields are not implemented (no field model)",
    ),
    (
        ExtendedCommandId::RText,
        "rtext.unsupported: remote text is not implemented (no RTEXT model)",
    ),
    (
        ExtendedCommandId::RTextEdit,
        "rtedit.unsupported: remote text is not implemented (no RTEXT model)",
    ),
    (
        ExtendedCommandId::Spell,
        "spell.unsupported: no spell-check dictionary or model is implemented",
    ),
    (
        ExtendedCommandId::SuperHatch,
        "superhatch.unsupported: image/block-based super-hatching is not implemented",
    ),
    (
        ExtendedCommandId::AlignSpace,
        "alignspace.unsupported: viewport alignment is not implemented (no viewport alignment API)",
    ),
    (
        ExtendedCommandId::VpSync,
        "vpsync.unsupported: viewport synchronization is not implemented (no viewport group model)",
    ),
    (
        ExtendedCommandId::LayTrans,
        "laytrans.unsupported: layer translation standards are not implemented",
    ),
];

#[test]
fn every_wave13_refusal_answers_with_its_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = execute(&mut app, &mut session, ext).expect_err("a refusal never succeeds");
        match error {
            CadError::Unsupported(reason) => assert_eq!(reason, expected),
            other => panic!("{ext:?} must be Unsupported, got {other:?}"),
        }
        assert_eq!(revision(&app), before);
        assert!(!app.can_undo(&DocumentId(1)));
        assert!(!ext.requires_work_mode(), "{ext:?} stays Viewer-legal");
    }
}

#[test]
fn wave13_reports_are_viewer_legal() {
    for ext in [
        ExtendedCommandId::PageSetup,
        ExtendedCommandId::DashPageSetup,
        ExtendedCommandId::ConvertPoly,
    ] {
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is legal");
        execute(&mut app, &mut session, ext).unwrap_or_else(|error| panic!("{ext:?}: {error:?}"));
    }
}
