//! `ORTHO`: the session-scoped orthographic drafting constraint.
//!
//! AutoCAD's `ORTHO` restricts the **next point entry** to a horizontal or
//! vertical direction from the tool's previous captured point. This module owns
//! the one pure rule; the session applies it at the two places a point can enter
//! or be previewed (see [`crate::SessionState::capture_point`] and the preview
//! projections), never a second copy of the decision.
//!
//! The rule, with previous point `p` and raw input `q`:
//!
//! * `dx = q.x - p.x`, `dy = q.y - p.y`;
//! * `|dx| >= |dy|` → the constrained point keeps `p.y` (a horizontal move);
//! * otherwise → it keeps `p.x` (a vertical move).
//!
//! An **exact tie** (`|dx| == |dy|`) therefore resolves to **horizontal**, which
//! is this build's declared deterministic choice (the comparison is `>=`, so
//! the result never depends on floating-point luck).
//!
//! `z` is always the raw input's `z`: `ORTHO` is a 2D drafting aid and never
//! moves a point in Z. This build documents that explicitly instead of implying
//! a 3D constraint it does not have.
//!
//! **Known, deliberate ordering deviation:** the host resolves object snapping
//! *first* and passes the already-snapped point here (see
//! [`crate::host::HostController::capture_tool_point`]), so `ORTHO` constrains a
//! snapped point. AutoCAD applies ortho before its own snap; this build documents
//! the difference rather than hiding it.
//!
//! What `ORTHO` deliberately does **not** include: polar tracking, relative or
//! absolute coordinate input modes, per-axis `SNAP`/grid snap, or any persistence.
//! It is session/display state only (no database write, no history entry, no
//! ChangeSet) and is allowed in Viewer and Work.

use cad_domain::Point3;

/// Constrain `raw` to the horizontal or vertical ray through `previous`.
///
/// See the module docs for the exact rule, the horizontal tie rule and the Z rule.
/// The caller is responsible for the "no previous point" case: this function
/// always needs a previous point, and the session only calls it when one exists.
pub fn ortho_constrain(previous: Point3, raw: Point3) -> Point3 {
    let dx = raw.x - previous.x;
    let dy = raw.y - previous.y;
    if dx.abs() >= dy.abs() {
        // Horizontal: keep the previous Y, take the raw X (and the raw Z).
        Point3 {
            x: raw.x,
            y: previous.y,
            z: raw.z,
        }
    } else {
        // Vertical: keep the previous X, take the raw Y (and the raw Z).
        Point3 {
            x: previous.x,
            y: raw.y,
            z: raw.z,
        }
    }
}

/// Apply the `ORTHO` switch and the "no previous point" rule to one candidate
/// point.
///
/// This is the single entry point the session uses for both captures and
/// previews:
///
/// * `enabled == false` → the raw point is returned unchanged (the switch is a
///   pure pass-through when off);
/// * `previous == None` → the raw point is returned unchanged (the first pick of
///   a tool has no direction to constrain against);
/// * otherwise → [`ortho_constrain`].
pub fn ortho_constrain_point(enabled: bool, previous: Option<Point3>, raw: Point3) -> Point3 {
    match (enabled, previous) {
        (true, Some(previous)) => ortho_constrain(previous, raw),
        _ => raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn a_wider_x_delta_keeps_the_previous_y() {
        let constrained = ortho_constrain(p(0.0, 0.0), p(10.0, 3.0));
        assert_eq!(constrained, p(10.0, 0.0));
        // The reverse direction is horizontal too.
        let constrained = ortho_constrain(p(10.0, 5.0), p(-4.0, 7.0));
        assert_eq!(constrained, p(-4.0, 5.0));
    }

    #[test]
    fn a_wider_y_delta_keeps_the_previous_x() {
        let constrained = ortho_constrain(p(0.0, 0.0), p(3.0, 10.0));
        assert_eq!(constrained, p(0.0, 10.0));
        let constrained = ortho_constrain(p(10.0, 5.0), p(12.0, -8.0));
        assert_eq!(constrained, p(10.0, -8.0));
    }

    #[test]
    fn an_exact_tie_resolves_to_horizontal() {
        // `|dx| == |dy|` is the documented tie: horizontal wins by the `>=`.
        let constrained = ortho_constrain(p(0.0, 0.0), p(10.0, 10.0));
        assert_eq!(constrained, p(10.0, 0.0));
        let constrained = ortho_constrain(p(0.0, 0.0), p(-10.0, -10.0));
        assert_eq!(constrained, p(-10.0, 0.0));
    }

    #[test]
    fn a_zero_delta_is_unchanged() {
        let point = p(4.0, -6.0);
        assert_eq!(ortho_constrain(point, point), point);
    }

    #[test]
    fn z_is_the_raw_input_z_never_the_previous_z() {
        let previous = Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        };
        let raw = Point3 {
            x: 10.0,
            y: 2.0,
            z: 7.0,
        };
        assert_eq!(ortho_constrain(previous, raw).z, 7.0);
        // The vertical branch preserves Z the same way.
        let raw = Point3 {
            x: 2.0,
            y: 10.0,
            z: 9.0,
        };
        assert_eq!(ortho_constrain(previous, raw).z, 9.0);
    }

    #[test]
    fn a_disabled_switch_or_a_missing_previous_point_passes_the_raw_point_through() {
        let previous = p(0.0, 0.0);
        let raw = p(10.0, 3.0);
        assert_eq!(ortho_constrain_point(false, Some(previous), raw), raw);
        assert_eq!(ortho_constrain_point(true, None, raw), raw);
        assert_eq!(ortho_constrain_point(false, None, raw), raw);
        // Enabled with a previous point is the one constraining case.
        assert_eq!(
            ortho_constrain_point(true, Some(previous), raw),
            p(10.0, 0.0)
        );
    }
}
