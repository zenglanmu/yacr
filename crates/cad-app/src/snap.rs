//! `SNAP`: the session-scoped **grid snap** quantiser (AutoCAD's `SNAP`, F9).
//!
//! This is **not** object snap (`OSNAP`, ADR 0014) and **not** the display grid
//! overlay (`GRID`, F7). It is the third, independent drafting aid: while the
//! session switch is on, a captured point is rounded to the nearest multiple of
//! the session **snap spacing** in **X and Y**.
//!
//! The rule, with raw input `q` and spacing `s > 0`:
//!
//! * `x = round(q.x / s) * s`, `y = round(q.y / s) * s`;
//! * `z` is the raw input's `z` — grid snap is a 2D drafting aid and never moves
//!   a point in Z (the same rule [`crate::ortho`] documents);
//! * `round` is Rust's [`f64::round`], i.e. **half values round away from zero**:
//!   `x = 1.5·s → 2·s` and `x = -1.5·s → -2·s`. Exact halves are the only
//!   ambiguous case and this build declares the away-from-zero reading instead of
//!   leaving it to floating-point luck.
//!
//! The spacing is a **session sysvar** ([`DEFAULT_SNAP_SPACING`] initially, set
//! with the typed `SNAPUNIT <value>` line). AutoCAD's initial spacing depends on
//! the drawing template's units, which this repository cannot know: the default
//! here is our declared **10.0 drawing units**, stated instead of guessed from
//! the document.
//!
//! ## Order of the drafting constraints
//!
//! The full order of a point entry is:
//!
//! 1. **object snap** — resolved by the host before the session sees the point
//!    ([`crate::host::HostController::capture_tool_point`]);
//! 2. **grid snap** — this module, applied inside
//!    [`crate::SessionState::capture_point`];
//! 3. **ORTHO** — applied last, so the orthographic constraint still decides the
//!    final axis ([`crate::ortho`]).
//!
//! The preview projections call the **same** session helper, so a rubber band can
//! never disagree with the point the next click commits. Entity-pick steps
//! (TRIM/EXTEND, CHAMFER/FILLET, the DIM* radial family, TEXTEDIT) are not point
//! entries: they bypass this path exactly like they bypass `ORTHO`, and their
//! preview stays raw.
//!
//! What grid snap deliberately does **not** include: per-axis spacing
//! (`SNAPUNIT` is one square spacing for both axes), a rotation angle
//! (`SNAPANG`), isometric snap, polar snap, or any persistence. It is
//! session/display state only (no database write, no history entry, no
//! ChangeSet) and is allowed in Viewer and Work.

use cad_domain::Point3;

/// The declared default snap spacing in drawing units.
///
/// AutoCAD's initial `SNAPUNIT` depends on the drawing template's unit setup,
/// which this repository cannot know from the drawing. 10.0 drawing units is this
/// build's declared choice, documented instead of inferred.
pub const DEFAULT_SNAP_SPACING: f64 = 10.0;

/// Whether `spacing` is an acceptable `SNAPUNIT` value: finite and strictly
/// positive.
///
/// Zero, negative, `NaN` and the infinities are refused. The check exists so the
/// quantiser can never divide by zero: the session setter validates before it
/// stores, and [`snap_point`] independently refuses an invalid spacing.
pub fn snap_spacing_is_valid(spacing: f64) -> bool {
    spacing.is_finite() && spacing > 0.0
}

/// Round one coordinate to the nearest multiple of `spacing` (half away from
/// zero).
///
/// `spacing` must already be valid ([`snap_spacing_is_valid`]); the caller is the
/// session, which validates on write. A quantised value that is not finite (only
/// reachable at extreme magnitude/spacing ratios) falls back to the raw
/// coordinate rather than storing a non-finite point.
fn quantise(value: f64, spacing: f64) -> f64 {
    let quantised = (value / spacing).round() * spacing;
    if quantised.is_finite() {
        quantised
    } else {
        value
    }
}

/// Quantise `raw` to the grid: `x` and `y` to the nearest multiple of `spacing`,
/// `z` unchanged.
///
/// See the module docs for the half-away-from-zero rule and the Z rule.
pub fn snap_to_grid(spacing: f64, raw: Point3) -> Point3 {
    if !snap_spacing_is_valid(spacing) {
        // Defensive: never divide by zero. The session refuses such a spacing on
        // write, so this arm is unreachable through the product paths.
        return raw;
    }
    Point3 {
        x: quantise(raw.x, spacing),
        y: quantise(raw.y, spacing),
        z: raw.z,
    }
}

/// Apply the `SNAP` switch to one candidate point.
///
/// * `enabled == false` → the raw point is returned unchanged (the switch is a
///   pure pass-through while off);
/// * an invalid spacing → the raw point is returned unchanged (see
///   [`snap_to_grid`]);
/// * otherwise → [`snap_to_grid`].
pub fn snap_point(enabled: bool, spacing: f64, raw: Point3) -> Point3 {
    if enabled {
        snap_to_grid(spacing, raw)
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn a_point_rounds_to_the_nearest_multiple_in_both_axes() {
        // 12.4 → 10, 18.6 → 20 with a spacing of 10.
        assert_eq!(snap_to_grid(10.0, p(12.4, 18.6)), p(10.0, 20.0));
        // Negative coordinates round to the nearest multiple as well.
        assert_eq!(snap_to_grid(10.0, p(-12.4, -18.6)), p(-10.0, -20.0));
        // A point already on the grid is unchanged.
        assert_eq!(snap_to_grid(10.0, p(20.0, -30.0)), p(20.0, -30.0));
    }

    #[test]
    fn exact_half_values_round_away_from_zero_on_both_signs() {
        // `1.5 * spacing` and `-1.5 * spacing` are the documented tie cases.
        assert_eq!(snap_to_grid(10.0, p(15.0, 0.0)).x, 20.0);
        assert_eq!(snap_to_grid(10.0, p(-15.0, 0.0)).x, -20.0);
        assert_eq!(snap_to_grid(10.0, p(0.0, 5.0)).y, 10.0);
        assert_eq!(snap_to_grid(10.0, p(0.0, -5.0)).y, -10.0);
        // The same rule for a fractional spacing.
        assert_eq!(snap_to_grid(0.5, p(0.75, -0.75)), p(1.0, -1.0));
    }

    #[test]
    fn a_large_or_small_spacing_still_quantises() {
        // A large spacing collapses everything nearby onto the origin multiple.
        assert_eq!(snap_to_grid(1.0e6, p(4.0e5, -4.0e5)), p(0.0, 0.0));
        assert_eq!(snap_to_grid(1.0e6, p(5.5e5, -5.5e5)), p(1.0e6, -1.0e6));
        // A small spacing keeps sub-unit detail.
        assert_eq!(snap_to_grid(0.25, p(1.1, -1.1)), p(1.0, -1.0));
        let tiny = snap_to_grid(1.0e-9, p(3.4e-9, -3.4e-9));
        assert!((tiny.x - 3.0e-9).abs() < 1.0e-22, "{tiny:?}");
        assert!((tiny.y + 3.0e-9).abs() < 1.0e-22, "{tiny:?}");
    }

    #[test]
    fn z_is_never_moved() {
        let raw = Point3 {
            x: 12.4,
            y: 18.6,
            z: 7.25,
        };
        assert_eq!(snap_to_grid(10.0, raw).z, 7.25);
        assert_eq!(snap_point(true, 10.0, raw).z, 7.25);
    }

    #[test]
    fn an_invalid_spacing_is_refused_by_validation_and_never_divides_by_zero() {
        for spacing in [0.0, -0.0, -10.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(!snap_spacing_is_valid(spacing), "{spacing:?}");
            // Even if a caller bypasses the session validation, no division by
            // zero happens: the raw point passes through unchanged.
            let raw = p(12.4, 18.6);
            assert_eq!(snap_to_grid(spacing, raw), raw, "{spacing:?}");
            assert_eq!(snap_point(true, spacing, raw), raw, "{spacing:?}");
        }
        for spacing in [10.0, 0.5, 1.0e-9, 1.0e12, f64::MIN_POSITIVE] {
            assert!(snap_spacing_is_valid(spacing), "{spacing:?}");
        }
    }

    #[test]
    fn a_disabled_switch_passes_the_raw_point_through() {
        let raw = p(12.4, 18.6);
        assert_eq!(snap_point(false, 10.0, raw), raw);
        assert_eq!(snap_point(true, 10.0, raw), p(10.0, 20.0));
    }
}
