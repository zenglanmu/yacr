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
    let report = row(&outcome, "layer.report_only");
    assert!(
        report.message.contains("read-only") && report.message.contains("NEW"),
        "the report names the options that write: {}",
        report.message
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dash_layer_question_filters_by_cad_wildcard() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashLayer,
        CommandPayload::DashLayer(crate::DashLayerOps::List {
            pattern: Some("w*".into()),
        }),
    )
    .expect("-LAYER ? <pattern> is the read-only report");
    let entries: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|row| row.code == "layer.entry")
        .collect();
    assert!(
        entries.iter().any(|row| row.message.contains("'WALLS'")),
        "case-insensitive wildcard matches WALLS: {entries:?}"
    );
    assert!(
        !entries.iter().any(|row| row.message.contains("'0'")),
        "the filter excludes non-matching rows: {entries:?}"
    );
    assert!(!outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "layer.no_match"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dash_layer_question_with_no_match_reports_by_name() {
    let (mut app, mut session) = application_with_document();
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashLayer,
        CommandPayload::DashLayer(crate::DashLayerOps::List {
            pattern: Some("zzz*".into()),
        }),
    )
    .expect("an empty report is not an error");
    assert!(!outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "layer.entry"));
    assert!(row(&outcome, "layer.no_match").message.contains("zzz*"));
}

#[test]
fn dash_layer_list_cannot_ride_the_write_carrier() {
    let (mut app, mut session) = application_with_document();
    let error = app
        .execute(
            &mut session,
            command(
                CommandId::SetLayerProperty,
                CommandPayload::DashLayer(crate::DashLayerOps::List { pattern: None }),
            ),
        )
        .expect_err("the list form is the report, not a write");
    assert!(format!("{error}").contains("dashlayer.payload"), "{error}");
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
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashScaleListEdit,
        CommandPayload::None,
    )
    .expect("-SCALELISTEDIT is admitted");
    assert!(outcome.changes.is_none());
    // A hand-built document has no ACAD_SCALELIST row: the report names that
    // instead of inventing one, and the grammar line names the real write forms.
    assert!(row(&outcome, "scalelistedit.empty")
        .message
        .contains("ADD <name> <paper_units> <drawing_units>"));
    assert!(row(&outcome, "scalelistedit.grammar")
        .message
        .contains("RESET"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
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

// ---------------------------------------------------------------------------
// `-SCALELISTEDIT` writes (the annotation-scale table)
// ---------------------------------------------------------------------------

/// A database with two imported rows and the `CANNOSCALE` value `1:100`.
fn application_with_scale_table() -> (Application, SessionState) {
    let document_id = DocumentId(1);
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for (index, (name, paper, drawing)) in [("1:1", 1.0, 1.0), ("1:100", 1.0, 100.0)]
        .into_iter()
        .enumerate()
    {
        builder
            .insert_scale(cad_db::Scale {
                id: ScaleId(index as u128),
                name: name.into(),
                paper_units: paper,
                drawing_units: drawing,
            })
            .unwrap();
    }
    builder.set_active_annotation_scale("1:100", 0.01).unwrap();
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        document_id,
        Document {
            id: document_id,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([0u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), document_id, [800.0, 600.0]),
    );
    let session = SessionState::new(document_id, AppMode::Work);
    (app, session)
}

#[test]
fn dash_scalelistedit_adds_a_row_in_one_revision() {
    let (mut app, mut session) = application_with_scale_table();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashScaleListEdit,
        CommandPayload::ScaleList(ScaleListCommand::Add {
            name: "1:200".into(),
            paper_units: 1.0,
            drawing_units: 200.0,
        }),
    )
    .expect("ADD writes one row");
    assert!(row(&outcome, "scalelistedit.added")
        .message
        .contains("1:200"));
    assert!(row(&outcome, "scalelistedit.no_history")
        .message
        .contains("NO undo step"));
    assert_eq!(revision(&app).0, before.0 + 1);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.scale_by_name("1:200").is_some());
    assert_eq!(drawing.scales().count(), 3);
    // The write is not undoable, and the command says so; no history entry.
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dash_scalelistedit_renames_a_row_and_cascades_the_cannoscale_value() {
    let (mut app, mut session) = application_with_scale_table();
    execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashScaleListEdit,
        CommandPayload::ScaleList(ScaleListCommand::Rename {
            old: "1:100".into(),
            new: "1:150".into(),
        }),
    )
    .expect("RENAME writes the row");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.scale_by_name("1:100").is_none());
    assert!(drawing.scale_by_name("1:150").is_some());
    // The header reference followed the row: the active scale still resolves.
    assert_eq!(drawing.annotation_scale_name(), Some("1:150"));
    assert!(drawing.annotation_scale().2);
}

#[test]
fn dash_scalelistedit_refuses_to_delete_a_referenced_scale() {
    let (mut app, mut session) = application_with_scale_table();
    let before = revision(&app);
    let refused = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashScaleListEdit,
        CommandPayload::ScaleList(ScaleListCommand::Delete {
            name: "1:100".into(),
        }),
    );
    match refused {
        Err(CadError::InvalidInput(message)) => {
            assert!(message.contains("scalelistedit.in_use"), "{message}");
            assert!(message.contains("CANNOSCALE"), "{message}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .scale_by_name("1:100")
        .is_some());
}

#[test]
fn dash_scalelistedit_deletes_an_unreferenced_row() {
    let (mut app, mut session) = application_with_scale_table();
    let before = revision(&app);
    execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashScaleListEdit,
        CommandPayload::ScaleList(ScaleListCommand::Delete { name: "1:1".into() }),
    )
    .expect("DELETE removes an unreferenced row");
    assert_eq!(revision(&app).0, before.0 + 1);
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .scale_by_name("1:1")
        .is_none());
}

#[test]
fn dash_scalelistedit_rename_to_the_stored_name_is_an_authoritative_no_op() {
    let (mut app, mut session) = application_with_scale_table();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashScaleListEdit,
        CommandPayload::ScaleList(ScaleListCommand::Rename {
            old: "1:100".into(),
            new: "1:100".into(),
        }),
    )
    .expect("renaming to the stored spelling is not an error");
    assert_eq!(revision(&app), before, "nothing changed, so nothing bumped");
    assert!(
        row(&outcome, "scalelistedit.renamed")
            .message
            .contains("already matched"),
        "the report must say the write was a no-op: {:?}",
        outcome.diagnostics
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dash_scalelistedit_writes_are_refused_in_viewer_mode() {
    let (mut app, mut session) = application_with_scale_table();
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    let refused = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashScaleListEdit,
        CommandPayload::ScaleList(ScaleListCommand::Add {
            name: "1:50".into(),
            paper_units: 1.0,
            drawing_units: 50.0,
        }),
    );
    assert!(
        matches!(refused, Err(CadError::PermissionDenied)),
        "a Viewer session may not edit the table, got {refused:?}"
    );
    assert_eq!(revision(&app), before);
    // The report itself stays Viewer-legal.
    execute(
        &mut app,
        &mut session,
        ExtendedCommandId::DashScaleListEdit,
        CommandPayload::None,
    )
    .expect("the read-only report is Viewer-legal");
}
