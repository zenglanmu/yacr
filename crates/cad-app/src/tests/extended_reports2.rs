//! Wave 2 read-only report contracts: `SYSVARMONITOR`, `SYSVDLG`, `VTOPTIONS`,
//! `ALIASEDIT`, `WHOHAS`, `ACADINFO`, `PTYPE`, `SCALELISTEDIT`.
//!
//! Every handler is a pure read: no transaction, no revision bump, no undo step
//! and no `ChangeSet`. Each test also pins the diagnostics that name the missing
//! sources, so a report can never silently invent a value.

use super::*;

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

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn diagnostic<'a>(outcome: &'a CommandOutcome, code: &str) -> &'a Diagnostic {
    outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("missing {code}"))
}

fn count_code(outcome: &CommandOutcome, code: &str) -> usize {
    outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .count()
}

#[test]
fn sysvarmonitor_reports_the_setvar_closed_set_only() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::SysVarMonitor)
        .expect("SYSVARMONITOR is admitted");
    assert!(outcome.changes.is_none());

    // Exactly the eight modelled variables, in the documented report order.
    assert_eq!(count_code(&outcome, "sysvarmonitor.value"), 8);
    let rows: Vec<&str> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "sysvarmonitor.value")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    for name in [
        "PDMODE", "PDSIZE", "SNAPUNIT", "LTSCALE", "GRID", "SNAP", "ORTHO", "TEXTSIZE",
    ] {
        assert!(
            rows.iter().any(|row| row.contains(name)),
            "SYSVARMONITOR must report {name}"
        );
    }
    assert!(diagnostic(&outcome, "sysvarmonitor.summary")
        .message
        .contains("not modeled"));
    assert_eq!(revision(&app), before, "SYSVARMONITOR writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn sysvdlg_reads_the_same_set_and_names_the_missing_dialog() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::SysVarDialog)
        .expect("SYSVDLG is admitted");
    assert_eq!(count_code(&outcome, "sysvdlg.value"), 8);
    assert_eq!(count_code(&outcome, "sysvarmonitor.value"), 0);
    assert!(diagnostic(&outcome, "sysvdlg.dialog_not_implemented")
        .message
        .contains("no system-variable dialog"));
    assert_eq!(revision(&app), before);
}

#[test]
fn vtopitions_declares_view_transitions_are_not_tracked() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::ViewTransitionOptions,
    )
    .expect("VTOPTIONS is admitted");
    let row = diagnostic(&outcome, "vtoptions.not_tracked");
    assert!(
        row.message
            .contains("view-transition settings are not modeled"),
        "{}",
        row.message
    );
    assert!(
        row.message.contains("none was invented"),
        "the report refuses to invent values: {}",
        row.message
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn aliasedit_lists_every_canonical_alias_row_without_editing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::AliasEdit)
        .expect("ALIASEDIT is admitted");

    // The report is the canonical table, not a hand-written mirror.
    let expected = crate::command_aliases::COMMAND_ALIASES.len();
    assert_eq!(count_code(&outcome, "aliasedit.alias"), expected);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "aliasedit.alias"
            && diagnostic.message.contains("EXPORT -> PLOT")));
    let summary = diagnostic(&outcome, "aliasedit.summary");
    assert!(
        summary.message.contains("NOT IMPLEMENTED"),
        "editing is declared missing: {}",
        summary.message
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn whohas_reports_a_single_document_build() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::WhoHas).expect("WHOHAS is admitted");
    assert!(diagnostic(&outcome, "whohas.single_document")
        .message
        .contains("single-document build"));
    assert_eq!(revision(&app), before);
}

#[test]
fn acadinfo_reports_real_product_and_document_counts_only() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::AcadInfo).expect("ACADINFO is admitted");
    let row = diagnostic(&outcome, "acadinfo.report");
    assert!(
        row.message.contains(env!("CARGO_PKG_VERSION")),
        "{}",
        row.message
    );
    assert!(row.message.contains("revision"), "{}", row.message);
    assert!(row.message.contains("entities"), "{}", row.message);
    assert!(diagnostic(&outcome, "acadinfo.not_collected")
        .message
        .contains("not collected"));
    assert_eq!(revision(&app), before);
}

#[test]
fn ptype_reports_the_point_display_style_read_only() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::PointType).expect("PTYPE is admitted");
    let row = diagnostic(&outcome, "ptype.report");
    assert!(row.message.contains("$PDMODE"), "{}", row.message);
    assert!(row.message.contains("$PDSIZE"), "{}", row.message);
    assert!(
        row.message.contains("PDMODE/PDSIZE"),
        "the write path is named: {}",
        row.message
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn scalelistedit_reports_the_page_and_refuses_the_write() {
    let (mut app, mut session) = application_with_layouts();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::ScaleListEdit)
        .expect("SCALELISTEDIT is admitted");
    let summary = diagnostic(&outcome, "scalelistedit.summary");
    assert!(summary.message.contains("Sheet1"), "{}", summary.message);
    assert!(summary.message.contains("read-only"), "{}", summary.message);
    assert!(
        diagnostic(&outcome, "scalelistedit.write_unsupported")
            .message
            .contains("no scale-list storage"),
        "the missing write API is named"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_report_payload_is_refused_and_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let refused = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::SysVarMonitor),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    assert!(refused.is_err(), "SYSVARMONITOR takes no payload");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}
