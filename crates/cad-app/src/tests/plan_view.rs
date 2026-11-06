//! `PLAN` (AutoCAD 2026): display an orthographic view of the XY plane.
//!
//! v1 is the **top/plan 2D view only**. This build has no UCS model, so
//! AutoCAD's `PLAN` orientation options (UCS / World / Current) are NOT
//! IMPLEMENTED; the command is payload-free and refuses a payload explicitly
//! instead of ignoring it. The contract pinned here:
//!
//! * `PLAN` while already in the 2D plan is the documented no-op — it moves no
//!   camera, records no `ZOOM PREVIOUS` step and writes nothing;
//! * `PLAN` from a 3D observation returns to the 2D plan (the saved 2D camera)
//!   and records one `ZOOM PREVIOUS` step, so the 3D view can be restored;
//! * a non-empty payload is an explicit `InvalidInput` (`PLAN needs no payload`);
//! * a stale document and an unknown viewport are refused without moving the
//!   camera;
//! * no document revision bump, no `cad-history` entry and no `ChangeSet`.
//!
//! No GUI, GPU or AutoCAD-parity claim is made by these tests; they are
//! command/session-level contracts only.

use super::*;

fn plan_command(payload: CommandPayload) -> Command {
    command(CommandId::PlanView, payload)
}

fn viewport(app: &Application) -> &Viewport {
    &app.workspace.viewports[&ViewportId(1)]
}

#[test]
fn plan_from_the_plan_2d_view_is_a_noop() {
    let (mut app, mut session) = application_with_document();
    let document = session.document;
    let camera_before = viewport(&app).camera;
    let steps_before = viewport(&app).zoom_history_len();
    let revision_before = app.workspace.documents[&document].drawing.revision();

    let outcome = app
        .execute(&mut session, plan_command(CommandPayload::None))
        .expect("PLAN is admitted in the 2D plan view");

    assert!(outcome.changes.is_none(), "a plan view writes no ChangeSet");
    assert!(
        outcome.diagnostics.is_empty(),
        "the no-op case reports no diagnostic: {:?}",
        outcome.diagnostics
    );
    let vp = viewport(&app);
    assert_eq!(vp.view_mode.kind(), ProjectionKind::TwoD);
    assert!(vp.camera.projection.is_orthographic());
    assert_eq!(
        vp.camera, camera_before,
        "the plan camera already on screen"
    );
    assert_eq!(
        vp.zoom_history_len(),
        steps_before,
        "a plan view while already 2D records no ZOOM PREVIOUS step"
    );
    assert_eq!(
        app.workspace.documents[&document].drawing.revision(),
        revision_before,
        "a view command never bumps the drawing revision"
    );
    assert!(!app.can_undo(&document));
    assert!(!app.can_redo(&document));
}

#[test]
fn plan_from_3d_returns_to_the_2d_plan_and_records_one_step() {
    let (mut app, mut session) = application_with_document();
    let saved_plan = viewport(&app).camera;

    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .expect("SWITCH2D3D enters 3D");
    assert_eq!(viewport(&app).view_mode.kind(), ProjectionKind::ThreeD);
    let steps_after_3d = viewport(&app).zoom_history_len();
    let camera_3d = viewport(&app).camera;

    app.execute(&mut session, plan_command(CommandPayload::None))
        .expect("PLAN is admitted from a 3D view");

    let vp = viewport(&app);
    assert_eq!(vp.view_mode.kind(), ProjectionKind::TwoD);
    assert!(vp.camera.projection.is_orthographic());
    assert_eq!(
        vp.camera, saved_plan,
        "PLAN restores the 2D camera saved when the 3D view was entered"
    );
    assert_eq!(
        vp.zoom_history_len(),
        steps_after_3d + 1,
        "leaving 3D for the plan is a discrete view change"
    );

    // The recorded step is the 3D view: ZOOM PREVIOUS must go back to it.
    app.execute(
        &mut session,
        command(CommandId::Zoom, CommandPayload::ZoomPrevious),
    )
    .expect("ZOOM PREVIOUS is never an error");
    let vp = viewport(&app);
    assert_eq!(vp.view_mode.kind(), ProjectionKind::ThreeD);
    assert_eq!(vp.camera, camera_3d, "ZOOM PREVIOUS undoes the PLAN");
}

#[test]
fn plan_refuses_a_payload_instead_of_ignoring_it() {
    let (mut app, mut session) = application_with_document();
    let camera_before = viewport(&app).camera;
    let steps_before = viewport(&app).zoom_history_len();

    for payload in [
        CommandPayload::StandardView(StandardView::Front),
        CommandPayload::Points(vec![point(1.0, 2.0)]),
    ] {
        let error = app
            .execute(&mut session, plan_command(payload))
            .expect_err("PLAN takes no payload");
        let CadError::InvalidInput(message) = error else {
            panic!("expected an explicit InvalidInput, got {error:?}");
        };
        assert!(
            message.contains("PLAN needs no payload"),
            "the refusal names the rule: {message}"
        );
    }

    assert_eq!(viewport(&app).camera, camera_before);
    assert_eq!(viewport(&app).zoom_history_len(), steps_before);
}

#[test]
fn plan_refuses_a_stale_document_and_an_unknown_viewport() {
    let (mut app, mut session) = application_with_document();
    let camera_before = viewport(&app).camera;
    let steps_before = viewport(&app).zoom_history_len();

    // A command addressed to a document the session is not on.
    let stale = Command {
        schema_version: 1,
        id: CommandId::PlanView,
        document: DocumentId(2),
        viewport: ViewportId(1),
        payload: CommandPayload::None,
    };
    assert!(matches!(
        app.execute(&mut session, stale),
        Err(CadError::StaleResult)
    ));

    // A command addressed to a viewport the workspace does not have.
    let unknown_viewport = Command {
        schema_version: 1,
        id: CommandId::PlanView,
        document: DocumentId(1),
        viewport: ViewportId(9),
        payload: CommandPayload::None,
    };
    let error = app
        .execute(&mut session, unknown_viewport)
        .expect_err("an unknown viewport is refused");
    let CadError::InvalidInput(message) = error else {
        panic!("expected an explicit InvalidInput, got {error:?}");
    };
    assert!(
        message.contains("unknown viewport"),
        "the refusal names the target: {message}"
    );

    assert_eq!(viewport(&app).camera, camera_before);
    assert_eq!(viewport(&app).zoom_history_len(), steps_before);
}
