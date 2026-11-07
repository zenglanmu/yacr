//! `LTSCALE` (AutoCAD 2026): the drawing-global linetype scale (`$LTSCALE`).
//!
//! One post-import database write through `DrawingDatabase::set_linetype_scale`:
//! the value round-trips through `linetype_scale()`, invalid values are refused
//! before any write, and the write records **no** history entry (there is no
//! header-variable patch kind).

use super::*;

#[test]
fn ltscale_writes_the_header_value_with_one_revision_bump_and_no_undo() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        Vec::new(),
    );
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .linetype_scale(),
        1.0
    );
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SetLinetypeScale,
        CommandPayload::LinetypeScale(2.5),
    )
    .expect("2.5 is a legal $LTSCALE");

    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .linetype_scale(),
        2.5
    );
    assert_eq!(revision(&app), Revision(before.0 .0 + 1));
    assert_eq!(undo_depth(&app), before.1, "header writes are not undoable");
    assert!(outcome.changes.is_some());
    assert_eq!(diagnostic_codes(&outcome), vec!["drawing.linetype_scale"]);
}

#[test]
fn ltscale_refuses_non_positive_and_non_finite_values() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        Vec::new(),
    );
    let before = state(&app);
    for bad in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::SetLinetypeScale,
            CommandPayload::LinetypeScale(bad),
        )
        .expect_err("the database refuses a non-positive/non-finite scale");
        assert!(
            refusal(error).contains("linetype scale must be a positive finite number"),
            "{bad}"
        );
        assert_untouched(&app, before);
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .linetype_scale(),
        1.0,
        "nothing was stored"
    );
}

#[test]
fn ltscale_reports_no_change_for_the_stored_value_and_refuses_a_missing_payload() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        Vec::new(),
    );
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SetLinetypeScale,
        CommandPayload::LinetypeScale(1.0),
    )
    .expect("writing the stored value is a reported no-op");
    assert_eq!(state(&app), before, "no revision bump, no history");
    assert!(outcome.diagnostics[0].message.contains("no change"));

    let error = execute(
        &mut app,
        &mut session,
        CommandId::SetLinetypeScale,
        CommandPayload::None,
    )
    .expect_err("a bare LTSCALE has no value");
    assert!(refusal(error).starts_with("ltscale.needs_value:"));
    assert_untouched(&app, before);
}
