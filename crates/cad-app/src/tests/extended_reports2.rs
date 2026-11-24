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
fn sysvar_dialog_lists_every_exposed_variable_and_marks_the_rest_not_tracked() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::SysVarDialog)
        .expect("SYSVDLG is admitted");
    // Eleven pre-existing values plus the session tokens `ELEV`/`FSMODE` and
    // `EXPLMODE`/`EXPMODE` (modelled, always have a value, and are listed like
    // every other modelled variable). `CVPORT`/`INSBASE`/`LIMMIN`/`LIMMAX` stay
    // `not_stored` in the demo document, so they add no `sysvdlg.value` row.
    assert_eq!(count_code(&outcome, "sysvdlg.value"), 13);
    assert_eq!(count_code(&outcome, "sysvarmonitor.value"), 0);
    for name in [
        "PDMODE",
        "PDSIZE",
        "SNAPUNIT",
        "LTSCALE",
        "GRID",
        "SNAP",
        "ORTHO",
        "TEXTSIZE",
        "WIPEOUTFRAME",
        "ELEV",
        "FSMODE",
        "EXPLMODE",
        "EXPMODE",
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
        .contains("no interactive dialog"));
    assert_eq!(revision(&app), before);
}

#[test]
fn sysvar_dialog_reads_one_variable_and_refuses_an_unknown_name() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let read = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SysVarDialog),
                CommandPayload::SysVarDialog(SysVarDialogCommand::Read("pdmode".into())),
            ),
        )
        .expect("SYSVDLG reads one modelled variable");
    assert!(diagnostic(&read, "sysvdlg.value")
        .message
        .contains("PDMODE = 0"));
    assert_eq!(revision(&app), before);

    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SysVarDialog),
                CommandPayload::SysVarDialog(SysVarDialogCommand::Read("PLOTSTYLE".into())),
            ),
        )
        .expect_err("an unmodelled name is refused by name");
    assert!(
        error.to_string().contains("sysvdlg.unknown_variable"),
        "{error}"
    );
}

#[test]
fn sysvar_dialog_applies_header_and_session_values() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SysVarDialog),
                CommandPayload::SysVarDialog(SysVarDialogCommand::Apply(vec![
                    SysVarAssignment {
                        name: "PDMODE".into(),
                        value: "3".into(),
                    },
                    SysVarAssignment {
                        name: "PDSIZE".into(),
                        value: "2.5".into(),
                    },
                    SysVarAssignment {
                        name: "SNAPUNIT".into(),
                        value: "25".into(),
                    },
                    SysVarAssignment {
                        name: "GRID".into(),
                        value: "OFF".into(),
                    },
                ])),
            ),
        )
        .expect("a mixed apply is admitted");
    assert!(outcome.changes.is_some());
    assert_eq!(revision(&app).0, before.0 + 1, "one header revision");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.point_display_mode(), 3);
    assert_eq!(drawing.point_display_size(), 2.5);
    assert_eq!(session.snap_spacing(), 25.0);
    assert!(!session.grid_enabled());
    assert_eq!(
        outcome
            .diagnostics
            .iter()
            .filter(|row| row.code == "sysvdlg.applied")
            .count(),
        4
    );
    assert!(diagnostic(&outcome, "sysvdlg.no_history")
        .message
        .contains("no undo step"));
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn sysvar_dialog_restores_header_values_in_one_revision() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SysVarDialog),
                CommandPayload::SysVarDialog(SysVarDialogCommand::Apply(vec![
                    SysVarAssignment {
                        name: "PDMODE".into(),
                        value: "35".into(),
                    },
                    SysVarAssignment {
                        name: "PDSIZE".into(),
                        value: "-3".into(),
                    },
                    SysVarAssignment {
                        name: "LTSCALE".into(),
                        value: "2.5".into(),
                    },
                    SysVarAssignment {
                        name: "INSBASE".into(),
                        value: "1,2,3".into(),
                    },
                    SysVarAssignment {
                        name: "LIMMIN".into(),
                        value: "0,0".into(),
                    },
                    SysVarAssignment {
                        name: "LIMMAX".into(),
                        value: "420,297".into(),
                    },
                ])),
            ),
        )
        .expect("a header restore is admitted");
    assert!(outcome.changes.is_some());
    assert_eq!(
        revision(&app).0,
        before.0 + 1,
        "all header values land in one revision"
    );
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.point_display_mode(), 35);
    assert_eq!(drawing.point_display_size(), -3.0);
    assert_eq!(drawing.linetype_scale(), 2.5);
    assert_eq!(
        drawing.insertion_base().map(|p| (p.x, p.y, p.z)),
        Some((1.0, 2.0, 3.0))
    );
    assert_eq!(drawing.limits_min(), Some([0.0, 0.0]));
    assert_eq!(drawing.limits_max(), Some([420.0, 297.0]));
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn sysvar_dialog_rejects_the_whole_apply_before_any_write() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    for assignments in [
        vec![SysVarAssignment {
            name: "NOT_A_VAR".into(),
            value: "1".into(),
        }],
        vec![
            SysVarAssignment {
                name: "PDMODE".into(),
                value: "1".into(),
            },
            SysVarAssignment {
                name: "PDMODE".into(),
                value: "2".into(),
            },
        ],
        vec![
            SysVarAssignment {
                name: "PDMODE".into(),
                value: "1".into(),
            },
            SysVarAssignment {
                name: "LTSCALE".into(),
                value: "0".into(),
            },
        ],
    ] {
        let error = app
            .execute(
                &mut session,
                command(
                    CommandId::Extended(ExtendedCommandId::SysVarDialog),
                    CommandPayload::SysVarDialog(SysVarDialogCommand::Apply(assignments)),
                ),
            )
            .expect_err("a bad apply is refused as a whole");
        assert!(error.to_string().contains("sysvdlg."), "{error}");
        assert_eq!(revision(&app), before, "nothing was written");
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .point_display_mode(),
            0
        );
    }
}

#[test]
fn sysvar_dialog_refuses_a_viewer_apply_and_model_space_cvport() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let cvport = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::SysVarDialog),
                CommandPayload::SysVarDialog(SysVarDialogCommand::Apply(vec![
                    SysVarAssignment {
                        name: "PDMODE".into(),
                        value: "1".into(),
                    },
                    SysVarAssignment {
                        name: "CVPORT".into(),
                        value: "1".into(),
                    },
                ])),
            ),
        )
        .expect_err("CVPORT in model space is refused before the PDMODE write");
    assert!(
        cvport.to_string().contains("sysvdlg.cvport_requires_paper"),
        "{cvport}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .point_display_mode(),
        0
    );

    session.switch_mode(AppMode::Viewer).unwrap();
    let viewer = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::SysVarDialog),
            CommandPayload::SysVarDialog(SysVarDialogCommand::Apply(vec![SysVarAssignment {
                name: "PDMODE".into(),
                value: "1".into(),
            }])),
        ),
    );
    assert!(viewer.is_err(), "Viewer may not apply variables");
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
            && diagnostic.message.contains("PRINT -> PLOT")));
    let summary = diagnostic(&outcome, "aliasedit.summary");
    assert!(
        summary.message.contains("root commandAliases preference"),
        "the durable runtime rows are named: {}",
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
    assert!(
        row.message.contains(std::env::consts::OS),
        "the real target OS is reported: {}",
        row.message
    );
    assert!(
        row.message.contains(std::env::consts::ARCH),
        "the real target architecture is reported: {}",
        row.message
    );
    // The report reads the live document header, not a constant.
    assert!(
        row.message
            .contains("units drawing units (3 decimal place(s))"),
        "{}",
        row.message
    );
    assert!(diagnostic(&outcome, "acadinfo.not_collected")
        .message
        .contains("not collected"));
    // The one non-global gap is named: no information file is written.
    assert!(diagnostic(&outcome, "acadinfo.file_not_written")
        .message
        .contains("writes no file"));
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
fn scalelistedit_reports_the_page_and_the_real_write_grammar() {
    let (mut app, mut session) = application_with_layouts();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::ScaleListEdit)
        .expect("SCALELISTEDIT is admitted");
    let summary = diagnostic(&outcome, "scalelistedit.summary");
    assert!(summary.message.contains("Sheet1"), "{}", summary.message);
    // The write is real: the report names the grammar instead of claiming the
    // table cannot be edited. This document has no ACAD_SCALELIST row, and the
    // report says so rather than inventing one.
    assert!(
        diagnostic(&outcome, "scalelistedit.grammar")
            .message
            .contains("ADD <name> <paper_units> <drawing_units>"),
        "the real write forms are named"
    );
    assert!(
        diagnostic(&outcome, "scalelistedit.empty")
            .message
            .contains("empty"),
        "an empty table is named, not fabricated"
    );
    assert!(!outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "scalelistedit.write_unsupported"));
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
