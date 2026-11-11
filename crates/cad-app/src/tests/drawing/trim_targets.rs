//! TRIM/EXTEND contract tests: the ELLIPSE targets and the named target-kind
//! refusals added in the 2026-11-11 completion batch.
//!
//! Split out of `trim_extend_curves.rs` so both test files stay inside the
//! code-size limit. The shared fixtures/assertions (`app_of`, `trim`, `extend`,
//! `snapshot`, ...) are reused from that module.

use super::trim_extend_curves::{
    angle_near, app_of, extend, geometry_of, line, near, snapshot, trim, undo_depth,
};
use super::*;

const PI: f64 = std::f64::consts::PI;
const TAU: f64 = std::f64::consts::TAU;

// ---------------------------------------------------------------------------
// The 2026-11-11 completion batch: ELLIPSE targets for TRIM/EXTEND, the lifted
// arc half-turn bound, and the named target-kind refusals.
// ---------------------------------------------------------------------------

/// An ELLIPSE target at the origin: major radius 4 along +x, ratio 0.5, spanning
/// `[start, start + sweep]` (a full ellipse when `sweep == TAU`).
fn ellipse_target(id: u128, start: f64, sweep: f64, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(4.0, 0.0, 0.0),
            ratio: 0.5,
            start,
            sweep,
        },
        order,
    )
}

/// `(center, major_axis, ratio, start, sweep)` of an ELLIPSE.
fn expect_ellipse(app: &Application, id: u128) -> (Point3, Point3, f64, f64, f64) {
    match geometry_of(app, id) {
        SemanticGeometry::Ellipse {
            center,
            major_axis,
            ratio,
            start,
            sweep,
            ..
        } => (center, major_axis, ratio, start, sweep),
        other => panic!("expected an ELLIPSE, got {other:?}"),
    }
}

/// A solid HATCH over a 4x4 square (used only as an unsupported *target*).
fn solid_hatch() -> SemanticGeometry {
    let hatch = HatchGeometry::new(
        vec![HatchLoop::new(
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
            vec![0.0; 4],
            HatchLoopFlags::default(),
        )],
        HatchPlane::default(),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::UserDefined,
    )
    .expect("valid hatch");
    SemanticGeometry::Hatch(Box::new(hatch))
}

#[test]
fn trim_a_full_ellipse_target_keeps_the_picked_elliptical_arc() {
    // The ellipse is cut by the vertical line x = 2 at the eccentric angles
    // ±60°; the pick sits on the +x end (eccentric angle 0), so the kept piece is
    // the elliptical arc from -60° to +60° — the same stored frame, ratio and
    // normal, only `start`/`sweep` change (never a refit, never a polyline).
    let mut app = app_of(vec![
        ellipse_target(1, 0.0, TAU, 0),
        line(2, point(2.0, -5.0), point(2.0, 5.0), 1),
    ]);
    let outcome = trim(&mut app, 1, &[2], point(4.0, 0.0)).unwrap();
    assert_eq!(outcome.diagnostics[0].code, "drawing.trim");
    let (center, major_axis, ratio, start, sweep) = expect_ellipse(&app, 1);
    assert_eq!(center, point(0.0, 0.0), "the centre is unchanged");
    assert_eq!(major_axis, p3(4.0, 0.0, 0.0), "the major axis is unchanged");
    assert!(near(ratio, 0.5), "the ratio is unchanged: {ratio}");
    assert!(
        angle_near(start, -PI / 3.0),
        "the kept piece starts at -60°: {start}"
    );
    assert!(
        near(sweep.abs(), TAU / 3.0),
        "the kept piece spans 120°: {sweep}"
    );
    assert_eq!(undo_depth(&app), 1, "one undo step");
}

#[test]
fn trim_an_elliptical_arc_target_at_one_cut_keeps_the_pick_side() {
    // Upper half of the same ellipse (eccentric 0..180°), cut once by x = 0 at 90°.
    // The pick at the +x end keeps 0..90° (an elliptical quarter arc).
    let mut app = app_of(vec![
        ellipse_target(1, 0.0, PI, 0),
        line(2, point(0.0, -5.0), point(0.0, 5.0), 1),
    ]);
    trim(&mut app, 1, &[2], point(4.0, 0.0)).unwrap();
    let (_, _, _, start, sweep) = expect_ellipse(&app, 1);
    assert!(angle_near(start, 0.0), "start {start}");
    assert!(near(sweep, PI / 2.0), "sweep {sweep}");
}

#[test]
fn extend_an_elliptical_arc_target_to_the_nearest_line_ahead() {
    // Elliptical quarter arc 0..90° (from (4,0) to (0,2)); the end is picked and
    // the line y = -1 crosses the ellipse at 210° and 330°, so the nearest crossing
    // ahead (210°) grows the sweep from 90° to 210° on the same stored frame.
    let mut app = app_of(vec![
        ellipse_target(1, 0.0, PI / 2.0, 0),
        line(2, point(-6.0, -1.0), point(6.0, -1.0), 1),
    ]);
    extend(&mut app, 1, &[2], point(0.0, 2.0)).unwrap();
    let (center, major_axis, ratio, start, sweep) = expect_ellipse(&app, 1);
    assert_eq!(center, point(0.0, 0.0));
    assert_eq!(major_axis, p3(4.0, 0.0, 0.0));
    assert!(near(ratio, 0.5));
    assert!(angle_near(start, 0.0), "start {start}");
    assert!(
        near(sweep, 7.0 * PI / 6.0),
        "the end grows from 90° to 210°: {sweep}"
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn extend_refuses_a_full_ellipse_target_because_it_has_no_ends() {
    let mut app = app_of(vec![
        ellipse_target(1, 0.0, TAU, 0),
        line(2, point(-6.0, -1.0), point(6.0, -1.0), 1),
    ]);
    let before = snapshot(&app);
    let error = extend(&mut app, 1, &[2], point(4.0, 0.0)).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("closed and has no endpoint"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before, "a refusal changes nothing");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_refuses_a_polyline_target_by_name() {
    let mut app = app_of(vec![
        ent(
            1,
            SemanticGeometry::Polyline {
                points: vec![point(0.0, 0.0), point(5.0, 0.0), point(10.0, 0.0)],
                bulges: vec![0.0, 0.0],
                closed: false,
            },
            0,
        ),
        line(2, point(5.0, -2.0), point(5.0, 2.0), 1),
    ]);
    let before = snapshot(&app);
    let error = trim(&mut app, 1, &[2], point(2.0, 0.0)).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("trim.target_polyline"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn extend_refuses_a_polyline_target_by_name() {
    let mut app = app_of(vec![
        ent(
            1,
            SemanticGeometry::Polyline {
                points: vec![point(0.0, 0.0), point(5.0, 0.0)],
                bulges: vec![0.0],
                closed: false,
            },
            0,
        ),
        line(2, point(9.0, -2.0), point(9.0, 2.0), 1),
    ]);
    let before = snapshot(&app);
    let error = extend(&mut app, 1, &[2], point(5.0, 0.0)).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("extend.target_polyline"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_and_extend_refuse_a_hatch_target_by_name() {
    // A HATCH's display is derived from its loops and fill: there is no source
    // curve to cut or extend, so both commands refuse by name and write nothing.
    let mut app = app_of(vec![
        ent(1, solid_hatch(), 0),
        line(2, point(5.0, -2.0), point(5.0, 2.0), 1),
    ]);
    let before = snapshot(&app);
    for error in [
        trim(&mut app, 1, &[2], point(2.0, 1.0)).unwrap_err(),
        extend(&mut app, 1, &[2], point(4.0, 1.0)).unwrap_err(),
    ] {
        assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
        assert!(
            error.to_string().contains(".target_hatch"),
            "the named target refusal: {error:?}"
        );
    }
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_refuses_an_opaque_target_by_name() {
    let mut app = app_of(vec![
        ent(
            1,
            SemanticGeometry::Opaque {
                type_key: "ACAD_PROXY_ENTITY".into(),
                version: 1,
                payload: vec![1, 2, 3],
            },
            0,
        ),
        line(2, point(5.0, -2.0), point(5.0, 2.0), 1),
    ]);
    let before = snapshot(&app);
    let error = trim(&mut app, 1, &[2], point(2.0, 1.0)).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains("trim.target_opaque"),
        "{error:?}"
    );
    assert_eq!(snapshot(&app), before);
}
