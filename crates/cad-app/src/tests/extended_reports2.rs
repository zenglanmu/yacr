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
fn sysvarmonitor_adds_lists_and_reports_a_real_change() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let added = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SysVarMonitor),
                CommandPayload::SysVarMonitor(SysVarMonitorAction::Add("SNAPUNIT".into())),
            ),
        )
        .expect("SYSVARMONITOR ADD is admitted");
    assert!(added.changes.is_none());
    assert!(diagnostic(&added, "sysvarmonitor.added")
        .message
        .contains("SNAPUNIT = 10"));

    // The bare selector is the LIST form. Nothing changed yet.
    let first = execute(&mut app, &mut session, ExtendedCommandId::SysVarMonitor)
        .expect("SYSVARMONITOR is admitted");
    assert_eq!(count_code(&first, "sysvarmonitor.value"), 1);
    assert!(diagnostic(&first, "sysvarmonitor.value")
        .message
        .contains("unchanged since the snapshot"));

    // A real session-value change is reported against the snapshot.
    app.execute(
        &mut session,
        command(CommandId::SetSnapSpacing, CommandPayload::SnapSpacing(25.0)),
    )
    .expect("SNAPUNIT writes the session spacing");
    let changed = execute(&mut app, &mut session, ExtendedCommandId::SysVarMonitor)
        .expect("SYSVARMONITOR is admitted");
    let row = diagnostic(&changed, "sysvarmonitor.changed");
    assert!(row.message.contains("from 10 to 25"), "{}", row.message);
    assert!(diagnostic(&changed, "sysvarmonitor.summary")
        .message
        .contains("1 change(s)"));

    // The snapshot was updated, so the next query is the unchanged row again.
    let stable = execute(&mut app, &mut session, ExtendedCommandId::SysVarMonitor)
        .expect("SYSVARMONITOR is admitted");
    assert_eq!(count_code(&stable, "sysvarmonitor.changed"), 0);
    assert_eq!(count_code(&stable, "sysvarmonitor.value"), 1);

    let removed = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SysVarMonitor),
                CommandPayload::SysVarMonitor(SysVarMonitorAction::Remove("SNAPUNIT".into())),
            ),
        )
        .expect("SYSVARMONITOR REMOVE is admitted");
    assert!(diagnostic(&removed, "sysvarmonitor.removed")
        .message
        .contains("no longer monitored"));

    let empty = execute(&mut app, &mut session, ExtendedCommandId::SysVarMonitor)
        .expect("SYSVARMONITOR is admitted");
    assert!(diagnostic(&empty, "sysvarmonitor.no_variables")
        .message
        .contains("no variables are monitored"));
    assert_eq!(revision(&app), before, "SYSVARMONITOR writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn sysvarmonitor_refuses_an_unknown_variable_by_name() {
    let (mut app, mut session) = application_with_document();
    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SysVarMonitor),
                CommandPayload::SysVarMonitor(SysVarMonitorAction::Add("PLOTSTYLE".into())),
            ),
        )
        .expect_err("an unmodelled name is refused");
    assert!(
        error.to_string().contains("sysvarmonitor.unknown_variable"),
        "{error}"
    );
    let refused = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::SysVarMonitor),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    assert!(refused.is_err(), "SYSVARMONITOR rejects another payload");
}

#[test]
fn sysvdlg_lists_every_exposed_variable_and_marks_the_rest_not_tracked() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::SysVarDialog)
        .expect("SYSVDLG is admitted");
    assert_eq!(count_code(&outcome, "sysvdlg.value"), 8);
    assert_eq!(count_code(&outcome, "sysvarmonitor.value"), 0);
    for name in [
        "PDMODE", "PDSIZE", "SNAPUNIT", "LTSCALE", "GRID", "SNAP", "ORTHO", "TEXTSIZE",
    ] {
        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "sysvdlg.value"
                    && diagnostic.message.contains(name)),
            "SYSVDLG must report {name}"
        );
    }
    assert!(diagnostic(&outcome, "sysvdlg.not_tracked")
        .message
        .contains("not tracked"));
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
