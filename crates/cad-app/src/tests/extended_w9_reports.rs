//! Wave 9 report/refusal contracts: `UNITS`, `DIMEX`, `DIMIM`, `SPACETRANS`,
//! `TXTEXP`, `UPDATEFIELD` and the `DIM` router.
//!
//! `UNITS` and `DIM` are read-only reports; the rest are stable named refusals.
//! Neither class writes the drawing, bumps the revision or records history, so
//! both classes stay Viewer-legal.

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
fn units_reports_the_fixed_display_rule_without_writing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::Units).expect("UNITS is admitted");
    assert!(outcome.changes.is_none());
    let report = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "units.report")
        .expect("UNITS reports the display rule");
    assert!(report.message.contains("4 decimal places"));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "units.dialog_not_implemented"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dim_lists_the_implemented_dimension_selectors_without_creating_one() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::DimRouter).expect("DIM is admitted");
    assert!(outcome.changes.is_none());
    let names: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|row| row.code == "dim.available")
        .map(|row| row.message.clone())
        .collect();
    assert!(names.iter().any(|row| row.contains("DIMLINEAR")));
    assert!(names.iter().any(|row| row.contains("QDIM")));
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "dim.router_only"));
    assert_eq!(revision(&app), before);
}

const REFUSALS: [(ExtendedCommandId, &str); 5] = [
    (
        ExtendedCommandId::DimExport,
        "dimex.unsupported: dimension-style export files are not modeled",
    ),
    (
        ExtendedCommandId::DimImport,
        "dimim.unsupported: dimension-style import files are not modeled",
    ),
    (
        ExtendedCommandId::SpaceTrans,
        "spacetrans.unsupported: no model/paper distance-conversion API",
    ),
    (
        ExtendedCommandId::TextExplode,
        "txtexp.unsupported: no font-outline-to-geometry path is implemented",
    ),
    (
        ExtendedCommandId::UpdateField,
        "updatefield.unsupported: fields are not modeled",
    ),
];

#[test]
fn wave9_report_refusals_answer_with_their_exact_stable_reason() {
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
fn wave9_reports_and_refusals_are_viewer_legal() {
    for ext in [ExtendedCommandId::Units, ExtendedCommandId::DimRouter] {
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is legal");
        execute(&mut app, &mut session, ext)
            .unwrap_or_else(|error| panic!("{ext:?} is Viewer-legal: {error:?}"));
    }
    for ext in [
        ExtendedCommandId::DimExport,
        ExtendedCommandId::DimImport,
        ExtendedCommandId::SpaceTrans,
        ExtendedCommandId::TextExplode,
        ExtendedCommandId::UpdateField,
    ] {
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is legal");
        let error = execute(&mut app, &mut session, ext)
            .expect_err("the feature is still missing in Viewer mode");
        match error {
            CadError::Unsupported(reason) => {
                assert!(reason.contains("unsupported"), "{ext:?}: {reason}")
            }
            other => panic!("{ext:?} must reach its named refusal, got {other:?}"),
        }
    }
}
