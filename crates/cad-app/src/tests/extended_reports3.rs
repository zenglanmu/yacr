//! Wave 3 report/session contracts: `MSPACE`, `PSPACE`, `STYLE`, `-STYLE`,
//! `MLEADERSTYLE`, `BLOCK?`, `DIMREASSOC`, `DIMREASSOCIATE`.
//!
//! The two space switches must reuse the existing `MODEL`/`SPACE` session path
//! (session state only). The reports are pure reads: no transaction, no revision
//! bump, no undo step and no `ChangeSet`, and each test pins the diagnostic codes
//! so a report can never silently invent a row.

use super::*;
use cad_db::Style;

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

/// A drawing whose text-style table has one real row (`Note`).
fn application_with_text_style() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_style(Style {
        id: StyleId(1),
        name: "Note".into(),
        resource_keys: vec!["note.shx".into()],
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

#[test]
fn mspace_and_pspace_reuse_the_existing_space_switch_path() {
    let (mut app, mut session) = application_with_layouts();
    let before = revision(&app);
    assert_eq!(session.active_space, SpaceId::Model);

    // Already in model space: a named no-op, not a second switch.
    let noop =
        execute(&mut app, &mut session, ExtendedCommandId::Mspace).expect("MSPACE is admitted");
    assert!(diagnostic(&noop, "mspace.noop")
        .message
        .contains("already in model space"));
    assert_eq!(session.active_space, SpaceId::Model);

    // PSPACE lands on the first paper layout (`Sheet1`, LayoutId(1)).
    let paper =
        execute(&mut app, &mut session, ExtendedCommandId::Pspace).expect("PSPACE is admitted");
    assert!(diagnostic(&paper, "pspace.switched")
        .message
        .contains("Sheet1"));
    assert_eq!(session.active_space, SpaceId::Paper(LayoutId(1)));

    // MSPACE returns to model space through the same path.
    let model =
        execute(&mut app, &mut session, ExtendedCommandId::Mspace).expect("MSPACE is admitted");
    assert!(diagnostic(&model, "mspace.switched")
        .message
        .contains("model space"));
    assert_eq!(session.active_space, SpaceId::Model);

    assert_eq!(revision(&app), before, "space switches write no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn pspace_without_a_paper_layout_is_a_named_noop() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::Pspace).expect("PSPACE is admitted");
    assert!(diagnostic(&outcome, "pspace.no_layout")
        .message
        .contains("no paper layout"));
    assert_eq!(session.active_space, SpaceId::Model, "nothing changed");
    assert_eq!(revision(&app), before);
}

#[test]
fn style_and_dash_style_list_the_same_table_under_their_own_codes() {
    let (mut app, mut session) = application_with_text_style();
    let before = revision(&app);

    let style =
        execute(&mut app, &mut session, ExtendedCommandId::TextStyle).expect("STYLE is admitted");
    assert!(style.changes.is_none());
    assert_eq!(count_code(&style, "style.entry"), 1);
    let entry = diagnostic(&style, "style.entry");
    assert!(entry.message.contains("Note"), "{}", entry.message);
    assert!(diagnostic(&style, "style.summary")
        .message
        .contains("write forms are NEW/RENAME/DELETE/SET"));
    assert_eq!(count_code(&style, "-style.entry"), 0);

    let dash =
        execute(&mut app, &mut session, ExtendedCommandId::DashStyle).expect("-STYLE is admitted");
    assert_eq!(count_code(&dash, "-style.entry"), 1);
    assert_eq!(count_code(&dash, "style.entry"), 0);
    assert!(diagnostic(&dash, "-style.summary")
        .message
        .contains("1 text style(s)"));

    assert_eq!(revision(&app), before, "style reports write no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn mleaderstyle_lists_the_standard_row_read_only() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::MLeaderStyleList)
        .expect("MLEADERSTYLE is admitted");
    assert_eq!(count_code(&outcome, "mleaderstyle.entry"), 1);
    assert!(diagnostic(&outcome, "mleaderstyle.entry")
        .message
        .contains("Standard"));
    assert!(diagnostic(&outcome, "mleaderstyle.summary")
        .message
        .contains("SET makes"));
    assert_eq!(revision(&app), before);
}

#[test]
fn block_list_reports_definitions_and_a_read_only_summary() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::BlockList).expect("BLOCK? is admitted");
    assert_eq!(
        count_code(&outcome, "block.entry"),
        0,
        "no block is invented"
    );
    let summary = diagnostic(&outcome, "block.summary");
    assert!(
        summary.message.contains("0 block definition(s)"),
        "{}",
        summary.message
    );
    assert!(
        summary.message.contains("not the block editor"),
        "{}",
        summary.message
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn dimreassoc_and_dimreassociate_are_named_non_associative_noops() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    for (ext, code) in [
        (ExtendedCommandId::DimReassoc, "dimreassoc.non_associative"),
        (
            ExtendedCommandId::DimReassociate,
            "dimreassociate.non_associative",
        ),
    ] {
        let outcome = execute(&mut app, &mut session, ext).expect("report is admitted");
        assert!(outcome.changes.is_none());
        assert!(diagnostic(&outcome, code)
            .message
            .contains("nothing changed"));
    }
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
            CommandId::Extended(ExtendedCommandId::TextStyle),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    assert!(refused.is_err(), "STYLE takes no payload");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}
