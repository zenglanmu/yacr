//! SPLINE as a **TRIM target**: an exact sub-curve from a sampled cut.
//!
//! A SPLINE target is trimmed by the same plan as a LINE/ARC/CIRCLE target
//! (`super::trim_curves`): the boundary intersections become parameters on the
//! target, the piece containing the projected pick is kept, and the target entity
//! is replaced **in place** (same id, layer, space, display state, one undo step).
//! The one thing that differs is what "the kept piece" is made of.
//!
//! # Two precisions, stated separately
//!
//! * The **cut parameter** comes from `cad_geometry`'s sampled intersection
//!   fallback: a SPLINE has no analytic arm, so the engine flattens the spline
//!   with chord height `extent × 1e-4` of its own control-point box (at most 256
//!   segments, `FALLBACK_CHORD_FRACTION` in `cad_geometry::engine`). The reported
//!   cut point therefore sits at most that chord from the true curve, and
//!   [`SplineTarget::param_of`] accepts the inverted parameter only within twice
//!   that bound ([`SAMPLED_CUT_TOLERANCE_MARGIN`]) instead of pretending the hit
//!   is exact.
//! * The **resulting sub-curve is exact**: [`cad_geometry::sub_curve`] inserts the
//!   two end knots and truncates the homogeneous control polygon (Boehm), so its
//!   endpoints are exact points *of the original spline* at that parameter and its
//!   degree, weights and interior knots are the source's. Nothing is re-interpolated
//!   or refitted and no sample becomes a control point.
//!
//! Those two statements are reported together through the existing
//! `drawing.trim` diagnostic channel: one note per sampled contributing pair (from
//! `super::trim_boundary::note_approximation`) plus one target-side note naming the
//! split ([`SplineTarget::note_exact_sub_curve_on_sampled_cuts`]). A caller must
//! never read the note as "the spline trim is approximate": only the cut parameter
//! is.
//!
//! # What is refused
//!
//! * a definition the NURBS validation rejects, or a shorthand that cannot be
//!   expanded ([`cad_geometry::expand_spline_source`]);
//! * a kept span too short for `degree + 1` control points (the parameter
//!   tolerance case, [`cad_geometry::sub_curve`]);
//! * weight arithmetic that is not representable, instead of normalising it;
//! * EXTEND, which has no continuation semantics at all
//!   ([`no_continuation_semantics`]): moving an end of a NURBS curve would change
//!   its knot vector, and therefore its parameterisation, and no extrapolation
//!   policy exists.
//!
//! A spline target is **not** periodic here: `SemanticGeometry::Spline` carries no
//! closed flag, so a closed spline is treated as an open curve and its seam is
//! simply its start/end (same reasoning as the containment refusal in
//! `super::trim_curves`).

use super::*;
use cad_geometry::{nearest_point_parameter, NurbsCurve, TessellationParams};

/// `cad_geometry`'s sampled-intersection fallback flattens each operand with chord
/// height `extent × this fraction` (mirrored from `FALLBACK_CHORD_FRACTION` in
/// `cad_geometry::engine`; that constant is crate-private, so the value is restated
/// here with the same meaning).
const SAMPLED_CUT_CHORD_FRACTION: f64 = 1e-4;

/// How far past the sampled chord a cut point may sit and still be accepted by the
/// inversion: `2×`, the same margin the SPLINE-boundary contract tests assert.
const SAMPLED_CUT_TOLERANCE_MARGIN: f64 = 2.0;

/// Chord height of the sampling used to measure the target's **world length**, as a
/// fraction of the control box. Only used to convert a world tolerance into this
/// target's parameter units; the length is a polyline estimate, never a claim.
const LENGTH_SAMPLE_CHORD_FRACTION: f64 = 1e-3;

/// Segment budget of the length sampling.
const LENGTH_SAMPLE_MAX_SEGMENTS: usize = 256;

/// Segment budget of the length sampling's lower bound.
const LENGTH_SAMPLE_MIN_SEGMENTS: usize = 8;

/// Rounding slack allowed when a window end lands a hair outside the knot domain.
const SPAN_END_ROUNDING_FRACTION: f64 = 1e-12;

/// A validated SPLINE target: the source curve plus the two derived quantities the
/// plan needs (its expanded geometry and its world length).
#[derive(Debug, Clone)]
pub(super) struct SplineTarget {
    /// The validated, shorthand-expanded source curve.
    curve: NurbsCurve,
    /// The same curve as a stored `SemanticGeometry::Spline`, handed to the
    /// intersector so it never has to re-derive the shorthands itself.
    geometry: SemanticGeometry,
    /// Diagonal of the control-polygon box, i.e. the scale the sampled fallback
    /// uses for this same curve.
    extent: f64,
    /// Polyline length of the target (see [`LENGTH_SAMPLE_CHORD_FRACTION`]).
    length: f64,
}

impl SplineTarget {
    /// Expand and validate a stored SPLINE definition.
    ///
    /// The shorthands (`knots: []`, `weights: []`) are expanded exactly as the
    /// tessellator does; everything else is refused by name (see
    /// `cad_geometry::expand_spline_source`). A failure here is
    /// `InvalidInput` — the stored definition is malformed, not an unsupported
    /// capability.
    pub(super) fn new(
        degree: u32,
        knots: &[f64],
        control_points: &[Point3],
        weights: &[f64],
    ) -> CadResult<Self> {
        let curve = cad_geometry::expand_spline_source(degree, knots, control_points, weights)?;
        let extent = cad_geometry::control_polygon_extent(curve.control_points());
        let length = curve_length(&curve, extent);
        let geometry = SemanticGeometry::Spline {
            degree: curve.degree() as u32,
            knots: curve.knots().to_vec(),
            control_points: curve.control_points().to_vec(),
            weights: curve.weights().to_vec(),
        };
        Ok(Self {
            curve,
            geometry,
            extent,
            length,
        })
    }

    /// The target's world length (a sampled polyline length; see the constant).
    pub(super) fn length(&self) -> f64 {
        self.length
    }

    /// The expanded geometry the cuts are resolved against.
    pub(super) fn geometry(&self) -> SemanticGeometry {
        self.geometry.clone()
    }

    /// The curve point at normalised parameter `u` (`0` = domain start).
    ///
    /// Unlike a LINE or an ARC a spline has no continuation beyond its domain, so
    /// `u` outside `[0, 1]` is clamped to the domain rather than extrapolated.
    /// Every caller here passes a parameter that is already inside it (a cut, a
    /// pick projection, or a window end).
    pub(super) fn point_at(&self, u: f64) -> Point3 {
        self.curve.evaluate(self.parameter(u))
    }

    /// The parameter of a **boundary hit** on this target, or `None` when the point
    /// is not on the curve.
    ///
    /// The inversion is exact ([`cad_geometry::invert_point`]); the tolerance only
    /// absorbs the sampled cut's own chord error, `2 × extent × 1e-4` on top of the
    /// caller's world tolerance. A hit farther away than that is rejected, never
    /// snapped onto the curve: a cut belongs on the target, not near it.
    pub(super) fn param_of(&self, point: Point3, world_tol: f64) -> Option<f64> {
        let tolerance = (self.extent * SAMPLED_CUT_CHORD_FRACTION).max(world_tol)
            * SAMPLED_CUT_TOLERANCE_MARGIN;
        let native = cad_geometry::invert_point(&self.curve, point, tolerance)?;
        Some(self.normalised(native))
    }

    /// The parameter of the **nearest point** to `pick`.
    ///
    /// The host resolves the target with its own pickbox and passes the raw click,
    /// so a near miss is still a pick of this entity — the same rule a LINE (own
    /// perpendicular) and an ARC (own radial direction) follow. No distance bound
    /// applies, and `_world_tol` is unused for the same reason; a spline's nearest
    /// point is the one projection it has.
    pub(super) fn project_param(&self, pick: Point3, _world_tol: f64) -> Option<f64> {
        let (native, _) = nearest_point_parameter(&self.curve, pick)?;
        Some(self.normalised(native))
    }

    /// Rebuild the target as the piece `[u0, u0 + fraction]` of its own span.
    ///
    /// The sub-curve is exact (knot insertion + truncation in `cad_geometry`), so
    /// both ends are points of the original spline; a span that cannot keep
    /// `degree + 1` control points is refused by name instead of returning a
    /// degenerate curve.
    pub(super) fn with_span(&self, u0: f64, fraction: f64) -> Result<SemanticGeometry, String> {
        if !u0.is_finite() || !fraction.is_finite() || fraction <= 0.0 {
            return Err(
                "TRIM would reduce the target spline to nothing: the kept span is below the \
                 parameter tolerance"
                    .into(),
            );
        }
        let (t_min, t_max) = self.curve.domain();
        let span = t_max - t_min;
        let t0 = t_min + span * u0;
        let mut t1 = t_min + span * (u0 + fraction);
        if t1 > t_max {
            // Only rounding can put a window end past the domain: the cuts and the
            // span ends are all inside it. Refuse anything larger instead of
            // shrinking a genuinely out-of-range request.
            if t1 - t_max > span * SPAN_END_ROUNDING_FRACTION {
                return Err(format!(
                    "TRIM could not extract the SPLINE sub-curve: the kept span ends at {t1}, \
                     outside the curve's knot domain [{t_min}, {t_max}]"
                ));
            }
            t1 = t_max;
        }
        let sub = cad_geometry::sub_curve(&self.curve, t0, t1).map_err(|error| {
            format!("TRIM could not extract the exact SPLINE sub-curve: {error}")
        })?;
        Ok(SemanticGeometry::Spline {
            degree: sub.degree as u32,
            knots: sub.knots,
            control_points: sub.control_points,
            weights: sub.weights,
        })
    }

    /// Report that the kept piece is an exact sub-curve whose **cut parameters**
    /// came from the sampled intersection.
    ///
    /// One note per replaced target (not per cut pair): the split is a property of
    /// the target, and the contributing pairs already have their own
    /// `note_approximation` entries.
    pub(super) fn note_exact_sub_curve_on_sampled_cuts(&self, notes: &mut Vec<String>) {
        notes.push(format!(
            "TRIM: the kept piece of the SPLINE target is an exact B-spline sub-curve (knot \
             insertion on the homogeneous control points, both ends clamped to multiplicity \
             degree+1, degree {} kept), but its end knots come from inverting sampled \
             intersection points (chord height extent x {SAMPLED_CUT_CHORD_FRACTION:e} of the \
             control-polygon box, at most 256 segments), so an end that came from a cut lies at \
             most that chord height from the true crossing; the curve is never refitted, \
             re-parameterised or replaced by a polyline",
            self.curve.degree()
        ));
    }

    /// Normalised `[0, 1]` parameter → the curve's raw knot parameter.
    fn parameter(&self, u: f64) -> f64 {
        let (t_min, t_max) = self.curve.domain();
        t_min + (t_max - t_min) * u
    }

    /// The curve's raw knot parameter → the normalised `[0, 1]` parameter.
    fn normalised(&self, native: f64) -> f64 {
        let (t_min, t_max) = self.curve.domain();
        (native - t_min) / (t_max - t_min)
    }
}

/// Why EXTEND refuses a SPLINE target.
///
/// Shared by the command (which refuses before any work) and the plan (which
/// cannot be reached with a spline, but must still answer by name), so there is
/// exactly one wording.
pub(super) fn no_continuation_semantics() -> String {
    "EXTEND target AcDbSpline has no continuation semantics: a NURBS curve is defined by its \
     knot vector, so moving an end would change the knot vector and therefore the \
     parameterisation of the whole curve, and no extrapolation policy exists; the target is \
     refused rather than extended by approximation"
        .into()
}

/// The target's polyline length, sampled adaptively at
/// [`LENGTH_SAMPLE_CHORD_FRACTION`] of its own extent.
///
/// A polyline length under-estimates arc length by at most the chord error, which
/// is fine for its two uses (converting a world tolerance into parameter units and
/// rejecting a zero-length target). It is never reported as a measurement.
fn curve_length(curve: &NurbsCurve, extent: f64) -> f64 {
    let chord = (extent * LENGTH_SAMPLE_CHORD_FRACTION).max(f64::MIN_POSITIVE);
    let params = TessellationParams {
        tolerance: chord,
        max_segments: LENGTH_SAMPLE_MAX_SEGMENTS,
        min_segments: LENGTH_SAMPLE_MIN_SEGMENTS,
    };
    let samples = curve.discretize(chord, &params);
    samples
        .windows(2)
        .map(|window| distance(window[0], window[1]))
        .sum()
}
