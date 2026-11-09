//! Wave 6 file/plot/export contracts: `DWGLOG`, `LOGFILEON`, `LOGFILEOFF`,
//! `DWGPROPS`, `DWFFORMAT`, `EXPORTPDF`.
//!
//! The three log commands flip one session token and write nothing (this build
//! has no log file). The three reports are pure reads: `DWGPROPS` reads the
//! document/revision/entity count, while `DWFFORMAT` and `EXPORTPDF` are
//! read-only capability reports that name the absent export paths. Each test
//! pins the diagnostic codes so a report can never silently invent a value, and
//! asserts no revision bump, no undo step and no `ChangeSet`.

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

#[test]
fn dwglog_toggles_a_session_token_and_writes_no_log_file() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    assert!(!session.drafting_extras.dwglog(), "DWGLOG defaults OFF");

    let on =
        execute(&mut app, &mut session, ExtendedCommandId::DwgLog).expect("DWGLOG is admitted");
    assert!(on.changes.is_none());
    assert!(session.drafting_extras.dwglog());
    let message = &diagnostic(&on, "dwglog.toggled").message;
    assert!(message.contains("ON"), "{message}");
    assert!(message.contains("no log file"), "{message}");
    assert!(message.contains("not written"), "{message}");

    let off =
        execute(&mut app, &mut session, ExtendedCommandId::DwgLog).expect("DWGLOG is admitted");
    assert!(!session.drafting_extras.dwglog());
    assert!(diagnostic(&off, "dwglog.toggled").message.contains("OFF"));

    assert_eq!(revision(&app), before, "DWGLOG writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn logfileon_and_logfileoff_set_the_same_token_with_their_own_codes() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);

    let on = execute(&mut app, &mut session, ExtendedCommandId::LogFileOn)
        .expect("LOGFILEON is admitted");
    assert!(on.changes.is_none());
    assert!(session.drafting_extras.dwglog());
    assert!(diagnostic(&on, "logfileon.toggled")
        .message
        .contains("no log file"));
    assert_eq!(
        on.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "logfileon.toggled")
            .count(),
        1
    );

    // LOGFILEON is idempotent: setting an already-ON token changes no row.
    let again = execute(&mut app, &mut session, ExtendedCommandId::LogFileOn)
        .expect("LOGFILEON is admitted twice");
    assert!(session.drafting_extras.dwglog());
    assert!(diagnostic(&again, "logfileon.toggled")
        .message
        .contains("ON"));

    let off = execute(&mut app, &mut session, ExtendedCommandId::LogFileOff)
        .expect("LOGFILEOFF is admitted");
    assert!(!session.drafting_extras.dwglog());
    assert!(diagnostic(&off, "logfileoff.toggled")
        .message
        .contains("OFF"));

    assert_eq!(revision(&app), before, "the log tokens write no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dwgprops_reports_document_identity_and_names_the_metadata_gap() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::DwgProps).expect("DWGPROPS is admitted");
    assert!(outcome.changes.is_none());
    let report = diagnostic(&outcome, "dwgprops.report");
    assert!(report.message.contains("document 1"), "{}", report.message);
    assert!(report.message.contains("revision 0"), "{}", report.message);
    assert!(report.message.contains("entities 0"), "{}", report.message);
    assert!(diagnostic(&outcome, "dwgprops.metadata_not_tracked")
        .message
        .contains("NOT MODELED"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dwfformat_is_a_read_only_report_naming_plot_as_the_vector_path() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::DwfFormat)
        .expect("DWFFORMAT is a report, not an error");
    assert!(outcome.changes.is_none());
    let report = diagnostic(&outcome, "dwfformat.not_supported");
    assert!(report.message.contains("DWF"), "{}", report.message);
    assert!(
        report.message.contains("SVG/PDF via PLOT"),
        "{}",
        report.message
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn exportpdf_is_a_read_only_report_pointing_at_host_owned_plot() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::ExportPdf)
        .expect("EXPORTPDF is a report, not an error");
    assert!(outcome.changes.is_none());
    let report = diagnostic(&outcome, "exportpdf.use_plot");
    assert!(report.message.contains("host-owned"), "{}", report.message);
    assert!(report.message.contains("PLOT"), "{}", report.message);
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_wave6_report_payload_is_refused_and_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let refused = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::DwgProps),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    assert!(refused.is_err(), "DWGPROPS takes no payload");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}
