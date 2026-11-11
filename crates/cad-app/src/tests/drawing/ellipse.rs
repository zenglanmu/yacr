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
    // Fewer than three picks, and a payload of the wrong kind, are explicit
    // refusals. ELLIPSE's click payload is the axis-endpoint construction input, and
    // a pre-built geometry of another kind is never coerced into an ellipse.
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
        CommandPayload::Geometry(Box::new(SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        })),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_ellipse_refuses_a_fourth_or_fifth_pick_instead_of_dropping_it() {
    // The axis-endpoint construction is exactly three picks. A longer list used to
    // be silently truncated into a full ellipse; it is now refused by name, because
    // "an arc was asked for but a full ellipse was drawn" is a fabricated success.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateEllipse,
        CommandPayload::Points(vec![
            point(-2.0, 0.0),
            point(2.0, 0.0),
            point(0.0, 1.0),
            point(1.0, 1.0),
        ]),
    )
    .unwrap_err();
    match &err {
        CadError::InvalidInput(message) => assert!(
            message.starts_with("ellipse.payload_length"),
            "the extra pick must be refused by name: {message}"
        ),
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

/// Run one `Geometry(Ellipse)` payload and return the stored
/// `(center, normal, major_axis, ratio, start, sweep)`.
fn execute_ellipse_geometry(
    app: &mut Application,
    session: &mut SessionState,
    geometry: SemanticGeometry,
) -> CadResult<(Point3, Point3, Point3, f64, f64, f64)> {
    let outcome = execute(
        app,
        session,
        CommandId::CreateEllipse,
        CommandPayload::Geometry(Box::new(geometry)),
    )?;
    let (center, normal, major_axis, ratio, start, sweep) =
        ellipse_frame_full(app, outcome.objects[0]);
    Ok((center, normal, major_axis, ratio, start, sweep))
}

/// Like [`ellipse_frame`] but also returns the stored normal (an authored ellipse
/// may legitimately be off the `+Z` plane — that is the rotation frame).
fn ellipse_frame_full(
    app: &Application,
    object: ObjectId,
) -> (Point3, Point3, Point3, f64, f64, f64) {
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
        } => (*center, *normal, *major_axis, *ratio, *start, *sweep),
        other => panic!("expected an ellipse, got {other:?}"),
    }
}

#[test]
fn create_ellipse_arc_mode_stores_the_requested_parameter_span() {
    // ELLIPSE arc mode: the same stored frame as the full ellipse, with a `start`
    // and a `sweep` shorter than one turn. A negative sweep is the clockwise
    // direction and is stored verbatim (never normalised away).
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let (center, normal, major_axis, ratio, start, sweep) = execute_ellipse_geometry(
        &mut app,
        &mut session,
        SemanticGeometry::Ellipse {
            center: point(1.0, 2.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: point(4.0, 0.0),
            ratio: 0.5,
            start: std::f64::consts::FRAC_PI_4,
            sweep: std::f64::consts::FRAC_PI_2,
        },
    )
    .unwrap();
    assert_eq!(center, point(1.0, 2.0));
    assert_eq!(normal, p3(0.0, 0.0, 1.0));
    assert_eq!(major_axis, point(4.0, 0.0));
    assert_close(ratio, 0.5);
    assert_close(start, std::f64::consts::FRAC_PI_4);
    assert_close(sweep, std::f64::consts::FRAC_PI_2);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");
    assert!(revision(&app).0 > before.0, "one revision bump");

    // The clockwise arc keeps its sign.
    let (.., start, sweep) = execute_ellipse_geometry(
        &mut app,
        &mut session,
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: point(2.0, 0.0),
            ratio: 0.25,
            start: 0.0,
            sweep: -std::f64::consts::FRAC_PI_3,
        },
    )
    .unwrap();
    assert_close(start, 0.0);
    assert_close(sweep, -std::f64::consts::FRAC_PI_3);
}

#[test]
fn ellipse_rotation_about_the_major_axis_is_the_projection_rule() {
    // Rotation `R` about the major axis: DXF stores the rotated ellipse as a tilted
    // extrusion plus the projected ratio, so the *orthographic projection* of the
    // tilted ellipse onto the `+Z` plane must be exactly the `+Z` ellipse with the
    // same major axis and `ratio · cos R`. This pins the claim the command
    // documents (the rotated frame is representable, not squashed):
    // it samples the stored tilted ellipse and compares it with the projected one.
    let ratio = 0.6;
    let major = 3.0;
    let rotation = 0.7_f64; // radians about the major (+X) axis
    let (mut app, mut session) = drawing_app();
    let stored = execute_ellipse_geometry(
        &mut app,
        &mut session,
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, rotation.sin(), rotation.cos()),
            major_axis: point(major, 0.0),
            // The tilted frame keeps the same minor/major relation in 3D; the
            // projection is what shortens it.
            ratio,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
    )
    .unwrap();
    let (center, normal, major_axis, stored_ratio, ..) = stored;
    assert_close(stored_ratio, ratio);
    assert_close(normal.y, rotation.sin());
    assert_close(normal.z, rotation.cos());

    // 3D points of the stored (tilted) ellipse, and their `+Z` projection.
    let minor_dir = {
        // cross(normal, major_axis) normalised: the ellipse's own minor direction.
        let cross = (
            normal.y * major_axis.z - normal.z * major_axis.y,
            normal.z * major_axis.x - normal.x * major_axis.z,
            normal.x * major_axis.y - normal.y * major_axis.x,
        );
        let len = (cross.0 * cross.0 + cross.1 * cross.1 + cross.2 * cross.2).sqrt();
        (cross.0 / len, cross.1 / len, cross.2 / len)
    };
    let minor = major * ratio;
    let projected_ratio = ratio * rotation.cos();
    let mut tilted_off_plane = false;
    for step in 0..16 {
        let t = std::f64::consts::TAU * step as f64 / 16.0;
        let (sin, cos) = t.sin_cos();
        let point_3d = (
            center.x + major_axis.x * cos + minor_dir.0 * minor * sin,
            center.y + major_axis.y * cos + minor_dir.1 * minor * sin,
            center.z + major_axis.z * cos + minor_dir.2 * minor * sin,
        );
        // The projected ellipse: +Z normal, same major axis, ratio · cos R.
        let expected = (
            center.x + major * cos,
            center.y + major * projected_ratio * sin,
        );
        // Orthographic projection onto the +Z plane drops z; x/y must be the flat
        // ellipse with the shortened minor axis.
        assert_close(point_3d.0, expected.0);
        assert_close(point_3d.1, expected.1);
        if point_3d.2.abs() > 1e-9 {
            tilted_off_plane = true;
        }
    }
    assert!(
        tilted_off_plane,
        "the rotated ellipse really leaves the +Z plane (it is not pre-flattened)"
    );
}

#[test]
fn create_ellipse_refuses_an_unrepresentable_authored_frame_without_change() {
    // Every authored refusal is named `ellipse.<reason>` and writes nothing.
    let cases: &[(&str, &str, SemanticGeometry)] = &[
        (
            "ratio above one (the stored major axis must be the longer semi-axis)",
            "ellipse.ratio",
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: point(2.0, 0.0),
                ratio: 2.0,
                start: 0.0,
                sweep: std::f64::consts::TAU,
            },
        ),
        (
            "zero ratio (rotated by 90° about the major axis)",
            "ellipse.ratio",
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: p3(0.0, 1.0, 0.0),
                major_axis: point(2.0, 0.0),
                ratio: 0.0,
                start: 0.0,
                sweep: std::f64::consts::TAU,
            },
        ),
        (
            "zero sweep",
            "ellipse.sweep",
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: point(2.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: 0.0,
            },
        ),
        (
            "sweep beyond one turn",
            "ellipse.sweep",
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: point(2.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: 5.0 * std::f64::consts::PI,
            },
        ),
        (
            "non-finite start",
            "ellipse.sweep",
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: point(2.0, 0.0),
                ratio: 0.5,
                start: f64::NAN,
                sweep: 1.0,
            },
        ),
        (
            "normal parallel to the major axis",
            "ellipse.frame",
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: p3(1.0, 0.0, 0.0),
                major_axis: point(2.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: 1.0,
            },
        ),
        (
            "collapsed major axis",
            "ellipse.axis",
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: point(0.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: 1.0,
            },
        ),
        (
            "zero-length normal",
            "ellipse.invalid_frame",
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: point(0.0, 0.0),
                major_axis: point(2.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: 1.0,
            },
        ),
        (
            "non-finite centre",
            "ellipse.invalid_frame",
            SemanticGeometry::Ellipse {
                center: p3(0.0, f64::INFINITY, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: point(2.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: 1.0,
            },
        ),
    ];
    for (label, code, geometry) in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateEllipse,
            CommandPayload::Geometry(Box::new(geometry.clone())),
        )
        .unwrap_err();
        match &err {
            CadError::InvalidInput(message) => assert!(
                message.starts_with(code),
                "{label}: expected {code}, got {message}"
            ),
            other => panic!("{label}: expected an InvalidInput refusal, got {other:?}"),
        }
        assert_eq!(revision(&app), rev, "{label}: refusal must not write");
        assert!(!app.can_undo(&DocumentId(1)), "{label}");
    }
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
