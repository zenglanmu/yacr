//! The source-object SPLINE chain: spline storage plus the exact C0
//! concatenation used when a SPLINE is the JOIN source object.
//!
//! Split out of `join_plan.rs` (the code-size limit); it is a private helper
//! module of that file, not a second JOIN mode. See `join_nurbs` for the mixed
//! LINE/ARC/elliptical-arc weld a SPLINE source also accepts.

use super::super::*;
use super::{invalid, join_tolerance, unsupported, Joined};
use cad_geometry::clamped_uniform_knots;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SplineCurve {
    degree: u32,
    knots: Vec<f64>,
    control_points: Vec<Point3>,
    weights: Vec<f64>,
}

impl SplineCurve {
    /// Expand the sanctioned empty-knot/empty-weight shorthand to the stored
    /// clamped-uniform form.
    fn from_geometry(
        degree: u32,
        knots: &[f64],
        control_points: &[Point3],
        weights: &[f64],
    ) -> CadResult<Self> {
        let n = control_points.len();
        let degree = degree.max(1).min(n.saturating_sub(1) as u32);
        if n < degree as usize + 1 {
            return Err(invalid(
                "join.degenerate",
                "a SPLINE has too few control points",
            ));
        }
        let knots = if knots.is_empty() {
            clamped_uniform_knots(n, degree as usize)
        } else {
            knots.to_vec()
        };
        let weights = if weights.is_empty() {
            vec![1.0; n]
        } else {
            weights.to_vec()
        };
        Ok(SplineCurve {
            degree,
            knots,
            control_points: control_points.to_vec(),
            weights,
        })
    }

    pub(super) fn geometry(self) -> SemanticGeometry {
        SemanticGeometry::Spline {
            degree: self.degree,
            knots: self.knots,
            control_points: self.control_points,
            weights: self.weights,
        }
    }

    fn head(&self) -> Point3 {
        self.control_points[0]
    }

    fn tail(&self) -> Point3 {
        *self.control_points.last().expect("validated non-empty")
    }

    /// Reverse the curve: reversed control points/weights and mirrored knots.
    fn reversed(self) -> Self {
        let mut control_points = self.control_points;
        let mut weights = self.weights;
        control_points.reverse();
        weights.reverse();
        let knots = self.knots;
        let sum = knots[0] + knots[knots.len() - 1];
        let knots = knots.iter().rev().map(|knot| sum - knot).collect();
        SplineCurve {
            degree: self.degree,
            knots,
            control_points,
            weights,
        }
    }

    /// Exact C0 concatenation: `U = U1 ++ shift(U2)[p+1..]` and
    /// `P = P1 ++ P2`, so the join knot has multiplicity `p + 1` and the
    /// control points stay the stored ones.
    fn concat(mut self, other: SplineCurve) -> CadResult<Self> {
        if self.degree != other.degree {
            return Err(unsupported(
                "join.spline_degree",
                format!(
                    "SPLINE degrees {} and {} cannot be one curve",
                    self.degree, other.degree
                ),
            ));
        }
        let p = self.degree as usize;
        let total = self.control_points.len() + other.control_points.len();
        if self.knots.len() != self.control_points.len() + p + 1
            || other.knots.len() != other.control_points.len() + p + 1
        {
            return Err(unsupported(
                "join.spline_clamped",
                "a source SPLINE does not have a clamped knot vector to concatenate",
            ));
        }
        let first = self.knots[0];
        let first_end = self.knots[self.control_points.len()];
        let other_start = other.knots[0];
        let other_end = other.knots[other.control_points.len()];
        if first_end <= first || other_end <= other_start {
            return Err(invalid(
                "join.degenerate",
                "a SPLINE has an empty knot domain",
            ));
        }
        if self.knots[self.control_points.len()..]
            .iter()
            .any(|knot| (knot - first_end).abs() > 1e-12)
            || other.knots[..=p]
                .iter()
                .any(|knot| (knot - other_start).abs() > 1e-12)
        {
            return Err(unsupported(
                "join.spline_clamped",
                "a source SPLINE is not clamped at the junction end",
            ));
        }
        // Normalise both pieces onto [0, 1] and [1, 2].
        let span = first_end - first;
        let mut knots: Vec<f64> = self
            .knots
            .iter()
            .map(|knot| (knot - first) / span)
            .collect();
        let other_span = other_end - other_start;
        let shifted: Vec<f64> = other.knots[p + 1..]
            .iter()
            .map(|knot| 1.0 + (knot - other_start) / other_span)
            .collect();
        knots.extend(shifted);
        debug_assert_eq!(knots.len(), total + p + 1);
        let mut control_points = std::mem::take(&mut self.control_points);
        control_points.extend(other.control_points);
        let mut weights = std::mem::take(&mut self.weights);
        weights.extend(other.weights);
        Ok(SplineCurve {
            degree: self.degree,
            knots,
            control_points,
            weights,
        })
    }
}

/// Plan a SPLINE merge: contiguous end-to-end, any plane, exact C0
/// concatenation for equal degrees.
pub(super) fn plan_splines(sources: &[DbEntity], close: bool) -> CadResult<Joined> {
    if close {
        return Err(unsupported(
            "join.close_spline",
            "JOIN CLOSE on SPLINEs is not implemented (the SPLINE model has no closed flag)",
        ));
    }
    let mut pieces: Vec<SplineCurve> = Vec::with_capacity(sources.len());
    for source in sources {
        let SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } = &source.geometry
        else {
            return Err(invalid(
                "join.mixed_kinds",
                "a SPLINE source takes SPLINEs only",
            ));
        };
        pieces.push(SplineCurve::from_geometry(
            *degree,
            knots,
            control_points,
            weights,
        )?);
    }
    let tol = join_tolerance();
    let mut current = pieces.remove(0);
    while !pieces.is_empty() {
        let head = current.head();
        let tail = current.tail();
        let mut matched = false;
        for index in 0..pieces.len() {
            let piece = &pieces[index];
            if distance(piece.head(), tail) <= tol {
                let piece = pieces.remove(index);
                current = current.concat(piece)?;
                matched = true;
                break;
            }
            if distance(piece.tail(), tail) <= tol {
                let piece = pieces.remove(index).reversed();
                current = current.concat(piece)?;
                matched = true;
                break;
            }
            if distance(piece.tail(), head) <= tol {
                let piece = pieces.remove(index);
                current = piece.concat(current)?;
                matched = true;
                break;
            }
            if distance(piece.head(), head) <= tol {
                let piece = pieces.remove(index).reversed();
                current = piece.concat(current)?;
                matched = true;
                break;
            }
        }
        if !matched {
            return Err(invalid(
                "join.not_contiguous",
                "a selected SPLINE does not touch the current spline; JOIN does not bridge gaps",
            ));
        }
    }
    Ok(Joined::Spline(current))
}
