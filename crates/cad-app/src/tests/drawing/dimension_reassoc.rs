//! `DIMREASSOC` contracts: restore the measured dimension text.
//!
//! The command's whole support domain is the stored text override
//! (`DimensionGeometry::text_override`, the same field `DIMEDIT TEXT`/`RESET`
//! write), so these tests pin:
//!
//! * every selected dimension with an override is restored in **one transaction /
//!   one undo step**, and undo brings the override back;
//! * a dimension without an override is named (`dimreassoc.no_override`) and is
//!   not rewritten;
//! * a non-dimension selection refuses the whole batch **before any write**, even
//!   when another selected dimension was writable;
//! * an empty selection and a payload are named refusals, and Viewer mode refuses
//!   the Work-only command;
//! * association to source objects is not claimed (ADR global scope).

use super::dimension_edit::{
    dimension_edit_app, edit_payload, select, stored, undo_depth, DIM_A, DIM_B,
};
use super::*;

/// Run `DIMREASSOC` (payload-free).
fn reassoc(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Extended(ExtendedCommandId::DimReassoc),
        CommandPayload::None,
    )
}

fn set_override(app: &mut Application, session: &mut SessionState, text: &str) {
    execute(
        app,
        session,
        CommandId::DimEdit,
        edit_payload(DimEditCommand::Text(text.into())),
    )
    .expect("DIMEDIT TEXT writes the override");
}

#[test]
fn dimreassoc_restores_every_override_in_one_undo_step() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A, DIM_B]);
    set_override(&mut app, &mut session, "<> mm");
    assert_eq!(stored(&app, DIM_A).text_override(), Some("<> mm"));
    assert_eq!(stored(&app, DIM_B).text_override(), Some("<> mm"));
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    let out = reassoc(&mut app, &mut session).expect("DIMREASSOC is admitted");
    assert_eq!(out.objects.len(), 2, "both dimensions are rewritten");
    assert!(out.changes.is_some(), "one ChangeSet");
    assert_eq!(revision(&app).0, before_revision.0 + 1);
    assert_eq!(undo_depth(&app), before_undo + 1, "one undo step");
    assert_eq!(stored(&app, DIM_A).text_override(), None);
    assert_eq!(stored(&app, DIM_B).text_override(), None);
    assert!(
        (stored(&app, DIM_A).measurement() - 10.0).abs() < 1e-9,
        "the measured value is untouched"
    );

    // The summary names the batch, and each restored entity is named by id.
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "dimreassoc.applied" && d.message.contains("2 dimension(s)")));
    let restored: Vec<_> = out
        .diagnostics
        .iter()
        .filter(|d| d.code == "dimreassoc.restored")
        .collect();
    assert_eq!(restored.len(), 2);
    for diagnostic in restored {
        let object = diagnostic.object.expect("a restored entity is named");
        assert!(
            object == ObjectId(DIM_A) || object == ObjectId(DIM_B),
            "{object:?}"
        );
    }

    // Undo puts the override back: the restore is a real, reversible entity write.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert_eq!(stored(&app, DIM_A).text_override(), Some("<> mm"));
    assert_eq!(stored(&app, DIM_B).text_override(), Some("<> mm"));
    assert_eq!(undo_depth(&app), before_undo);
}

#[test]
fn dimreassoc_names_dimensions_without_an_override_and_writes_only_the_rest() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A]);
    set_override(&mut app, &mut session, "custom");
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    select(&mut session, &[DIM_A, DIM_B]);
    let out = reassoc(&mut app, &mut session).expect("DIMREASSOC is admitted");
    assert_eq!(out.objects, vec![ObjectId(DIM_A)]);
    assert_eq!(revision(&app).0, before_revision.0 + 1);
    assert_eq!(undo_depth(&app), before_undo + 1);
    assert_eq!(stored(&app, DIM_A).text_override(), None);
    assert_eq!(
        stored(&app, DIM_B).text_override(),
        None,
        "the no-override row was already at its measured value"
    );
    let no_override: Vec<_> = out
        .diagnostics
        .iter()
        .filter(|d| d.code == "dimreassoc.no_override")
        .collect();
    assert_eq!(no_override.len(), 1);
    assert_eq!(no_override[0].object, Some(ObjectId(DIM_B)));
}

#[test]
fn dimreassoc_with_nothing_writable_is_a_named_noop_without_a_transaction() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A, DIM_B]);
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    let out = reassoc(&mut app, &mut session).expect("no override is not an error");
    assert!(out.changes.is_none(), "no transaction");
    assert!(out.objects.is_empty());
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), before_undo);
    assert!(!app.can_undo(&DocumentId(1)));
    assert_eq!(
        out.diagnostics
            .iter()
            .filter(|d| d.code == "dimreassoc.no_override")
            .count(),
        2
    );
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "dimreassoc.unchanged"));
}

#[test]
fn dimreassoc_refuses_before_writing() {
    let (mut app, mut session) = dimension_edit_app();

    // Empty pre-selection.
    let error = reassoc(&mut app, &mut session).expect_err("needs a selection");
    assert!(
        format!("{error}").contains("dimreassoc.selection"),
        "{error}"
    );

    // A non-dimension selection.
    select(&mut session, &[803]);
    let error = reassoc(&mut app, &mut session).expect_err("not a dimension");
    assert!(
        format!("{error}").contains("dimreassoc.not_dimension"),
        "{error}"
    );

    // A mixed batch with one writable dimension still writes nothing: the
    // non-dimension refusal comes before staging.
    select(&mut session, &[DIM_A]);
    set_override(&mut app, &mut session, "custom");
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    select(&mut session, &[DIM_A, 803]);
    let error = reassoc(&mut app, &mut session).expect_err("the batch is refused");
    assert!(
        format!("{error}").contains("dimreassoc.not_dimension"),
        "{error}"
    );
    assert_eq!(stored(&app, DIM_A).text_override(), Some("custom"));
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), before_undo);

    // A payload is refused by name too.
    let refused = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::DimReassoc),
        CommandPayload::Points(vec![point(1.0, 2.0)]),
    );
    let error = refused.expect_err("DIMREASSOC takes no payload");
    assert!(format!("{error}").contains("dimreassoc.payload"), "{error}");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), before_undo);
}

#[test]
fn dimreassoc_is_work_only() {
    assert!(ExtendedCommandId::DimReassoc.requires_work_mode());
    assert!(!ExtendedCommandId::DimReassociate.requires_work_mode());

    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A]);
    set_override(&mut app, &mut session, "custom");
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    session.switch_mode(AppMode::Viewer).expect("Viewer mode");
    let error = reassoc(&mut app, &mut session).expect_err("Viewer refuses the write");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(stored(&app, DIM_A).text_override(), Some("custom"));
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), before_undo);
}
