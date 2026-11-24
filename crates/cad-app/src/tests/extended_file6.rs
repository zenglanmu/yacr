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
fn dwgprops_reports_document_identity_and_an_empty_summary() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::DwgProps).expect("DWGPROPS is admitted");
    assert!(outcome.changes.is_none());
    let report = diagnostic(&outcome, "dwgprops.report");
    assert!(report.message.contains("document 1"), "{}", report.message);
    assert!(report.message.contains("revision 0"), "{}", report.message);
    assert!(report.message.contains("entities 0"), "{}", report.message);
    // Empty metadata is named with the real write grammar, never invented.
    assert!(diagnostic(&outcome, "dwgprops.empty")
        .message
        .contains("DWGPROPS TITLE"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dwgprops_writes_summary_fields_in_one_header_revision_and_reports_them() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    for (field, value) in [
        (cad_db::DocumentMetadataField::Title, "Pump station"),
        (cad_db::DocumentMetadataField::Author, "A. Drafter"),
        (cad_db::DocumentMetadataField::Keywords, "pump, station"),
    ] {
        let outcome = app
            .execute(
                &mut session,
                command(
                    CommandId::Extended(ExtendedCommandId::DwgProps),
                    CommandPayload::DwgProps(DwgPropsCommand::SetField {
                        field,
                        value: value.into(),
                    }),
                ),
            )
            .expect("a summary field writes");
        assert!(outcome.changes.is_some());
        assert!(diagnostic(&outcome, "dwgprops.set").message.contains(value));
        assert!(diagnostic(&outcome, "dwgprops.no_history")
            .message
            .contains("NO undo step"));
    }
    let custom = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::DwgProps),
                CommandPayload::DwgProps(DwgPropsCommand::SetCustom {
                    name: "Project".into(),
                    value: "North".into(),
                }),
            ),
        )
        .expect("a custom property writes");
    assert!(diagnostic(&custom, "dwgprops.set")
        .message
        .contains("Project"));
    assert_eq!(
        revision(&app).0,
        before.0 + 4,
        "each write is exactly one header revision"
    );
    assert!(
        !app.can_undo(&DocumentId(1)),
        "document metadata has no undo patch kind"
    );

    let report =
        execute(&mut app, &mut session, ExtendedCommandId::DwgProps).expect("DWGPROPS is admitted");
    assert!(diagnostic(&report, "dwgprops.field")
        .message
        .contains("Pump station"));
    assert!(diagnostic(&report, "dwgprops.custom")
        .message
        .contains("Project"));
    assert!(diagnostic(&report, "dwgprops.summary")
        .message
        .contains("4 metadata value(s)"));

    // The same write is an authoritative no-op: no revision, no change set.
    let stable = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::DwgProps),
                CommandPayload::DwgProps(DwgPropsCommand::SetField {
                    field: cad_db::DocumentMetadataField::Title,
                    value: "Pump station".into(),
                }),
            ),
        )
        .expect("an identical write is admitted");
    assert!(stable.changes.is_none());
    assert!(diagnostic(&stable, "dwgprops.unchanged")
        .message
        .contains("no revision"));
    assert_eq!(revision(&app).0, before.0 + 4);
}

#[test]
fn dwgprops_clears_a_named_field_and_a_custom_property() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::DwgProps),
            CommandPayload::DwgProps(DwgPropsCommand::SetField {
                field: cad_db::DocumentMetadataField::Title,
                value: "Temporary".into(),
            }),
        ),
    )
    .unwrap();
    app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::DwgProps),
            CommandPayload::DwgProps(DwgPropsCommand::SetCustom {
                name: "Temp".into(),
                value: "1".into(),
            }),
        ),
    )
    .unwrap();
    let cleared = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::DwgProps),
                CommandPayload::DwgProps(DwgPropsCommand::SetField {
                    field: cad_db::DocumentMetadataField::Title,
                    value: String::new(),
                }),
            ),
        )
        .unwrap();
    assert!(diagnostic(&cleared, "dwgprops.cleared")
        .message
        .contains("title"));
    let custom = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::DwgProps),
                CommandPayload::DwgProps(DwgPropsCommand::SetCustom {
                    name: "Temp".into(),
                    value: String::new(),
                }),
            ),
        )
        .unwrap();
    assert!(diagnostic(&custom, "dwgprops.cleared")
        .message
        .contains("removed"));
    let metadata = app.workspace.documents[&DocumentId(1)]
        .drawing
        .document_metadata();
    assert!(metadata.is_empty());
}

#[test]
fn dwgprops_metadata_round_trips_through_the_document_header() {
    let (mut app, mut session) = application_with_document();
    let expected = cad_db::DocumentMetadata {
        title: Some("Pump station".into()),
        author: Some("A. Drafter".into()),
        custom: [("Project".to_string(), "North".to_string())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    for (field, value) in [
        (cad_db::DocumentMetadataField::Title, "Pump station"),
        (cad_db::DocumentMetadataField::Author, "A. Drafter"),
    ] {
        app.execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::DwgProps),
                CommandPayload::DwgProps(DwgPropsCommand::SetField {
                    field,
                    value: value.into(),
                }),
            ),
        )
        .unwrap();
    }
    app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::DwgProps),
            CommandPayload::DwgProps(DwgPropsCommand::SetCustom {
                name: "Project".into(),
                value: "North".into(),
            }),
        ),
    )
    .unwrap();
    let document = &app.workspace.documents[&DocumentId(1)];
    assert_eq!(document.drawing.document_metadata(), &expected);
    let snapshot = document
        .drawing
        .snapshot(
            document.units.clone(),
            document.identity.clone(),
            document.resource_keys.clone(),
            document.drawing.revision(),
        )
        .expect("the header encodes");
    let loaded = cad_db::DrawingDatabase::from_snapshot(&snapshot).expect("the header decodes");
    assert_eq!(loaded.document_metadata(), &expected);
}

#[test]
fn dwgprops_refuses_a_write_in_viewer_mode_and_a_malformed_payload() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    let refused = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::DwgProps),
            CommandPayload::DwgProps(DwgPropsCommand::SetField {
                field: cad_db::DocumentMetadataField::Title,
                value: "Nope".into(),
            }),
        ),
    );
    assert!(refused.is_err(), "Viewer metadata writes are refused");
    assert_eq!(revision(&app), before);
    let malformed = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::DwgProps),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    assert!(malformed.is_err(), "DWGPROPS rejects another payload");
    assert_eq!(revision(&app), before);
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
