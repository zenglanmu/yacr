//! ELLIPSE create-command contract tests (ADR 0015 decision A + D, P3a).
//!
//! The three picks are AutoCAD's **axis-endpoint method**: `p1`/`p2` are the two
//! endpoints of the first axis and `p3` gives the other half-axis distance. These
//! tests pin the decision-D rule that is not in dispute — the longer semi-axis is
//! stored as `major_axis` (so `ratio <= 1`) while the shape is identical — plus
//! the explicit refusals (non-finite, degenerate first axis, third pick on the
//! first axis, non-coplanar `+Z`, mismatched payload, missing active layer) that
//! change nothing, and one entity / one transaction / one undo step.

use super::*;

/// Read the stored ellipse `(center, major_axis, ratio, start, sweep)` of an
/// entity, asserting it is on layer 0 in model space.
fn ellipse_frame(app: &Application, object: ObjectId) -> (Point3, Point3, f64, f64, f64) {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the ellipse entity");
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbEllipse");
    match &entity.geometry {
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            assert_eq!(
                *normal,
                p3(0.0, 0.0, 1.0),
                "the axis-endpoint method is a +Z construction"
            );
            (*center, *major_axis, *ratio, *start, *sweep)
        }
        other => panic!("expected an ellipse, got {other:?}"),
    }
}

/// The two semi-axis lengths `{major, minor}` of a stored ellipse.
fn semi_axes(major_axis: Point3, ratio: f64) -> (f64, f64) {
    let major = length(major_axis);
    (major, major * ratio)
}

fn length(v: Point3) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn create_ellipse_inserts_one_entity_and_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateEllipse,
        CommandPayload::Points(vec![point(-2.0, 0.0), point(2.0, 0.0), point(0.0, 1.0)]),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one entity for one ellipse");

    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.before, before);
    assert_eq!(cs.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(cs.changes.len(), 1, "one transaction");

    let (center, major_axis, ratio, start, sweep) = ellipse_frame(&app, outcome.objects[0]);
    assert_eq!(center, point(0.0, 0.0));
    let major = length(major_axis);
    assert_close(major, 2.0);
    assert_close(ratio, 0.5);
    assert_close(start, 0.0);
    assert_close(sweep, std::f64::consts::TAU);

    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");

    // Undo removes the ellipse exactly; redo restores it.
    let entity_id = EntityId(outcome.objects[0].0);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(entity_id)
        .is_none());
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(entity_id)
        .is_some());
}

#[test]
fn ellipse_shorter_first_axis_swaps_axes_and_ratio_stays_le_one() {
    // The first axis (p1 -> p2, length 2 => semi-axis a = 1) is SHORTER than the
    // half-axis the third pick gives (b = 3). Decision D stores the longer one as
    // `major_axis`, so ratio = a/b = 1/3 and the stored major axis is the
    // perpendicular (y) direction, not the clicked first axis. The shape (the two
    // semi-axis lengths) is exactly preserved.
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateEllipse,
        CommandPayload::Points(vec![point(-1.0, 0.0), point(1.0, 0.0), point(0.0, 3.0)]),
    )
    .unwrap();
    let (center, major_axis, ratio, ..) = ellipse_frame(&app, outcome.objects[0]);

    // Centre is the midpoint of the first axis.
    assert_eq!(center, point(0.0, 0.0));
    // ratio <= 1 by construction (the DXF-legal invariant the create path owns).
    assert!(ratio <= 1.0, "ratio must stay <= 1, got {ratio}");
    assert!(ratio > 0.0);
    // The stored major axis is the LONGER one: length 3, along the perpendicular
    // (y) direction the third pick defined, not the clicked x axis.
    assert_close(length(major_axis), 3.0);
    assert_close(major_axis.x, 0.0);
    assert_close(major_axis.y.abs(), 3.0);
    assert!(major_axis.y.abs() > major_axis.x.abs());
    // The shape is identical: semi-axes {a, b} = {1, 3}, whichever is major.
    let (major_len, minor_len) = semi_axes(major_axis, ratio);
    assert_close(minor_len, 1.0);
    let mut axes = [major_len, minor_len];
    axes.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert_close(axes[0], 1.0);
    assert_close(axes[1], 3.0);
}

#[test]
fn ellipse_keeps_the_first_axis_as_major_when_it_is_longer() {
    // The mirrored case: the first axis (semi-axis a = 4) is longer than the third
    // pick's half-axis (b = 1), so no swap happens: major_axis = (p2 - p1)/2 and
    // ratio = b/a = 1/4.
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateEllipse,
        CommandPayload::Points(vec![point(-4.0, 0.0), point(4.0, 0.0), point(0.0, 1.0)]),
    )
    .unwrap();
    let (center, major_axis, ratio, ..) = ellipse_frame(&app, outcome.objects[0]);
    assert_eq!(center, point(0.0, 0.0));
    assert!(ratio <= 1.0);
    assert_close(length(major_axis), 4.0);
    assert_close(major_axis.x.abs(), 4.0);
    assert_close(major_axis.y, 0.0);
    assert_close(ratio, 0.25);
    let (major_len, minor_len) = semi_axes(major_axis, ratio);
    assert_close(major_len, 4.0);
    assert_close(minor_len, 1.0);
}

#[test]
fn create_ellipse_refuses_degenerate_and_non_finite_picks_without_change() {
    let cases: &[(&str, Vec<Point3>)] = &[
        (
            "coincident first-axis endpoints",
            vec![point(3.0, 3.0), point(3.0, 3.0), point(0.0, 4.0)],
        ),
        (
            "third pick on the first axis",
            vec![point(0.0, 0.0), point(4.0, 0.0), point(2.0, 0.0)],
        ),
        (
            "non-finite first pick",
            vec![point(f64::NAN, 0.0), point(4.0, 0.0), point(2.0, 3.0)],
        ),
        (
            "non-finite third pick",
            vec![point(0.0, 0.0), point(4.0, 0.0), point(f64::INFINITY, 3.0)],
        ),
        (
            "non-coplanar +Z pick",
            vec![point(0.0, 0.0), point(4.0, 0.0), p3(2.0, 3.0, 5.0)],
        ),
    ];
    for (label, picks) in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateEllipse,
            CommandPayload::Points(picks.clone()),
        )
        .unwrap_err();
        assert!(
            matches!(err, CadError::InvalidInput(_)),
            "{label}: expected an InvalidInput refusal, got {err:?}"
        );
        assert_eq!(revision(&app), rev, "{label}: refusal must not write");
        assert!(!app.can_undo(&DocumentId(1)), "{label}");
    }
}

#[test]
fn create_ellipse_refuses_an_incomplete_pick_list_and_a_wrong_payload() {
    // Fewer than three picks, and a non-`Points` payload, are explicit refusals
    // (ELLIPSE's payload is the construction input, like CIRCLE/RECTANG — never a
    // pre-built geometry).
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    for picks in [
        vec![point(0.0, 0.0)],
        vec![point(0.0, 0.0), point(4.0, 0.0)],
    ] {
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateEllipse,
            CommandPayload::Points(picks),
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)));
    }
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateEllipse,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: point(3.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        })),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_ellipse_on_a_missing_layer_is_refused_by_the_shared_funnel() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateEllipse,
        CommandPayload::Points(vec![point(-1.0, 0.0), point(1.0, 0.0), point(0.0, 3.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}
