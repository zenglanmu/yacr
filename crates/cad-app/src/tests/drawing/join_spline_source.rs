//! Source-object `JOIN` mode with a SPLINE first pick: a touching LINE/ARC is
//! welded into one exact rational SPLINE, and a POLYLINE pick is refused.
//!
//! Split out of `join_extended.rs` for the code-size limit; it reuses that
//! module's fixtures through `use super::*`.

use super::*;
use cad_geometry::NurbsCurve;

// --- source-object SPLINE mode: mixed LINE/ARC continuation ------------------

#[test]
fn a_spline_source_welds_a_touching_line_exactly() {
    // The SPLINE is picked FIRST, so it is the source object; AutoCAD's SPLINE
    // source also accepts a touching LINE. The exact rational weld prepends the
    // LINE span, so the result evaluates as LINE then the unchanged spline.
    let source = spline_geometry(
        vec![
            point(4.0, 0.0),
            point(5.0, 2.0),
            point(7.0, 2.0),
            point(9.0, 0.0),
        ],
        3,
    );
    let SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
    } = source
    else {
        unreachable!()
    };
    let original = NurbsCurve::new(degree as usize, control_points, knots, weights).unwrap();
    let (mut app, mut session) = app_with(vec![
        line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 4.0, 0.0), 0),
        part(
            2,
            spline_geometry(
                vec![
                    point(4.0, 0.0),
                    point(5.0, 2.0),
                    point(7.0, 2.0),
                    point(9.0, 0.0),
                ],
                3,
            ),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            1,
        ),
    ]);
    // Spline first: it is the source object, not the LINE.
    select(&mut app, &mut session, &[2, 1]);
    join(&mut app, &mut session).expect("the SPLINE source accepts a touching LINE");

    let (degree_out, knots_out, control_points, weights) = spline_cps(&app);
    assert_eq!(degree_out, 3);
    assert_eq!(control_points.len(), 8);
    assert_eq!(knots_out.len(), 8 + 3 + 1);
    assert_eq!(weights, vec![1.0; 8], "the LINE elevation is polynomial");
    assert!(near(control_points[0], point(0.0, 0.0)));
    // The welded curve is the LINE span [0, 1] then the source spline [1, 2].
    assert!(near(evaluate_weld(&app, 0.5), point(2.0, 0.0)));
    assert!(near(evaluate_weld(&app, 1.0), point(4.0, 0.0)));
    for step in 0..=10 {
        let u = step as f64 / 10.0;
        let welded = evaluate_weld(&app, 1.0 + u);
        let expected = original.evaluate(u);
        assert!(
            near(welded, expected),
            "spline span at u={u}: {welded:?} vs {expected:?}"
        );
    }
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 1, "one undo step");
}

#[test]
fn a_spline_source_refuses_a_polyline_pick_by_name() {
    // A POLYLINE is not part of the SPLINE source's accepted set: the named
    // refusal writes nothing.
    let (mut app, mut session) = app_with(vec![
        part(
            1,
            spline_geometry(
                vec![
                    point(4.0, 0.0),
                    point(5.0, 2.0),
                    point(7.0, 2.0),
                    point(9.0, 0.0),
                ],
                3,
            ),
            LayerId(0),
            SpaceId::Model,
            "AcDbSpline",
            0,
        ),
        part(
            2,
            polyline_geometry(vec![point(9.0, 0.0), point(12.0, 0.0)], vec![0.0], false),
            LayerId(0),
            SpaceId::Model,
            "AcDbPolyline",
            1,
        ),
    ]);
    select(&mut app, &mut session, &[1, 2]);
    let before = revision(&app);
    let error = join(&mut app, &mut session).unwrap_err();
    assert!(
        format!("{error:?}").contains("join.mixed_kinds"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}
