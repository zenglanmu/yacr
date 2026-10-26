//! `ID` point-coordinate contract tests (measure/query area).
//!
//! Pins the declared rule: one pick ⇒ exactly one [`crate::PointReport`] with the
//! space and the snap provenance of that pick, no `ChangeSet`, no history/undo
//! step and no database write at all; the payload path is refused (a `Points`
//! payload has no acquisition layer to resolve a snap); block space is refused;
//! and the report never enters the measurement engine (`measurement` stays
//! `None`).

use super::*;

fn id_command(payload: CommandPayload) -> Command {
    command(CommandId::Measure, payload)
}

/// Start the interactive `ID` tool and return the app/session pair.
fn id_session() -> (Application, SessionState) {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        id_command(CommandPayload::MeasureTool(MeasurementToolKind::Id)),
    )
    .expect("ID starts its capture");
    (app, session)
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn one_pick_reports_the_point_with_its_space_and_no_history_step() {
    let (mut app, mut session) = id_session();
    let before_revision = revision(&app);
    let before_count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();

    // The capture path the host uses (object snap resolved, then the point).
    let snap = InputSnap {
        kind: cad_measure::SnapKind::Endpoint,
        precision: Precision::Analytic,
        logical_pixel_distance: 3.5,
    };
    let ready = session
        .capture_point_with_snap(point(4.0, 5.0), Some(snap))
        .expect("the pick is captured");
    assert!(ready, "one pick completes ID (exact_points == 1)");

    let outcome = app
        .execute(
            &mut session,
            command(CommandId::ConfirmMeasurement, CommandPayload::None),
        )
        .expect("the confirm reports the point");

    let report = outcome
        .point_report
        .as_ref()
        .expect("one pick produces one point report");
    assert_eq!(report.position, point(4.0, 5.0));
    assert_eq!(report.space, IdSpace::Model);
    assert_eq!(report.snap, Some(cad_measure::SnapKind::Endpoint));
    assert_eq!(report.precision, Precision::Analytic);
    assert_eq!(report.logical_pixel_distance, Some(3.5));
    // The demo document has no unit assignment: drawing units stay `None`
    // instead of being fabricated as millimetres.
    assert_eq!(report.unit, None);

    // A coordinate query is read-only: no ChangeSet, no undo entry, no revision
    // bump and no entity change. `measurement` stays measurement-only.
    assert!(outcome.changes.is_none(), "ID changes nothing");
    assert!(outcome.measurement.is_none(), "ID is not a measurement");
    assert_eq!(undo_depth(&app), 0, "no history entry");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before_count
    );
    // The tool returns to navigation after its one-shot result.
    assert!(session.measurement_preview().is_none());
}

#[test]
fn a_bare_pick_reports_no_snap_and_the_raw_coordinate() {
    let (mut app, mut session) = id_session();
    session
        .capture_point_with_snap(point(1.0, 2.0), None)
        .expect("the pick is captured");
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::ConfirmMeasurement, CommandPayload::None),
        )
        .expect("the confirm reports the point");
    let report = outcome.point_report.expect("a report exists");
    assert_eq!(report.position, point(1.0, 2.0));
    assert_eq!(report.snap, None, "no candidate is never a fabricated one");
    assert_eq!(
        report.precision,
        Precision::Analytic,
        "a point the user picked has no approximated source"
    );
    assert_eq!(report.logical_pixel_distance, None);
}

#[test]
fn id_skips_the_grid_and_ortho_constraint_the_measurements_obey() {
    // Our declared rule (crates/cad-app/src/point_report.rs): `ID` reports the
    // picked (or snapped) coordinate verbatim, so the grid/ORTHO quantiser — which
    // every other point-entry tool applies — must not move it.
    let (mut app, mut session) = id_session();
    session.set_snap_grid_enabled(true);
    session.set_snap_spacing(10.0).expect("a valid spacing");
    session.set_ortho_enabled(true);

    session
        .capture_point_with_snap(point(12.4, 7.6), None)
        .expect("the pick is captured");
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::ConfirmMeasurement, CommandPayload::None),
        )
        .expect("the confirm reports the point");
    let report = outcome.point_report.expect("a report exists");
    assert_eq!(
        report.position,
        point(12.4, 7.6),
        "the grid must not re-quantise an ID pick"
    );

    // Contrast: the same switches DO move a measurement's captured point, which
    // proves the switches were really on.
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        id_command(CommandPayload::MeasureTool(MeasurementToolKind::Distance)),
    )
    .unwrap();
    session.set_snap_grid_enabled(true);
    session.set_snap_spacing(10.0).expect("a valid spacing");
    session
        .capture_point_with_snap(point(12.4, 7.6), None)
        .expect("the pick is captured");
    let captured = session
        .measurement_preview()
        .expect("a distance capture is active")
        .points;
    assert_eq!(
        captured,
        vec![point(10.0, 10.0)],
        "DIST still quantises onto the grid"
    );
}

#[test]
fn the_points_payload_path_refuses_id_instead_of_fabricating_a_snap() {
    let (mut app, mut session) = id_session();
    let error = app
        .execute(
            &mut session,
            id_command(CommandPayload::Points(vec![point(1.0, 1.0)])),
        )
        .expect_err("a payload pick has no acquisition layer");
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.contains("ID")),
        "{error:?}"
    );
    // Nothing was captured and nothing was reported.
    assert_eq!(
        session
            .measurement_preview()
            .expect("the tool is still active")
            .points,
        Vec::<Point3>::new()
    );
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn block_space_is_refused_and_writes_nothing() {
    let (mut app, mut session) = id_session();
    session.active_space = SpaceId::Block(cad_domain::BlockId(0));
    session
        .capture_point_with_snap(point(0.0, 0.0), None)
        .expect("the pick is captured");
    let error = app
        .execute(
            &mut session,
            command(CommandId::ConfirmMeasurement, CommandPayload::None),
        )
        .expect_err("block space is not an observation space");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_perspective_view_cannot_yield_a_reported_world_point() {
    let (mut app, mut session) = id_session();
    {
        let viewport = app.workspace.viewports.get_mut(&ViewportId(1)).unwrap();
        viewport.camera = viewport
            .camera
            .enter_3d(None, camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)
            .expect("the perspective camera is valid");
    }
    session
        .capture_point_with_snap(point(0.0, 0.0), None)
        .expect("the pick is captured");
    let error = app
        .execute(
            &mut session,
            command(CommandId::ConfirmMeasurement, CommandPayload::None),
        )
        .expect_err("a perspective view has no single world point under the cursor");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
}

#[test]
fn paper_space_is_reported_as_paper() {
    let (mut app, mut session) = application_with_layouts();
    app.execute(
        &mut session,
        id_command(CommandPayload::MeasureTool(MeasurementToolKind::Id)),
    )
    .expect("ID starts its capture");
    session.active_space = SpaceId::Paper(LayoutId(0));
    session
        .capture_point_with_snap(point(1.0, 1.0), None)
        .expect("the pick is captured");
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::ConfirmMeasurement, CommandPayload::None),
        )
        .expect("the confirm reports the point");
    assert_eq!(
        outcome.point_report.expect("a report exists").space,
        IdSpace::Paper
    );
}
