//! `HATCHEDIT ORIGIN <x> <y>` contract tests (AutoCAD 2026).
//!
//! The option moves the **pattern phase** — the hatch-plane displacement applied
//! to the pattern's definition rows — because neither the DXF nor the DWG hatch
//! record has a dedicated origin group (the phase lives in each row's base point,
//! groups 43/44; `cad-connector-acadrust`'s export folds it back there). These tests
//! pin the stored value, the rebuilt display, the one-transaction/one-undo shape,
//! the interaction with the other pattern edits, and every refusal.

use super::hatch::{
    entity_count, hatch_app, run, set_number, square, the_hatch, undo_depth, user_defined,
};
use super::*;

use crate::HatchEditOption;
use cad_domain::{HatchFill, HatchPatternType};

fn select(session: &mut SessionState, ids: &[u128]) {
    session
        .selection
        .replace(ids.iter().copied().map(ref_of).collect::<Vec<_>>());
}

fn edit(
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

/// A user-defined pattern hatch (spacing 2, angle 0, single family) on the closed
/// square entity `id`; returns the hatch's object id.
fn pattern_hatch(app: &mut Application, session: &mut SessionState, id: u128) -> u128 {
    user_defined(session, 2.0, 0.0, false);
    run(app, session, &[id]).expect("HATCH succeeds").objects[0].0
}

/// The `z`-plane `y` of every visible pattern line, sorted.
fn pattern_line_ys(hatch: &HatchGeometry) -> Vec<f64> {
    let boundaries = hatch.loops().len();
    let mut ys: Vec<f64> = hatch
        .display()
        .iter()
        .skip(boundaries)
        .map(|geometry| {
            let SemanticGeometry::Polyline { points, .. } = geometry else {
                panic!("pattern display must be polylines, got {geometry:?}");
            };
            points[0].y
        })
        .collect();
    ys.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    ys
}

#[test]
fn origin_moves_the_stored_pattern_phase_and_rebuilds_the_display() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    let before = the_hatch(&app).clone();
    assert_eq!(
        before.pattern_origin(),
        [0.0, 0.0],
        "a fresh hatch starts at the rows' own origin"
    );
    let ys_before = pattern_line_ys(&before);
    assert!(!ys_before.is_empty(), "the user pattern draws lines");
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let out = edit(
        &mut app,
        &mut session,
        &[id],
        HatchEditOption::Origin { x: 0.0, y: 0.75 },
    )
    .expect("HATCHEDIT ORIGIN succeeds");
    assert_eq!(out.objects, vec![ObjectId(id)]);
    assert_eq!(
        undo_depth(&app),
        undo_before + 1,
        "one transaction / one undo step"
    );
    assert_eq!(entity_count(&app), 2, "no entity was inserted or removed");
    assert_eq!(revision(&app).0, revision_before.0 + 1, "one revision");

    let after = the_hatch(&app);
    assert_eq!(after.pattern_origin(), [0.0, 0.75]);
    // The display is the domain-derived one: every pattern line moved by the
    // displacement, no boundary loop changed.
    let ys_after = pattern_line_ys(after);
    assert_eq!(
        ys_after.len(),
        ys_before.len(),
        "the phase moved, the line count did not change"
    );
    for (moved, original) in ys_after.iter().zip(&ys_before) {
        assert!(
            (moved - (original + 0.75)).abs() < 1e-9,
            "line {original} should have moved to {}, got {moved}",
            original + 0.75
        );
    }
    assert_eq!(
        &after.display()[..after.loops().len()],
        &before.display()[..before.loops().len()],
        "the boundary polylines are untouched"
    );
    // Every other stored field is preserved.
    assert_eq!(after.loops(), before.loops());
    assert_eq!(after.plane(), before.plane());
    assert_eq!(after.style(), before.style());
    assert_eq!(after.fill(), before.fill());
    assert_eq!(after.seeds(), before.seeds());
    assert_eq!(after.pixel_size(), before.pixel_size());
    assert_eq!(after.pattern_type(), before.pattern_type());

    // One undo restores the exact stored origin.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(the_hatch(&app).pattern_origin(), [0.0, 0.0]);
}

#[test]
fn origin_moves_every_selected_hatch_in_one_undo() {
    let (mut app, mut session) =
        hatch_app(vec![square(1, 0.0, 10.0, 0.0), square(2, 20.0, 30.0, 0.0)]);
    let first = pattern_hatch(&mut app, &mut session, 1);
    set_number(&mut session, ToolParameter::HatchSpacing, 1.0);
    let second = run(&mut app, &mut session, &[2]).expect("HATCH 2").objects[0].0;
    let undo_before = undo_depth(&app);

    let out = edit(
        &mut app,
        &mut session,
        &[first, second],
        HatchEditOption::Origin { x: -1.0, y: 2.0 },
    )
    .expect("multi-target ORIGIN");
    assert_eq!(out.objects.len(), 2);
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo for both");
    for id in [first, second] {
        let stored = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(id))
            .cloned()
            .expect("the hatch survives");
        let SemanticGeometry::Hatch(hatch) = &stored.geometry else {
            panic!("entity {id} must stay a hatch");
        };
        assert_eq!(hatch.pattern_origin(), [-1.0, 2.0], "entity {id}");
    }
}

#[test]
fn repeating_the_stored_origin_is_a_named_no_op() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    edit(
        &mut app,
        &mut session,
        &[id],
        HatchEditOption::Origin { x: 1.0, y: 2.0 },
    )
    .expect("first ORIGIN");
    let undo_before = undo_depth(&app);
    let revision_before = revision(&app);

    let out = edit(
        &mut app,
        &mut session,
        &[id],
        HatchEditOption::Origin { x: 1.0, y: 2.0 },
    )
    .expect("ORIGIN to the same point");
    assert_eq!(out.diagnostics[0].code, "hatchedit.unchanged");
    assert!(out.changes.is_none(), "no transaction on a no-op");
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(revision(&app).0, revision_before.0);

    // `ORIGIN 0 0` is the source rows' own origin, so it is a real change back.
    edit(
        &mut app,
        &mut session,
        &[id],
        HatchEditOption::Origin { x: 0.0, y: 0.0 },
    )
    .expect("ORIGIN 0 0");
    assert_eq!(the_hatch(&app).pattern_origin(), [0.0, 0.0]);
}

#[test]
fn origin_is_refused_on_a_solid_fill_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    // The default creation parameters are a solid fill.
    let id = run(&mut app, &mut session, &[1])
        .expect("HATCH solid")
        .objects[0]
        .0;
    assert!(matches!(the_hatch(&app).fill(), HatchFill::Solid));
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let error = edit(
        &mut app,
        &mut session,
        &[id],
        HatchEditOption::Origin { x: 1.0, y: 2.0 },
    )
    .expect_err("a solid fill has no pattern rows to move");
    let text = format!("{error:?}");
    assert!(text.contains("hatchedit.not_pattern"), "{text}");
    assert!(text.contains("a solid fill"), "{text}");
    assert_eq!(revision(&app).0, revision_before.0);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(the_hatch(&app).pattern_origin(), [0.0, 0.0]);
}

#[test]
fn origin_needs_finite_coordinates() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    let undo_before = undo_depth(&app);
    for (x, y) in [(f64::NAN, 0.0), (0.0, f64::INFINITY)] {
        let error = edit(
            &mut app,
            &mut session,
            &[id],
            HatchEditOption::Origin { x, y },
        )
        .expect_err("a non-finite origin is refused");
        assert!(
            format!("{error:?}").contains("hatchedit.origin"),
            "{error:?}"
        );
    }
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(the_hatch(&app).pattern_origin(), [0.0, 0.0]);
}

#[test]
fn origin_survives_the_other_pattern_edits() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    edit(
        &mut app,
        &mut session,
        &[id],
        HatchEditOption::Origin { x: 0.5, y: -0.5 },
    )
    .expect("ORIGIN");
    // Every rebuild path goes through the domain constructor, which starts at the
    // zero origin; the stored displacement must be applied back explicitly.
    for option in [
        HatchEditOption::Angle(30.0),
        HatchEditOption::Scale(1.5),
        HatchEditOption::Pattern("ANSI31".into()),
        HatchEditOption::PatternType(HatchPatternType::Custom),
        HatchEditOption::Style(HatchStyle::Outer),
    ] {
        edit(&mut app, &mut session, &[id], option.clone()).expect("property edit");
        assert_eq!(
            the_hatch(&app).pattern_origin(),
            [0.5, -0.5],
            "{option:?} must not reset the pattern origin"
        );
    }
    // Converting the fill away from a pattern keeps the stored value (it is the
    // export that declares it unwritable for a solid/gradient: there are no rows
    // left to carry it).
    edit(&mut app, &mut session, &[id], HatchEditOption::Solid).expect("SOLID");
    assert_eq!(the_hatch(&app).pattern_origin(), [0.5, -0.5]);
    assert!(matches!(the_hatch(&app).fill(), HatchFill::Solid));
}

#[test]
fn viewer_mode_refuses_origin_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    session.switch_mode(AppMode::Viewer).unwrap();
    select(&mut session, &[id]);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchEdit,
        CommandPayload::HatchEdit(HatchEditOption::Origin { x: 1.0, y: 1.0 }),
    )
    .expect_err("Viewer must refuse");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app).0, revision_before.0);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(the_hatch(&app).pattern_origin(), [0.0, 0.0]);
}

#[test]
fn an_origin_edit_keeps_the_hatch_addressable_and_its_selection_semantics() {
    // A non-hatch selection and an empty selection stay the named refusals they
    // already were; ORIGIN adds no new path around the target resolution.
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    let revision_before = revision(&app);
    let error = edit(
        &mut app,
        &mut session,
        &[1],
        HatchEditOption::Origin { x: 1.0, y: 1.0 },
    )
    .expect_err("the source square is not a hatch");
    assert!(
        format!("{error:?}").contains("hatchedit.not_hatch"),
        "{error:?}"
    );
    select(&mut session, &[]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchEdit,
        CommandPayload::HatchEdit(HatchEditOption::Origin { x: 1.0, y: 1.0 }),
    )
    .expect_err("an empty selection is refused");
    assert!(
        format!("{error:?}").contains("hatchedit.selection"),
        "{error:?}"
    );
    assert_eq!(revision(&app).0, revision_before.0);
    assert_eq!(the_hatch(&app).pattern_origin(), [0.0, 0.0]);
    assert_eq!(
        the_hatch(&app).loops()[0].points.len(),
        4,
        "the hatch itself is untouched (entity {id})"
    );
}

#[test]
fn a_boundary_edit_keeps_the_stored_pattern_origin() {
    // ADDBOUNDARIES/REMOVEBOUNDARIES rebuild through the creation-path
    // constructor, which starts at the zero origin; the displacement must be
    // applied back so adding a loop does not re-phase the pattern.
    let (mut app, mut session) =
        hatch_app(vec![square(1, 0.0, 10.0, 0.0), square(2, 20.0, 30.0, 0.0)]);
    let id = pattern_hatch(&mut app, &mut session, 1);
    edit(
        &mut app,
        &mut session,
        &[id],
        HatchEditOption::Origin { x: 0.5, y: 0.5 },
    )
    .expect("ORIGIN");

    edit(
        &mut app,
        &mut session,
        &[id, 2],
        HatchEditOption::AddBoundaries,
    )
    .expect("ADDBOUNDARIES");
    assert_eq!(the_hatch(&app).loops().len(), 2, "the loop was appended");
    assert_eq!(the_hatch(&app).pattern_origin(), [0.5, 0.5]);

    edit(
        &mut app,
        &mut session,
        &[id, 2],
        HatchEditOption::RemoveBoundaries,
    )
    .expect("REMOVEBOUNDARIES");
    assert_eq!(the_hatch(&app).loops().len(), 1, "the loop was removed");
    assert_eq!(the_hatch(&app).pattern_origin(), [0.5, 0.5]);
}
