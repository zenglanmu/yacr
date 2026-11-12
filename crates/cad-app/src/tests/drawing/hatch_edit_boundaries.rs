//! HATCHEDIT ADDBOUNDARIES / REMOVEBOUNDARIES contract tests.
//!
//! The pre-selection model is the input: exactly one target hatch plus the other
//! selected closed entities as boundary sources. These tests pin the loop-count
//! change, the one-transaction/one-undo shape (UNDO/REDO), and every named
//! refusal writing nothing.

use super::hatch::{circle, hatch_app, run, square, undo_depth};
use super::*;

use crate::HatchEditOption;

fn select(session: &mut SessionState, ids: &[u128]) {
    session
        .selection
        .replace(ids.iter().copied().map(ref_of).collect::<Vec<_>>());
}

fn stored_hatch(app: &Application, id: u128) -> HatchGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the hatch entity");
    match &entity.geometry {
        SemanticGeometry::Hatch(hatch) => (**hatch).clone(),
        other => panic!("expected a hatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 2026-11 HATCHEDIT ADDBOUNDARIES / REMOVEBOUNDARIES (pre-selection model).
// ---------------------------------------------------------------------------

fn edit_selection(
    app: &mut Application,
    session: &mut SessionState,
    ids: &[u128],
    option: HatchEditOption,
) -> CadResult<CommandOutcome> {
    select(session, ids);
    execute(
        app,
        session,
        CommandId::HatchEdit,
        CommandPayload::HatchEdit(option),
    )
}

#[test]
fn hatchedit_add_boundaries_appends_a_loop_in_one_undo_step() {
    let (mut app, mut session) = hatch_app(vec![
        square(1, 0.0, 10.0, 0.0),
        circle(2, 20.0, 0.0, 0.0, 5.0),
    ]);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let before = stored_hatch(&app, id);
    assert_eq!(before.loops().len(), 1);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let out = edit_selection(
        &mut app,
        &mut session,
        &[id, 2],
        HatchEditOption::AddBoundaries,
    )
    .expect("ADDBOUNDARIES appends the circle loop");
    assert_eq!(out.objects, vec![ObjectId(id)]);
    assert_eq!(revision(&app).0, revision_before.0 + 1, "one revision");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");

    let after = stored_hatch(&app, id);
    assert_eq!(after.loops().len(), before.loops().len() + 1);
    assert_eq!(
        &after.loops()[..before.loops().len()],
        before.loops(),
        "the existing loops are kept in order"
    );
    assert_eq!(after.plane(), before.plane(), "plane preserved");
    assert_eq!(after.style(), before.style(), "style preserved");
    assert_eq!(after.fill(), before.fill(), "fill preserved");
    assert_eq!(after.seeds(), before.seeds(), "seeds preserved");
    assert_eq!(after.pixel_size(), before.pixel_size());
    assert_eq!(after.pattern_type(), before.pattern_type());

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        stored_hatch(&app, id).loops(),
        before.loops(),
        "UNDO restores the original loop set"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        stored_hatch(&app, id).loops().len(),
        before.loops().len() + 1,
        "REDO reapplies the added loop"
    );
}

#[test]
fn hatchedit_add_boundaries_refuses_a_duplicate_loop_writing_nothing() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    // The square already is the hatch's only loop: adding it again is a
    // duplicate ring, rejected by the domain constructor.
    let error = edit_selection(
        &mut app,
        &mut session,
        &[id, 1],
        HatchEditOption::AddBoundaries,
    )
    .expect_err("a duplicate boundary must refuse");
    assert!(
        format!("{error:?}").contains("hatchedit.add_boundaries_invalid"),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before, "no revision bump");
    assert_eq!(undo_depth(&app), undo_before, "no undo entry");
    assert_eq!(stored_hatch(&app, id).loops().len(), 1);
}

#[test]
fn hatchedit_remove_boundaries_drops_the_matching_loop_in_one_undo_step() {
    let (mut app, mut session) =
        hatch_app(vec![square(1, 0.0, 10.0, 0.0), square(2, 20.0, 30.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1, 2])
        .expect("HATCH from two boundaries")
        .objects[0]
        .0;
    let before = stored_hatch(&app, id);
    assert_eq!(before.loops().len(), 2);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let out = edit_selection(
        &mut app,
        &mut session,
        &[id, 2],
        HatchEditOption::RemoveBoundaries,
    )
    .expect("REMOVEBOUNDARIES matches square 2");
    assert_eq!(out.objects, vec![ObjectId(id)]);
    assert_eq!(revision(&app).0, revision_before.0 + 1, "one revision");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    let after = stored_hatch(&app, id);
    assert_eq!(after.loops().len(), before.loops().len() - 1);
    assert_eq!(
        after.loops()[0],
        before.loops()[0],
        "the surviving loop is unchanged"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        stored_hatch(&app, id).loops(),
        before.loops(),
        "UNDO restores the removed loop"
    );
}

#[test]
fn hatchedit_remove_boundaries_refuses_a_non_matching_loop() {
    let (mut app, mut session) =
        hatch_app(vec![square(1, 0.0, 10.0, 0.0), square(2, 20.0, 30.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let error = edit_selection(
        &mut app,
        &mut session,
        &[id, 2],
        HatchEditOption::RemoveBoundaries,
    )
    .expect_err("square 2 is not a stored loop of this hatch");
    assert!(
        format!("{error:?}").contains("hatchedit.remove_no_match"),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored_hatch(&app, id).loops().len(), 1);
}

#[test]
fn hatchedit_remove_boundaries_refuses_to_leave_zero_loops() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let error = edit_selection(
        &mut app,
        &mut session,
        &[id, 1],
        HatchEditOption::RemoveBoundaries,
    )
    .expect_err("removing the only loop must refuse");
    assert!(
        format!("{error:?}").contains("hatchedit.remove_last_loop"),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored_hatch(&app, id).loops().len(), 1);
}

#[test]
fn hatchedit_boundary_edits_require_exactly_one_hatch_and_one_boundary() {
    // No hatch in the selection.
    let (mut app, mut session) =
        hatch_app(vec![square(1, 0.0, 10.0, 0.0), square(2, 20.0, 30.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let error = edit_selection(&mut app, &mut session, &[2], HatchEditOption::AddBoundaries)
        .expect_err("no hatch selected");
    assert!(
        format!("{error:?}").contains("hatchedit.selection"),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);

    // Exactly one hatch but no boundary source.
    let error = edit_selection(
        &mut app,
        &mut session,
        &[id],
        HatchEditOption::AddBoundaries,
    )
    .expect_err("no boundary source selected");
    assert!(
        format!("{error:?}").contains("hatchedit.no_boundary_source"),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);

    // Two hatches in one selection.
    let (mut app, mut session) = hatch_app(vec![
        square(1, 0.0, 10.0, 0.0),
        square(2, 20.0, 30.0, 0.0),
        circle(3, 40.0, 0.0, 0.0, 5.0),
    ]);
    let first = run(&mut app, &mut session, &[1]).expect("HATCH 1").objects[0].0;
    let second = run(&mut app, &mut session, &[2]).expect("HATCH 2").objects[0].0;
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let error = edit_selection(
        &mut app,
        &mut session,
        &[first, second, 3],
        HatchEditOption::AddBoundaries,
    )
    .expect_err("more than one hatch");
    assert!(
        format!("{error:?}").contains("hatchedit.multiple_hatches"),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
}

// ---------------------------------------------------------------------------
// Boundary-source refusals (the shared HATCHSETBOUNDARY projection)
// ---------------------------------------------------------------------------

#[test]
fn hatchedit_boundary_edits_refuse_an_open_or_off_plane_source_writing_nothing() {
    // An open polyline is not a closed region: the shared projection refuses it
    // before the domain constructor sees a loop.
    let (mut app, mut session) = hatch_app(vec![
        square(1, 0.0, 10.0, 0.0),
        (
            2,
            SemanticGeometry::Polyline {
                points: vec![p3(20.0, 0.0, 0.0), p3(30.0, 0.0, 0.0), p3(30.0, 5.0, 0.0)],
                bulges: vec![0.0; 3],
                closed: false,
                width: 0.0,
            },
        ),
    ]);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let loops_before = stored_hatch(&app, id).loops().len();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let error = edit_selection(
        &mut app,
        &mut session,
        &[id, 2],
        HatchEditOption::AddBoundaries,
    )
    .expect_err("an open polyline is not a boundary");
    assert!(
        format!("{error:?}").contains("hatchsetboundary.not_closed"),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored_hatch(&app, id).loops().len(), loops_before);

    // A closed square on another elevation cannot be flattened onto the hatch
    // plane, so it is refused by name instead of being projected.
    let (mut app, mut session) =
        hatch_app(vec![square(1, 0.0, 10.0, 0.0), square(2, 20.0, 30.0, 5.0)]);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let error = edit_selection(
        &mut app,
        &mut session,
        &[id, 2],
        HatchEditOption::AddBoundaries,
    )
    .expect_err("an off-plane source is not a boundary");
    assert!(
        format!("{error:?}").contains("hatchsetboundary.non_planar"),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(stored_hatch(&app, id).loops().len(), 1);
}
