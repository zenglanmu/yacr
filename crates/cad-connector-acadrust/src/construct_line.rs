//! Reduction of an infinite construct line (RAY / XLINE) to a finite segment.
//!
//! `cad-db` has no infinite line geometry, so the only way to display an XLINE
//! or RAY is a finite segment. This module owns that reduction and its honest
//! verdict: **every** successful reduction is `Partial` with a named reason
//! ([`support::XLINE_SEGMENT_REASON`], or [`support::XLINE_DEFAULT_BOX_REASON`]
//! when the clip extents were guessed). The stored entity keeps its source class
//! name (`AcDbXline`/`AcDbRay`) in `DbObject::type_key`, which is what lets the
//! exporter report the same loss instead of claiming an exact round-trip.
//!
//! Split out of `primitives` to keep that module under the file-size limit.

use super::*;

/// An infinite construct line (RAY / XLINE) clipped to an XY box.
///
/// A fixed-viewport render has no per-view clipping, so the line is clipped to
/// the drawing's model bounds (tracked as entities are imported). When no other
/// geometry has been seen yet a default 1000-unit box is used.
///
/// **Every** successful reduction is `Partial`: the database has no infinite
/// line geometry, so the finite segment is an approximation whose length follows
/// the clip box rather than the source entity (see
/// [`support::XLINE_SEGMENT_REASON`] and [`support::XLINE_DEFAULT_BOX_REASON`]).
/// Only the refusal/degradation paths differ in the kind of loss, never in
/// completeness.
pub(crate) fn ray_semantics(
    base: Point3,
    direction: Point3,
    is_ray: bool,
    bounds: Option<(Point3, Point3)>,
) -> (SemanticGeometry, Completeness) {
    let opaque = || SemanticGeometry::Opaque {
        type_key: if is_ray {
            "AcDbRay".into()
        } else {
            "AcDbXline".into()
        },
        version: 1,
        payload: Vec::new(),
    };
    if !cad_geometry::is_finite(base)
        || !cad_geometry::is_finite(direction)
        || cad_geometry::length(direction) < 1e-12
    {
        return (
            opaque(),
            Completeness::Partial(vec!["ray/xline direction is degenerate".into()]),
        );
    }
    let direction = cad_geometry::normalize(direction);
    let (min, max, margin, reason) = match bounds {
        Some((lo, hi)) => {
            let margin = (cad_geometry::distance(lo, hi) * 0.05).max(1.0);
            (lo, hi, margin, support::XLINE_SEGMENT_REASON)
        }
        None => (
            Point3 {
                x: base.x - 1.0e3,
                y: base.y - 1.0e3,
                z: 0.0,
            },
            Point3 {
                x: base.x + 1.0e3,
                y: base.y + 1.0e3,
                z: 0.0,
            },
            0.0,
            support::XLINE_DEFAULT_BOX_REASON,
        ),
    };
    let min = Point3 {
        x: min.x - margin,
        y: min.y - margin,
        z: 0.0,
    };
    let max = Point3 {
        x: max.x + margin,
        y: max.y + margin,
        z: 0.0,
    };
    let span = ((max.x - min.x).abs().max((max.y - min.y).abs()) * 4.0).max(1.0);
    let a = if is_ray {
        base
    } else {
        cad_geometry::add(base, cad_geometry::scale(direction, -span))
    };
    let b = cad_geometry::add(base, cad_geometry::scale(direction, span));
    let completeness = Completeness::Partial(vec![reason.to_string()]);
    match cad_geometry::clip_segment_to_xy_rect(a, b, (min.x, min.y), (max.x, max.y)) {
        Some((start, end)) if cad_geometry::distance(start, end) > 1e-9 => {
            (SemanticGeometry::Line { start, end }, completeness)
        }
        _ => (
            opaque(),
            Completeness::Partial(vec![
                "ray/xline does not intersect the drawing bounds".into()
            ]),
        ),
    }
}

impl<'a> ImporterBuilder<'a> {
    /// Reduce a RAY/XLINE using the drawing extent seen so far.
    ///
    /// The named loss is also recorded once per import (`import.construct_line`),
    /// so a drawing that contains a construct line is not reported `Complete` as a
    /// whole merely because its entities are drawable.
    pub(crate) fn construct_line_semantics(
        &mut self,
        base: Point3,
        direction: Point3,
        is_ray: bool,
    ) -> (SemanticGeometry, Completeness) {
        let (geometry, completeness) = ray_semantics(base, direction, is_ray, self.model_bounds);
        if let Completeness::Partial(reasons) = &completeness {
            let reasons: Vec<&str> = reasons.iter().map(String::as_str).collect();
            self.named_partial("import.construct_line", &reasons);
        }
        (geometry, completeness)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn xline_clips_to_the_box_and_ray_keeps_only_the_positive_half() {
        let bounds = Some((point(-10.0, -10.0), point(10.0, 10.0)));
        let (xline, completeness) = ray_semantics(point(0.0, 0.0), point(1.0, 0.0), false, bounds);
        // An infinite construct line is never a complete import: the stored
        // geometry is a finite segment whose ends follow the clip box.
        assert_eq!(
            completeness,
            Completeness::Partial(vec![support::XLINE_SEGMENT_REASON.into()])
        );
        let SemanticGeometry::Line { start, end } = xline else {
            panic!("expected a clipped line");
        };
        // Clip box plus its 5% margin (the box diagonal here), symmetric about
        // the base point; the reduction is unchanged, only its verdict is.
        let margin = (20.0f64 * 20.0 + 20.0f64 * 20.0).sqrt() * 0.05;
        assert!(approx(start.x, -10.0 - margin), "{start:?} {end:?}");
        assert!(approx(end.x, 10.0 + margin), "{start:?} {end:?}");
        assert!(approx(start.y, 0.0) && approx(end.y, 0.0));

        let (ray, ray_completeness) = ray_semantics(point(0.0, 0.0), point(1.0, 0.0), true, bounds);
        assert_eq!(
            ray_completeness,
            Completeness::Partial(vec![support::XLINE_SEGMENT_REASON.into()])
        );
        let SemanticGeometry::Line { start, end } = ray else {
            panic!("expected a clipped ray");
        };
        assert!(approx(start.x, 0.0), "ray must not start behind its base");
        assert!(approx(end.x, 10.0 + margin), "{start:?} {end:?}");
    }

    #[test]
    fn xline_without_drawing_bounds_still_reports_the_default_box() {
        let (geometry, completeness) = ray_semantics(point(0.0, 0.0), point(1.0, 0.0), false, None);
        assert!(matches!(geometry, SemanticGeometry::Line { .. }));
        assert_eq!(
            completeness,
            Completeness::Partial(vec![support::XLINE_DEFAULT_BOX_REASON.into()])
        );
    }

    #[test]
    fn degenerate_infinite_line_direction_is_partial_not_a_fake_line() {
        let (geometry, completeness) = ray_semantics(point(0.0, 0.0), point(0.0, 0.0), true, None);
        assert!(matches!(geometry, SemanticGeometry::Opaque { .. }));
        assert!(matches!(completeness, Completeness::Partial(_)));
    }
}
