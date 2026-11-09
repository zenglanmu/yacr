//! Extended `PEDIT` options: FIT (open and closed), SPLINE (open and closed),
//! DECURVE, JOIN, conversion and UNDO.

use super::*;

// --- FIT ---------------------------------------------------------------------

#[test]
fn fit_writes_tangent_continuous_arcs_through_every_vertex() {
    let points = vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0), p3(7.0, 2.0, 0.0)];
    let fixture = SemanticGeometry::Polyline {
        points: points.clone(),
        bulges: vec![0.0, 0.0],
        closed: false,
    };
    let (mut app, mut session) = build(vec![ent(1, fixture, 9)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);

    let outcome = pedit_edit(&mut app, &mut session, PeditEditOption::Fit).expect("FIT succeeds");
    assert_eq!(outcome.diagnostics[0].code, "pedit.applied");
    let SemanticGeometry::Polyline {
        points: fitted_points,
        bulges,
        closed,
    } = geometry(&app, 1)
    else {
        panic!("FIT keeps the entity a POLYLINE");
    };
    assert_eq!(fitted_points, points, "FIT keeps every original vertex");
    assert!(!closed);
    assert_eq!(bulges.len(), 2, "one arc per original segment");
    assert!(bulges.iter().all(|bulge| bulge.is_finite()));
    for pair in 0..bulges.len() - 1 {
        let delta_in = 2.0 * bulges[pair].atan();
        let delta_out = 2.0 * bulges[pair + 1].atan();
        let end_tangent = rotate(
            chord_direction(fitted_points[pair], fitted_points[pair + 1]),
            delta_in,
        );
        let start_tangent = rotate(
            chord_direction(fitted_points[pair + 1], fitted_points[pair + 2]),
            -delta_out,
        );
        let dot = end_tangent.0 * start_tangent.0 + end_tangent.1 * start_tangent.1;
        assert!(
            (dot - 1.0).abs() <= 1e-9,
            "segments {pair}/{} are not tangent-continuous: {dot}",
            pair + 1
        );
    }
    let after = entity(&app, 1);
    assert_eq!(after.id, EntityId(1));
    assert_eq!(after.layer, LayerId(7));
    assert_eq!(after.draw_order, 9);
    assert_ne!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 1);

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(
        geometry(&app, 1),
        SemanticGeometry::Polyline {
            points,
            bulges: vec![0.0, 0.0],
            closed: false,
        }
    );
}

#[test]
fn fit_writes_a_closed_tangent_continuous_ring() {
    // A square: every turn is +π/2, so the closed fit is four +90° arcs (the
    // circle through the corners), i.e. δ = π/4 and bulge = tan(π/8).
    let points = vec![
        p3(0.0, 0.0, 0.0),
        p3(4.0, 0.0, 0.0),
        p3(4.0, 4.0, 0.0),
        p3(0.0, 4.0, 0.0),
    ];
    let fixture = SemanticGeometry::Polyline {
        points: points.clone(),
        bulges: vec![0.0; 4],
        closed: true,
    };
    let (mut app, mut session) = build(vec![ent(1, fixture, 3)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);

    pedit_edit(&mut app, &mut session, PeditEditOption::Fit).expect("closed FIT succeeds");
    let SemanticGeometry::Polyline {
        points: fitted_points,
        bulges,
        closed,
    } = geometry(&app, 1)
    else {
        panic!("FIT keeps the entity a POLYLINE");
    };
    assert_eq!(fitted_points, points, "FIT keeps every ring vertex");
    assert!(closed, "FIT keeps the ring closed");
    assert_eq!(bulges.len(), 4, "one arc per ring segment");
    let expected = (std::f64::consts::FRAC_PI_8).tan();
    for (index, bulge) in bulges.iter().enumerate() {
        assert!(
            (bulge - expected).abs() <= 1e-9,
            "segment {index} bulge {bulge} != {expected}"
        );
    }
    // Tangent continuity around the loop, including the closing vertex.
    for vertex in 0..4 {
        let incoming = (vertex + 3) % 4;
        let delta_in = 2.0 * bulges[incoming].atan();
        let delta_out = 2.0 * bulges[vertex].atan();
        let end_tangent = rotate(
            chord_direction(fitted_points[incoming], fitted_points[vertex]),
            delta_in,
        );
        let start_tangent = rotate(
            chord_direction(fitted_points[vertex], fitted_points[(vertex + 1) % 4]),
            -delta_out,
        );
        let dot = end_tangent.0 * start_tangent.0 + end_tangent.1 * start_tangent.1;
        assert!(
            (dot - 1.0).abs() <= 1e-9,
            "vertex {vertex} is not tangent-continuous: {dot}"
        );
    }
    assert_ne!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 1);

    // DECURVE restores the closed frame (bulges cleared, ring still closed).
    pedit_edit(&mut app, &mut session, PeditEditOption::Decurve).expect("DECURVE succeeds");
    assert_eq!(
        geometry(&app, 1),
        SemanticGeometry::Polyline {
            points,
            bulges: vec![0.0; 4],
            closed: true,
        },
        "the closed fit round-trips through DECURVE"
    );
    assert_eq!(undo_depth(&app), 2);
}

#[test]
fn fit_writes_a_closed_ring_for_an_odd_vertex_count() {
    // An equilateral triangle: the cyclic system is uniquely solvable and each
    // segment becomes a +120° arc, i.e. δ = π/3 and bulge = tan(π/6).
    let points = vec![
        p3(0.0, 0.0, 0.0),
        p3(4.0, 0.0, 0.0),
        p3(2.0, 3.4641016151377544, 0.0),
    ];
    let (mut app, mut session) = build(
        vec![ent(
            1,
            SemanticGeometry::Polyline {
                points,
                bulges: vec![0.0; 3],
                closed: true,
            },
            0,
        )],
        AppMode::Work,
    );
    select(&mut session, &[1]);
    pedit_edit(&mut app, &mut session, PeditEditOption::Fit).expect("closed FIT succeeds");
    let SemanticGeometry::Polyline { bulges, closed, .. } = geometry(&app, 1) else {
        panic!("FIT keeps the entity a POLYLINE");
    };
    assert!(closed);
    let expected = (std::f64::consts::FRAC_PI_6).tan();
    assert_eq!(bulges.len(), 3);
    assert!(
        bulges.iter().all(|bulge| (bulge - expected).abs() <= 1e-9),
        "every segment is a 120° arc: {bulges:?}"
    );
}

#[test]
fn fit_refuses_a_closed_ring_whose_turns_cannot_close_and_a_half_turn_corner() {
    // A generic quadrilateral: an even ring whose alternating turn residual does
    // not vanish has no single-arc-per-segment tangent-continuous fit.
    let (mut app, mut session) = build(
        vec![ent(
            1,
            SemanticGeometry::Polyline {
                points: vec![
                    p3(0.0, 0.0, 0.0),
                    p3(4.0, 0.0, 0.0),
                    p3(4.0, 3.0, 0.0),
                    p3(1.0, 2.0, 0.0),
                ],
                bulges: vec![0.0; 4],
                closed: true,
            },
            0,
        )],
        AppMode::Work,
    );
    select(&mut session, &[1]);
    let rev = revision(&app);
    let error = pedit_edit(&mut app, &mut session, PeditEditOption::Fit).unwrap_err();
    assert_unsupported(error, "pedit.fit_closure");
    assert_wrote_nothing(&app, rev);

    // An open right-angle corner still needs a half-turn arc: refused by name.
    let (mut app, mut session) = build(
        vec![ent(
            1,
            SemanticGeometry::Polyline {
                points: vec![p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0), p3(4.0, 4.0, 0.0)],
                bulges: vec![0.0, 0.0],
                closed: false,
            },
            0,
        )],
        AppMode::Work,
    );
    select(&mut session, &[1]);
    let rev = revision(&app);
    let error = pedit_edit(&mut app, &mut session, PeditEditOption::Fit).unwrap_err();
    assert_unsupported(error, "pedit.fit_tangent");
    assert_wrote_nothing(&app, rev);
}

// --- SPLINE / DECURVE --------------------------------------------------------

#[test]
fn spline_stores_the_vertices_as_a_clamped_control_frame() {
    let points = vec![
        p3(0.0, 0.0, 0.0),
        p3(2.0, 3.0, 0.0),
        p3(5.0, 1.0, 0.0),
        p3(9.0, 0.0, 0.0),
    ];
    let (mut app, mut session) = build(
        vec![ent(
            1,
            SemanticGeometry::Polyline {
                points: points.clone(),
                bulges: vec![0.0, 0.0, 0.0],
                closed: false,
            },
            0,
        )],
        AppMode::Work,
    );
    select(&mut session, &[1]);

    pedit_edit(
        &mut app,
        &mut session,
        PeditEditOption::Spline { precision: 32 },
    )
    .expect("SPLINE succeeds");
    let after = entity(&app, 1);
    assert_eq!(after.object.type_key, "AcDbSpline");
    let SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
    } = after.geometry
    else {
        panic!("SPLINE stores a B-spline");
    };
    assert_eq!(degree, 3);
    assert_eq!(control_points, points, "vertices are the control frame");
    assert_eq!(knots, clamped_uniform_knots(4, 3));
    assert_eq!(weights, vec![1.0; 4]);
    let curve = NurbsCurve::new(3, control_points.clone(), knots, weights).unwrap();
    assert_point_close(curve.evaluate(0.0), points[0], "spline start");
    assert_point_close(
        curve.evaluate(1.0),
        *points.last().expect("four points"),
        "spline end",
    );
    assert_eq!(undo_depth(&app), 1);

    // DECURVE restores the exact control frame as a straight polyline.
    pedit_edit(&mut app, &mut session, PeditEditOption::Decurve).expect("DECURVE succeeds");
    let restored = entity(&app, 1);
    assert_eq!(restored.object.type_key, "AcDbPolyline");
    assert_eq!(
        restored.geometry,
        SemanticGeometry::Polyline {
            points,
            bulges: vec![0.0, 0.0, 0.0],
            closed: false,
        }
    );
    assert_eq!(
        undo_depth(&app),
        2,
        "SPLINE and DECURVE are one undo step each"
    );
}

#[test]
fn spline_stores_a_closed_polyline_as_a_repeated_closed_control_frame() {
    let points = vec![
        p3(0.0, 0.0, 0.0),
        p3(4.0, 0.0, 0.0),
        p3(4.0, 4.0, 0.0),
        p3(0.0, 4.0, 0.0),
    ];
    let (mut app, mut session) = build(
        vec![ent(
            1,
            SemanticGeometry::Polyline {
                points: points.clone(),
                bulges: vec![0.0; 4],
                closed: true,
            },
            0,
        )],
        AppMode::Work,
    );
    select(&mut session, &[1]);
    let rev = revision(&app);

    pedit_edit(
        &mut app,
        &mut session,
        PeditEditOption::Spline { precision: 8 },
    )
    .expect("closed SPLINE succeeds");
    let after = entity(&app, 1);
    assert_eq!(after.object.type_key, "AcDbSpline");
    let SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
    } = after.geometry.clone()
    else {
        panic!("SPLINE stores a B-spline");
    };
    assert_eq!(degree, 3, "degree is min(3, n-1) over the ring vertices");
    let mut expected_points = points.clone();
    expected_points.push(points[0]);
    assert_eq!(
        control_points, expected_points,
        "the closed frame repeats the first vertex at the end"
    );
    assert_eq!(knots, clamped_uniform_knots(5, 3));
    assert_eq!(weights, vec![1.0; 5]);
    // The clamped spline starts and ends on the repeated first control point.
    let curve = NurbsCurve::new(3, control_points.clone(), knots.clone(), weights.clone()).unwrap();
    assert_point_close(curve.evaluate(0.0), points[0], "closed spline start");
    assert_point_close(curve.evaluate(1.0), points[0], "closed spline end");
    assert_ne!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 1);

    // DECURVE reads the repeated control point as the closed form.
    pedit_edit(&mut app, &mut session, PeditEditOption::Decurve).expect("DECURVE succeeds");
    let restored = entity(&app, 1);
    assert_eq!(restored.object.type_key, "AcDbPolyline");
    assert_eq!(
        restored.geometry,
        SemanticGeometry::Polyline {
            points,
            bulges: vec![0.0; 4],
            closed: true,
        },
        "DECURVE restores the closed frame"
    );
    assert_eq!(undo_depth(&app), 2);
}

#[test]
fn spline_refuses_out_of_range_precision() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);
    let error = pedit_edit(
        &mut app,
        &mut session,
        PeditEditOption::Spline { precision: 100 },
    )
    .unwrap_err();
    assert_invalid(error, "pedit.spline_precision");
    assert_wrote_nothing(&app, rev);
}

#[test]
fn decurve_clears_the_fit_bulges_and_keeps_the_frame() {
    let SemanticGeometry::Polyline { points, .. } = closed_bulged() else {
        unreachable!()
    };
    let (mut app, mut session) = build(vec![ent(1, closed_bulged(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    pedit_edit(&mut app, &mut session, PeditEditOption::Decurve).expect("DECURVE succeeds");
    assert_eq!(
        geometry(&app, 1),
        SemanticGeometry::Polyline {
            points,
            bulges: vec![0.0, 0.0, 0.0, 0.0],
            closed: true,
        }
    );
    assert_eq!(undo_depth(&app), 1);
}

// --- JOIN --------------------------------------------------------------------

#[test]
fn join_welds_touching_line_arc_and_polyline_into_the_base() {
    let base = straight_piece(p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0));
    let line = SemanticGeometry::Line {
        start: p3(4.0, 0.0, 0.0),
        end: p3(8.0, 0.0, 0.0),
    };
    let arc = SemanticGeometry::Arc {
        center: p3(8.0, 4.0, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        radius: 4.0,
        start: -std::f64::consts::FRAC_PI_2,
        sweep: std::f64::consts::FRAC_PI_2,
    };
    let tail = straight_piece(p3(12.0, 4.0, 0.0), p3(15.0, 4.0, 0.0));
    let (mut app, mut session) = build(
        vec![
            ent(1, base.clone(), 5),
            ent(2, line, 6),
            ent(3, arc, 7),
            ent(4, tail, 8),
        ],
        AppMode::Work,
    );
    select(&mut session, &[1, 2, 3, 4]);

    let outcome = pedit_edit(&mut app, &mut session, PeditEditOption::Join).expect("JOIN succeeds");
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    let after = entity(&app, 1);
    assert_eq!(after.object.type_key, "AcDbPolyline");
    let SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
    } = after.geometry
    else {
        panic!("the base stays a POLYLINE");
    };
    assert!(!closed);
    assert_eq!(points.len(), 5);
    assert_point_close(points[0], p3(0.0, 0.0, 0.0), "start");
    assert_point_close(points[1], p3(4.0, 0.0, 0.0), "line junction");
    assert_point_close(points[2], p3(8.0, 0.0, 0.0), "arc start");
    assert_point_close(points[3], p3(12.0, 4.0, 0.0), "arc end");
    assert_point_close(points[4], p3(15.0, 4.0, 0.0), "tail");
    assert_eq!(bulges.len(), 4);
    assert!((bulges[0] - 0.0).abs() <= 1e-12);
    assert!((bulges[1] - 0.0).abs() <= 1e-12);
    assert!(
        (bulges[2] - (std::f64::consts::FRAC_PI_8).tan()).abs() <= 1e-9,
        "the ARC becomes one bulge: {}",
        bulges[2]
    );
    assert!((bulges[3] - 0.0).abs() <= 1e-12);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        1,
        "the three joined sources are deleted"
    );
    assert_eq!(undo_depth(&app), 1);

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(geometry(&app, 1), base);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        4,
        "UNDO restores every joined source"
    );
}

#[test]
fn join_refuses_a_gap_a_closed_base_and_a_closed_source() {
    let base = straight_piece(p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0));
    let (mut app, mut session) = build(
        vec![
            ent(1, base.clone(), 0),
            ent(2, straight_piece(p3(5.0, 0.0, 0.0), p3(9.0, 0.0, 0.0)), 1),
        ],
        AppMode::Work,
    );
    select(&mut session, &[1, 2]);
    let rev = revision(&app);
    let error = pedit_edit(&mut app, &mut session, PeditEditOption::Join).unwrap_err();
    assert_invalid(error, "pedit.join.not_contiguous");
    assert_wrote_nothing(&app, rev);

    let (mut app, mut session) = build(
        vec![
            ent(1, closed_bulged(), 0),
            ent(2, straight_piece(p3(0.0, 0.0, 0.0), p3(-4.0, 0.0, 0.0)), 1),
        ],
        AppMode::Work,
    );
    select(&mut session, &[1, 2]);
    let rev = revision(&app);
    let error = pedit_edit(&mut app, &mut session, PeditEditOption::Join).unwrap_err();
    assert_invalid(error, "pedit.join.closed");
    assert_wrote_nothing(&app, rev);
}

// --- capability refusals and conversion --------------------------------------

#[test]
fn width_ltype_gen_and_edit_vertex_are_named_refusals() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);

    let error = pedit_edit(&mut app, &mut session, PeditEditOption::Width(1.5)).unwrap_err();
    assert_unsupported(error, "pedit.width_unsupported");
    let error = pedit_edit(&mut app, &mut session, PeditEditOption::Width(-1.0)).unwrap_err();
    assert_invalid(error, "pedit.width_invalid");
    let error = pedit_edit(
        &mut app,
        &mut session,
        PeditEditOption::LtypeGen { enabled: true },
    )
    .unwrap_err();
    assert_unsupported(error, "pedit.ltype_gen_unsupported");
    let error = pedit_edit(&mut app, &mut session, PeditEditOption::EditVertex).unwrap_err();
    assert_unsupported(error, "pedit.edit_vertex_unsupported");
    assert_wrote_nothing(&app, rev);
}

#[test]
fn the_peditaccept_answer_converts_a_line_then_applies_the_option() {
    let (mut app, mut session) = build(vec![ent(1, line(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    let rev = revision(&app);
    let error = pedit_edit(&mut app, &mut session, PeditEditOption::Reverse).unwrap_err();
    assert_unsupported(error, "pedit.not_polyline");
    assert_wrote_nothing(&app, rev);

    pedit_edit_with(&mut app, &mut session, PeditEditOption::Reverse, true)
        .expect("convert = true turns the LINE into a polyline");
    let after = entity(&app, 1);
    assert_eq!(after.object.type_key, "AcDbPolyline");
    assert_eq!(
        after.geometry,
        SemanticGeometry::Polyline {
            points: vec![p3(10.0, 0.0, 0.0), p3(0.0, 0.0, 0.0)],
            bulges: vec![0.0],
            closed: false,
        }
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn the_peditaccept_answer_converts_an_arc_and_keeps_its_bulge() {
    let arc = SemanticGeometry::Arc {
        center: p3(0.0, 0.0, 0.0),
        normal: p3(0.0, 0.0, 1.0),
        radius: 5.0,
        start: 0.0,
        sweep: std::f64::consts::FRAC_PI_2,
    };
    let (mut app, mut session) = build(vec![ent(1, arc, 0)], AppMode::Work);
    select(&mut session, &[1]);
    pedit_edit_with(&mut app, &mut session, PeditEditOption::Reverse, true)
        .expect("convert = true turns the ARC into a bulged polyline");
    let SemanticGeometry::Polyline { points, bulges, .. } = geometry(&app, 1) else {
        panic!("the ARC becomes a POLYLINE");
    };
    assert_point_close(points[0], p3(0.0, 5.0, 0.0), "reversed end");
    assert_point_close(points[1], p3(5.0, 0.0, 0.0), "reversed start");
    assert_eq!(bulges.len(), 1);
    assert!(
        (bulges[0] + (std::f64::consts::FRAC_PI_8).tan()).abs() <= 1e-9,
        "REVERSE negates the converted arc bulge: {}",
        bulges[0]
    );
}

// --- UNDO --------------------------------------------------------------------

#[test]
fn undo_option_applies_the_document_undo_step_and_names_an_empty_history() {
    let (mut app, mut session) = build(vec![ent(1, open_bulged(), 0)], AppMode::Work);
    select(&mut session, &[1]);
    pedit(&mut app, &mut session, PeditOption::Close).expect("CLOSE succeeds");
    assert_eq!(undo_depth(&app), 1);

    let outcome = pedit_edit(&mut app, &mut session, PeditEditOption::Undo)
        .expect("PEDIT UNDO applies the document undo step");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "pedit.undo"),
        "the document-undo divergence is named"
    );
    assert_eq!(undo_depth(&app), 0);
    let SemanticGeometry::Polyline { closed, .. } = geometry(&app, 1) else {
        panic!("the polyline is restored");
    };
    assert!(!closed, "UNDO restored the open ring");

    let error = pedit_edit(&mut app, &mut session, PeditEditOption::Undo).unwrap_err();
    assert_invalid(error, "pedit.undo_empty");
}

#[test]
fn join_prepends_and_reverses_picks_at_either_end_of_the_base() {
    let base = straight_piece(p3(0.0, 0.0, 0.0), p3(4.0, 0.0, 0.0));
    let before = SemanticGeometry::Line {
        start: p3(-4.0, 0.0, 0.0),
        end: p3(0.0, 0.0, 0.0),
    };
    let after = SemanticGeometry::Line {
        start: p3(4.0, 0.0, 0.0),
        end: p3(8.0, 0.0, 0.0),
    };
    // This pick runs right-to-left: it must be reversed to continue the chain.
    let reversed = SemanticGeometry::Line {
        start: p3(12.0, 0.0, 0.0),
        end: p3(8.0, 0.0, 0.0),
    };
    let (mut app, mut session) = build(
        vec![
            ent(1, base, 0),
            ent(2, before, 1),
            ent(3, after, 2),
            ent(4, reversed, 3),
        ],
        AppMode::Work,
    );
    select(&mut session, &[1, 2, 3, 4]);
    pedit_edit(&mut app, &mut session, PeditEditOption::Join).expect("JOIN reaches both ends");
    let SemanticGeometry::Polyline { points, bulges, .. } = geometry(&app, 1) else {
        panic!("the base stays a POLYLINE");
    };
    assert_eq!(points.len(), 5);
    for (index, x) in [-4.0, 0.0, 4.0, 8.0, 12.0].into_iter().enumerate() {
        assert_point_close(points[index], p3(x, 0.0, 0.0), &format!("vertex {index}"));
    }
    assert_eq!(bulges, vec![0.0; 4]);
    assert_eq!(undo_depth(&app), 1);
}
