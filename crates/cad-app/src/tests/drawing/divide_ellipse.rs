//! `DIVIDE` on an elliptical arc (a partial `ELLIPSE`): equal **arc length**,
//! not equal eccentric-angle parameter.
//!
//! Split out of `divide.rs` for the code-size limit; it reuses that module's
//! fixtures through `use super::*`.

use super::*;

#[test]
fn an_elliptical_arc_divides_by_equal_arc_length_not_by_parameter() {
    // A partial ELLIPSE (eccentric angle 0 → π/2, major 5, minor 2.5). DIVIDE
    // puts N−1 marks on an open curve by *arc length*, so the midpoint mark is
    // past the eccentric-angle midpoint (the minor-axis half of the arc is
    // shorter than the major-axis half).
    let ellipse = curve_entity(
        1,
        LayerId(0),
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(5.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::FRAC_PI_2,
        },
        "AcDbEllipse",
        0,
    );
    let (mut app, mut session) = app_with(vec![ellipse]);
    set_segments(&mut session, 2.0);
    let outcome =
        divide(&mut app, &mut session, vec![ref_of(1)]).expect("the elliptical arc divides");
    assert_eq!(outcome.objects.len(), 1, "an open curve places N−1 marks");
    let marks = point_positions(&app);
    assert_eq!(marks.len(), 1);
    let mark = marks[0];
    // On the ellipse: (x/5)² + (y/2.5)² = 1.
    let residual = (mark.x / 5.0).powi(2) + (mark.y / 2.5).powi(2) - 1.0;
    assert!(residual.abs() < 1e-3, "mark {mark:?} is not on the ellipse");
    assert!(
        mark.y > 1.85,
        "an equal-parameter midpoint would be y = 1.768; the equal-arc-length mark is \
         farther along the minor axis, got {mark:?}"
    );
    assert!(matches!(
        entity(&app, 1).geometry,
        SemanticGeometry::Ellipse { .. }
    ));
    assert_eq!(undo_depth(&app), 1);
}
