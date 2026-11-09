//! Wave 10 contracts: layer/view/session reports and named refusals.
//!
//! `-LAYER`, `-SCALELISTEDIT`, `-UNITS`, `LWEIGHT`, `COLOR` and `COPYHIST` are
//! read-only reports over data this build really stores. `LAYERSTATESAVE` and
//! `NEWVIEW` route their owning payload to the one session store; without that
//! payload they report the missing prompt. Ten names are stable named refusals.

use super::*;
use crate::command_payload::LayerStateCommand;

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::Extended(ext), payload))
}

fn row<'a>(outcome: &'a CommandOutcome, code: &str) -> &'a Diagnostic {
    outcome
        .diagnostics
        .iter()
        .find(|row| row.code == code)
        .unwrap_or_else(|| panic!("missing {code}"))
}

#[test]
fn dash_layer_reports_the_real_table_without_writing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashLayer,
        CommandPayload::None,
    )
    .expect("-LAYER is admitted");
    assert!(outcome.changes.is_none());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "layer.entry" && row.message.contains("'0'")));
    assert!(row(&outcome, "layer.report_only")
        .message
        .contains("NOT IMPLEMENTED"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn layerstatesave_routes_the_owning_payload_and_reports_without_it() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let report = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::LayerStateSave,
        CommandPayload::None,
    )
    .expect("the report is admitted");
    assert!(row(&report, "layerstatesave.needs_name")
        .message
        .contains("LAYERSTATE SAVE"));

    let saved = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::LayerStateSave,
        CommandPayload::LayerState(LayerStateCommand::Save("wave10".into())),
    )
    .expect("the owning layer-state store accepts the save");
    assert!(
        saved.changes.is_none(),
        "a layer-state save is session state"
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn newview_routes_the_owning_payload_and_reports_without_it() {
    let (mut app, mut session) = application_with_document();
    let report = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::NewView,
        CommandPayload::None,
    )
    .expect("the report is admitted");
    assert!(row(&report, "newview.needs_name")
        .message
        .contains("VIEW SAVE"));
}

#[test]
fn dash_scalelistedit_reports_the_real_scale_table() {
    let (mut app, mut session) = application_with_document();
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashScaleListEdit,
        CommandPayload::None,
    )
    .expect("-SCALELISTEDIT is admitted");
    assert!(outcome.changes.is_none());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "scalelistedit.write_unsupported"));
}

#[test]
fn session_token_reports_state_their_boundaries() {
    let (mut app, mut session) = application_with_document();
    for ext in [
        ExtendedCommandId::DashUnits,
        ExtendedCommandId::LineWeight,
        ExtendedCommandId::DashLineWeight,
        ExtendedCommandId::Color,
        ExtendedCommandId::DashColor,
        ExtendedCommandId::CopyHist,
    ] {
        let before = revision(&app);
        let outcome = execute(&mut app, &mut session, ext, CommandPayload::None)
            .unwrap_or_else(|error| panic!("{ext:?} is a read-only report: {error:?}"));
        assert!(outcome.changes.is_none(), "{ext:?} writes nothing");
        assert_eq!(revision(&app), before, "{ext:?}");
        assert!(!app.can_undo(&DocumentId(1)), "{ext:?}");
    }
    let (mut app, mut session) = application_with_document();
    let weight = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::LineWeight,
        CommandPayload::None,
    )
    .unwrap();
    assert!(row(&weight, "lweight.report").message.contains("ON"));
    let color = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::Color,
        CommandPayload::None,
    )
    .unwrap();
    assert!(row(&color, "color.report").message.contains("ByLayer"));
}

const REFUSALS: [(ExtendedCommandId, &str); 10] = [
    (
        ExtendedCommandId::Help,
        "help.unsupported: the help system is host-owned and not implemented",
    ),
    (
        ExtendedCommandId::ClassicInsert,
        "classicinsert.unsupported: no classic insert dialog; use INSERT",
    ),
    (
        ExtendedCommandId::ClassicGroup,
        "classicgroup.unsupported: no classic group dialog; use GROUP",
    ),
    (
        ExtendedCommandId::FileTab,
        "filetab.unsupported: file-tab chrome is host-owned",
    ),
    (
        ExtendedCommandId::FileTabClose,
        "filetabclose.unsupported: file-tab chrome is host-owned",
    ),
    (
        ExtendedCommandId::HidePalettes,
        "hidepalettes.unsupported: palette chrome is host-owned",
    ),
    (
        ExtendedCommandId::ShowPalettes,
        "showpalettes.unsupported: palette chrome is host-owned",
    ),
    (
        ExtendedCommandId::RibbonClose,
        "ribbonclose.unsupported: ribbon chrome is host-owned; use RIBBON",
    ),
    (
        ExtendedCommandId::GoToStart,
        "gotostart.unsupported: switching to the start tab is host-owned",
    ),
    (
        ExtendedCommandId::Transparency,
        "transparency.unsupported: image transparency is not modeled",
    ),
];

#[test]
fn every_wave10_refusal_answers_with_its_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = execute(&mut app, &mut session, ext, CommandPayload::None)
            .expect_err("a refusal never succeeds");
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
fn only_layerstatesave_is_work_only() {
    for ext in [
        ExtendedCommandId::DashLayer,
        ExtendedCommandId::NewView,
        ExtendedCommandId::DashScaleListEdit,
        ExtendedCommandId::LineWeight,
        ExtendedCommandId::Color,
        ExtendedCommandId::CopyHist,
    ] {
        assert!(!ext.requires_work_mode(), "{ext:?} is Viewer-legal");
    }
    assert!(ExtendedCommandId::LayerStateSave.requires_work_mode());
}
