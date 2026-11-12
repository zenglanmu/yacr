//! `HATCHEDIT PATTERN <name>` contract tests (AutoCAD 2026).
//!
//! The option selects a **built-in** `.pat` library pattern by name
//! (case-insensitive), keeps the hatch's stored pattern angle/scale/`double`,
//! rewrites the fill as a predefined pattern (`76=1`) and writes it in one
//! transaction / one undo step. Every refusal writes nothing. A user
//! `acad.pat`/`acadiso.pat` entry is not selectable yet (host file I/O), so the
//! library is the closed built-in set.

use super::hatch::{entity_count, hatch_app, run, square, the_hatch, undo_depth, user_defined};
use super::*;

use crate::HatchEditOption;
use cad_domain::hatch::{predefined_pattern, predefined_patterns};
use cad_domain::{HatchFill, HatchPattern, HatchPatternType};

fn select(session: &mut SessionState, ids: &[u128]) {
    session
        .selection
        .replace(ids.iter().copied().map(ref_of).collect::<Vec<_>>());
}

fn edit(
    app: &mut Application,
    session: &mut SessionState,
    entity: u128,
    option: HatchEditOption,
) -> CadResult<CommandOutcome> {
    select(session, &[entity]);
    execute(
        app,
        session,
        CommandId::HatchEdit,
        CommandPayload::HatchEdit(option),
    )
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

fn pattern(hatch: &HatchGeometry) -> &HatchPattern {
    let HatchFill::Pattern(pattern) = hatch.fill() else {
        panic!("expected a pattern fill, got {:?}", hatch.fill());
    };
    pattern
}

// ---------------------------------------------------------------------------
// 2026-11-12 HATCHEDIT PATTERN <name> (built-in `.pat` name channel).
// ---------------------------------------------------------------------------

#[test]
fn hatchedit_pattern_name_uses_the_library_rows_and_keeps_angle_scale_double() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    // A non-default angle/scale and HPDOUBLE make the "keep the stored settings"
    // contract visible: only the name and the definition rows may change.
    user_defined(&mut session, 4.0, 30.0, true);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let before = stored_hatch(&app, id);
    assert_eq!(before.pattern_type(), HatchPatternType::UserDefined);
    assert_eq!(pattern(&before).name, "USER");
    let display_before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_render_attributes(EntityId(id));
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let out = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Pattern("ansi31".into()),
    )
    .expect("HATCHEDIT PATTERN ansi31");
    assert_eq!(out.objects, vec![ObjectId(id)]);
    assert_eq!(revision(&app).0, revision_before.0 + 1, "one revision");
    assert_eq!(undo_depth(&app), undo_before + 1, "one undo step");
    assert_eq!(entity_count(&app), 2, "no entity was inserted or removed");

    let after = stored_hatch(&app, id);
    assert_eq!(after.loops(), before.loops(), "loops preserved");
    assert_eq!(after.plane(), before.plane(), "plane preserved");
    assert_eq!(after.style(), before.style(), "style preserved");
    assert_eq!(after.seeds(), before.seeds(), "seeds preserved");
    assert_eq!(after.pixel_size(), before.pixel_size());
    assert_eq!(
        after.pattern_type(),
        HatchPatternType::Predefined,
        "a built-in library name is group 76 = 1"
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(EntityId(id)),
        display_before,
        "display attributes are untouched"
    );
    assert!(!after.display().is_empty(), "the display is rebuilt");

    let stored = pattern(&after);
    assert_eq!(stored.name, "ANSI31", "the library name is canonical");
    assert_eq!(stored.scale, 4.0, "the stored scale is kept");
    assert!(
        (stored.angle - 30.0f64.to_radians()).abs() < 1e-12,
        "the stored angle is kept verbatim: {}",
        stored.angle
    );
    assert!(stored.double, "the stored HPDOUBLE flag is kept");
    let expected = predefined_pattern("ANSI31")
        .expect("ANSI31 is implemented")
        .to_hatch_pattern(4.0, 30.0, true);
    assert_eq!(stored.lines, expected.lines, "the library rows are copied");

    // Repeating the same name changes nothing: named no-op, no transaction.
    let undo_after = undo_depth(&app);
    let out = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Pattern("ANSI31".into()),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "hatchedit.unchanged");
    assert_eq!(undo_depth(&app), undo_after);

    // One undo restores the user-defined fill and group 76 = 0.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let restored = stored_hatch(&app, id);
    assert_eq!(pattern(&restored).name, "USER");
    assert_eq!(restored.pattern_type(), HatchPatternType::UserDefined);
}

#[test]
fn hatchedit_pattern_accepts_every_builtin_library_name() {
    let library = predefined_patterns();
    assert!(!library.is_empty(), "the built-in library is not empty");
    for entry in library {
        let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
        user_defined(&mut session, 2.0, 0.0, false);
        let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
        edit(
            &mut app,
            &mut session,
            id,
            HatchEditOption::Pattern(entry.name.to_lowercase()),
        )
        .unwrap_or_else(|error| panic!("PATTERN {}: {error:?}", entry.name));
        assert_eq!(
            pattern(&stored_hatch(&app, id)).name,
            entry.name,
            "the library is the single name authority"
        );
    }
}

#[test]
fn hatchedit_pattern_unknown_name_is_named_and_writes_nothing() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    user_defined(&mut session, 2.0, 0.0, false);
    let id = run(&mut app, &mut session, &[1]).expect("HATCH").objects[0].0;
    let before = stored_hatch(&app, id);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let error = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Pattern("ANSI999".into()),
    )
    .expect_err("an unknown pattern name must refuse");
    let text = format!("{error:?}");
    assert!(text.contains("hatchedit.unknown_pattern"), "{text}");
    assert!(text.contains("ANSI999"), "{text}");
    assert!(
        text.contains("ANSI31"),
        "the message lists the accepted names: {text}"
    );
    assert_eq!(revision(&app), revision_before, "nothing was written");
    assert_eq!(undo_depth(&app), undo_before, "no transaction");
    assert_eq!(stored_hatch(&app, id), before);
}

#[test]
fn hatchedit_pattern_refuses_solid_and_gradient_fills() {
    let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
    let id = run(&mut app, &mut session, &[1])
        .expect("solid HATCH")
        .objects[0]
        .0;
    assert!(matches!(the_hatch(&app).fill(), HatchFill::Solid));

    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let error = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Pattern("ANSI31".into()),
    )
    .expect_err("a solid fill must refuse PATTERN");
    let text = format!("{error:?}");
    assert!(
        text.contains("hatchedit.pattern_requires_pattern"),
        "{text}"
    );
    assert!(text.contains("solid fill"), "{text}");
    assert_eq!(revision(&app), revision_before, "nothing was written");
    assert_eq!(undo_depth(&app), undo_before, "no transaction");

    // The same refusal applies to a gradient fill: the option never silently
    // converts either fill variant into a pattern.
    edit(&mut app, &mut session, id, HatchEditOption::Gradient).expect("SOLID -> GRADIENT");
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let error = edit(
        &mut app,
        &mut session,
        id,
        HatchEditOption::Pattern("ANSI31".into()),
    )
    .expect_err("a gradient fill must refuse PATTERN");
    let text = format!("{error:?}");
    assert!(
        text.contains("hatchedit.pattern_requires_pattern"),
        "{text}"
    );
    assert!(text.contains("gradient fill"), "{text}");
    assert_eq!(revision(&app), revision_before, "nothing was written");
    assert_eq!(undo_depth(&app), undo_before, "no transaction");
}
