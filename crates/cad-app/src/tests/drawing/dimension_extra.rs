//! DIMROTATED create-command contract tests (AutoCAD 2026).
//!
//! The rotated linear dimension uses the **existing** `DimensionKind::Linear {
//! rotation }` subtype and the shared synthesis, so the numeric contracts are the
//! synthesis's own measurement formula: the projection of the origin separation
//! onto the dimension line's direction,
//! `|cos θ · Δx + sin θ · Δy|`. The angle arrives in the payload in radians (the
//! host converts the session `dimRotation` degrees exactly once).
//!
//! Pinned here:
//!
//! * `0` measures the X extent, `π/2` the Y extent and `30°` the 30° projection;
//! * one entity lands in one transaction / one undo step on the active layer;
//! * a non-finite angle, an off-plane triple, coincident origins and a
//!   zero-measurement direction are named refusals that change nothing;
//! * Viewer mode refuses the command before any write.

use super::*;

fn payload(
    first: Point3,
    second: Point3,
    location: Point3,
    rotation_degrees: f64,
) -> CommandPayload {
    CommandPayload::RotatedLinearDimension(Box::new(crate::RotatedLinearDimensionCommand {
        first,
        second,
        location,
        rotation: rotation_degrees.to_radians(),
    }))
}

fn stored(app: &Application, object: ObjectId) -> DimensionGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the dimension entity");
    assert_eq!(entity.object.type_key, "AcDbDimension");
    match &entity.geometry {
        SemanticGeometry::Dimension(dimension) => (**dimension).clone(),
        other => panic!("expected a dimension, got {other:?}"),
    }
}

fn linear_points(dimension: &DimensionGeometry) -> (Point3, Point3, Point3) {
    match &dimension.definition().points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => (*first, *second, *dimension_line),
        other => panic!("expected Linear points, got {other:?}"),
    }
}

fn rotation_of(dimension: &DimensionGeometry) -> f64 {
    match dimension.kind() {
        DimensionKind::Linear { rotation } => rotation,
        other => panic!("expected a Linear kind, got {other:?}"),
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

#[test]
fn zero_degrees_measures_the_x_extent_and_pi_over_two_the_y_extent() {
    let (mut app, mut session) = drawing_app();
    let first = point(1.0, 2.0);
    let second = point(11.0, 9.0);
    let location = point(6.0, 15.0);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimRotated,
        payload(first, second, location, 0.0),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 1);
    let horizontal = stored(&app, out.objects[0]);
    assert_eq!(linear_points(&horizontal), (first, second, location));
    assert!(
        rotation_of(&horizontal).abs() < 1e-12,
        "a 0 degree dimension line is horizontal"
    );
    assert!(
        approx(horizontal.measurement(), 10.0),
        "0 degrees measures Δx: {}",
        horizontal.measurement()
    );

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimRotated,
        payload(first, second, location, 90.0),
    )
    .unwrap();
    let vertical = stored(&app, out.objects[0]);
    assert!(
        approx(rotation_of(&vertical), std::f64::consts::FRAC_PI_2),
        "90 degrees stores π/2 radians"
    );
    assert!(
        approx(vertical.measurement(), 7.0),
        "90 degrees measures Δy: {}",
        vertical.measurement()
    );
}

#[test]
fn thirty_degrees_measures_the_projection_onto_the_rotated_direction() {
    let (mut app, mut session) = drawing_app();
    let first = point(0.0, 0.0);
    // Δx = 10, Δy = 10: the 30° projection is 10·cos30 + 10·sin30.
    let second = point(10.0, 10.0);
    let location = point(4.0, 12.0);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimRotated,
        payload(first, second, location, 30.0),
    )
    .unwrap();
    let dimension = stored(&app, out.objects[0]);
    let expected = 10.0 * 30.0_f64.to_radians().cos() + 10.0 * 30.0_f64.to_radians().sin();
    assert!(
        approx(dimension.measurement(), expected),
        "30 degrees measures |cos30·Δx + sin30·Δy| = {expected}, got {}",
        dimension.measurement()
    );
    assert!(
        approx(rotation_of(&dimension), 30.0_f64.to_radians()),
        "the stored rotation is exactly the payload radians"
    );
    // The location pick only places the dimension line; it never changes the
    // angle (another location keeps the same rotation and measurement).
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimRotated,
        payload(first, second, point(4.0, -12.0), 30.0),
    )
    .unwrap();
    let moved = stored(&app, out.objects[0]);
    assert!(approx(rotation_of(&moved), 30.0_f64.to_radians()));
    assert!(approx(moved.measurement(), expected));
    // A negative angle is legal and is not normalised: its direction is
    // (cos(-30), sin(-30)), so the projection is 10·cos30 − 10·sin30.
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimRotated,
        payload(first, second, location, -30.0),
    )
    .unwrap();
    let negative = stored(&app, out.objects[0]);
    assert!(approx(rotation_of(&negative), -30.0_f64.to_radians()));
    let expected_negative =
        10.0 * (-30.0_f64).to_radians().cos() + 10.0 * (-30.0_f64).to_radians().sin();
    assert!(approx(negative.measurement(), expected_negative));
}

#[test]
fn one_transaction_one_undo_step_and_one_entity() {
    let (mut app, mut session) = drawing_app();
    let before_revision = revision(&app);
    let before_entities = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    let before_undo = undo_depth(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimRotated,
        payload(point(0.0, 0.0), point(10.0, 0.0), point(5.0, 5.0), 30.0),
    )
    .unwrap();
    let changes = out.changes.expect("change set");
    assert_eq!(changes.before, before_revision);
    assert_eq!(changes.after.0, before_revision.0 + 1, "one revision bump");
    assert!(!changes.is_empty(), "the insert publishes a change");
    assert_eq!(undo_depth(&app), before_undo + 1, "one undo step");
    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before_entities + 1
    );
    // Undo removes it again in one step.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before_entities,
        "one undo removes the created dimension"
    );
}

#[test]
fn refusals_change_nothing_and_name_their_reason() {
    let (mut app, mut session) = drawing_app();
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    let origin = point(0.0, 0.0);
    let cases: Vec<(CommandPayload, &str)> = vec![
        // Coincident extension-line origins.
        (
            payload(origin, origin, point(1.0, 1.0), 30.0),
            "dimrotated.origins",
        ),
        // Off the shared +Z plane.
        (
            payload(
                origin,
                Point3 {
                    x: 10.0,
                    y: 0.0,
                    z: 3.0,
                },
                point(5.0, 5.0),
                0.0,
            ),
            "dimrotated.off_plane",
        ),
        // The 90° direction projects two X-only origins onto the same point.
        (
            payload(origin, point(10.0, 0.0), point(5.0, 5.0), 90.0),
            "dimrotated.degenerate",
        ),
    ];
    for (payload, code) in cases {
        let error = execute(&mut app, &mut session, CommandId::DimRotated, payload).unwrap_err();
        assert!(
            format!("{error:?}").contains(code),
            "expected {code}, got {error:?}"
        );
        assert_eq!(revision(&app), before_revision, "{code} bumps no revision");
        assert_eq!(undo_depth(&app), before_undo, "{code} records no undo");
    }
    // A non-finite angle is refused by name (the typed channel also refuses it,
    // so this pins the payload path).
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimRotated,
        CommandPayload::RotatedLinearDimension(Box::new(crate::RotatedLinearDimensionCommand {
            first: origin,
            second: point(10.0, 0.0),
            location: point(5.0, 5.0),
            rotation: f64::NAN,
        })),
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("dimrotated.rotation"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before_revision);
}

#[test]
fn dimrotated_is_work_only_and_refused_in_viewer_mode() {
    assert!(CommandId::DimRotated.requires_work_mode());
    let (mut app, mut session) = drawing_app();
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimRotated,
        payload(point(0.0, 0.0), point(10.0, 0.0), point(5.0, 5.0), 30.0),
    )
    .unwrap_err();
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), before);
}
