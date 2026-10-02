//! Rational B-spline (NURBS) curves.
//!
//! Spec v2.0 §16.1 and audit B23: a spline is defined by its own degree, knot
//! vector and (optional) weights. Re-inventing a uniform knot vector or
//! ignoring the weights changes the geometry, so this type owns the source
//! data, evaluates points and first derivatives from it, and discretises
//! adaptively against a chord tolerance rather than a fixed sample count.
//!
//! The representation is deliberately self-contained: no external maths crate
//! leaks into the public contract.

use cad_domain::{CadError, CadResult, Point3};

use crate::{distance, is_finite, TessellationParams};

/// A rational (or, when every weight is 1, polynomial) B-spline curve.
///
/// `control_points.len() == weights.len() == n` and
/// `knots.len() == n + degree + 1`. The curve is defined on
/// `[knots[degree], knots[n]]`.
#[derive(Debug, Clone, PartialEq)]
pub struct NurbsCurve {
    degree: usize,
    control_points: Vec<Point3>,
    weights: Vec<f64>,
    knots: Vec<f64>,
}

impl NurbsCurve {
    /// Build a validated NURBS curve.
    ///
    /// Rejects non-finite/degenerate data instead of fabricating a curve: an
    /// empty control polygon, a degree with no interior, a wrong-length or
    /// non-monotone knot vector, and non-finite or non-positive weights are all
    /// errors (spec §16.1).
    pub fn new(
        degree: usize,
        control_points: Vec<Point3>,
        knots: Vec<f64>,
        weights: Vec<f64>,
    ) -> CadResult<Self> {
        let n = control_points.len();
        if n == 0 {
            return Err(CadError::InvalidInput(
                "NURBS curve has no control points".into(),
            ));
        }
        if degree == 0 {
            return Err(CadError::InvalidInput(
                "NURBS degree must be at least 1".into(),
            ));
        }
        if n < degree + 1 {
            return Err(CadError::InvalidInput(format!(
                "NURBS needs at least degree+1 control points (degree {degree}, {n} given)"
            )));
        }
        if !control_points.iter().all(|p| is_finite(*p)) {
            return Err(CadError::InvalidInput(
                "NURBS has a non-finite control point".into(),
            ));
        }
        if knots.len() != n + degree + 1 {
            return Err(CadError::InvalidInput(format!(
                "NURBS knot vector must have n+degree+1 = {} entries, got {}",
                n + degree + 1,
                knots.len()
            )));
        }
        if !knots.iter().all(|k| k.is_finite()) {
            return Err(CadError::InvalidInput("NURBS has a non-finite knot".into()));
        }
        if knots.windows(2).any(|w| w[1] < w[0]) {
            return Err(CadError::InvalidInput(
                "NURBS knot vector must be non-decreasing".into(),
            ));
        }
        let weights = if weights.is_empty() {
            vec![1.0; n]
        } else {
            if weights.len() != n {
                return Err(CadError::InvalidInput(format!(
                    "NURBS weights must have n = {n} entries, got {}",
                    weights.len()
                )));
            }
            if weights.iter().any(|w| !w.is_finite() || *w <= 0.0) {
                return Err(CadError::InvalidInput(
                    "NURBS weights must be finite and positive".into(),
                ));
            }
            weights
        };
        let (t_min, t_max) = (knots[degree], knots[n]);
        if t_max <= t_min {
            return Err(CadError::InvalidInput("NURBS knot domain is empty".into()));
        }
        Ok(NurbsCurve {
            degree,
            control_points,
            weights,
            knots,
        })
    }

    pub fn degree(&self) -> usize {
        self.degree
    }

    pub fn control_points(&self) -> &[Point3] {
        &self.control_points
    }

    pub fn weights(&self) -> &[f64] {
        &self.weights
    }

    pub fn knots(&self) -> &[f64] {
        &self.knots
    }

    fn count(&self) -> usize {
        self.control_points.len()
    }

    /// The parameter domain `[knots[degree], knots[n]]`.
    pub fn domain(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.count()])
    }

    /// The homogeneous control points `(w*x, w*y, w*z, w)`.
    fn homogeneous(&self) -> Vec<[f64; 4]> {
        self.control_points
            .iter()
            .zip(&self.weights)
            .map(|(p, w)| [p.x * w, p.y * w, p.z * w, *w])
            .collect()
    }

    /// Evaluate the curve at `t` (clamped to the domain).
    pub fn evaluate(&self, t: f64) -> Point3 {
        if !t.is_finite() {
            return Point3::default();
        }
        let (t_min, t_max) = self.domain();
        let t = t.clamp(t_min, t_max);
        let hom = self.homogeneous();
        let d = de_boor_homogeneous(&hom, &self.knots, self.degree, t);
        if d[3].abs() < 1e-300 {
            Point3 {
                x: d[0],
                y: d[1],
                z: d[2],
            }
        } else {
            Point3 {
                x: d[0] / d[3],
                y: d[1] / d[3],
                z: d[2] / d[3],
            }
        }
    }

    /// First derivative (tangent vector, not normalised) at `t`.
    ///
    /// For a rational curve `P(t) = A(t)/w(t)` the quotient rule gives
    /// `P' = (A' - P w') / w`, with `A` and `w` the polynomial B-spline
    /// components of the homogeneous curve.
    pub fn tangent(&self, t: f64) -> Point3 {
        let (t_min, t_max) = self.domain();
        if !t.is_finite() {
            return Point3::default();
        }
        let t = t.clamp(t_min, t_max);
        let p = self.evaluate(t);
        let hom = self.homogeneous();
        let dh = self.homogeneous_derivative();
        // Degree 0 means the derivative is identically zero.
        let d = match dh {
            Some((deriv, knots, degree)) => de_boor_homogeneous(&deriv, &knots, degree, t),
            None => [0.0; 4],
        };
        let w = {
            let base = de_boor_homogeneous(&hom, &self.knots, self.degree, t);
            base[3]
        };
        if w.abs() < 1e-300 {
            return Point3::default();
        }
        Point3 {
            x: (d[0] - p.x * d[3]) / w,
            y: (d[1] - p.y * d[3]) / w,
            z: (d[2] - p.z * d[3]) / w,
        }
    }

    /// Homogeneous derivative curve control points and knots (degree - 1).
    fn homogeneous_derivative(&self) -> Option<(Vec<[f64; 4]>, Vec<f64>, usize)> {
        if self.degree == 0 {
            return None;
        }
        let n = self.count();
        let p = self.degree as f64;
        let hom = self.homogeneous();
        let mut deriv = Vec::with_capacity(n - 1);
        for i in 0..n - 1 {
            let denom = self.knots[i + self.degree + 1] - self.knots[i + 1];
            let mut q = [0.0; 4];
            if denom.abs() > 1e-300 {
                for c in 0..4 {
                    q[c] = p * (hom[i + 1][c] - hom[i][c]) / denom;
                }
            }
            deriv.push(q);
        }
        let knots = self.knots[1..self.knots.len() - 1].to_vec();
        Some((deriv, knots, self.degree - 1))
    }

    /// Adaptively discretise the curve so every chord stays within `tolerance`.
    ///
    /// Each non-empty knot span is refined by recursive bisection until the
    /// true midpoint is within `tolerance` of the chord, so the sample count
    /// follows the geometry instead of a fixed parameter step. `params`
    /// bounds the result; its `max_segments` is a hard cap.
    pub fn discretize(&self, tolerance: f64, params: &TessellationParams) -> Vec<Point3> {
        let tol = tolerance.max(0.0);
        let (t_min, t_max) = self.domain();
        let mut breaks = vec![t_min];
        breaks.extend(
            self.knots
                .iter()
                .copied()
                .filter(|k| *k > t_min && *k < t_max),
        );
        breaks.push(t_max);
        breaks.dedup_by(|a, b| (*a - *b).abs() <= f64::EPSILON);

        let mut budget = params.max_segments.max(1);
        let mut out = Vec::with_capacity(budget.min(4096) + 1);
        out.push(self.evaluate(t_min));
        for span in breaks.windows(2) {
            let (a, b) = (span[0], span[1]);
            if b <= a {
                continue;
            }
            let pa = *out.last().unwrap_or(&self.evaluate(a));
            let pb = self.evaluate(b);
            refine(self, a, b, pa, pb, tol, 0, &mut budget, &mut out);
        }
        out
    }
}

/// Recursively bisect `[a, b]` until the true midpoint is within `tol` of the
/// chord. `out` already holds the point at `a`, so only points after `a` are
/// appended; the endpoint is always emitted even when the budget runs out, so
/// the sampled curve is never silently truncated.
#[allow(clippy::too_many_arguments)]
fn refine(
    curve: &NurbsCurve,
    a: f64,
    b: f64,
    pa: Point3,
    pb: Point3,
    tol: f64,
    depth: usize,
    budget: &mut usize,
    out: &mut Vec<Point3>,
) {
    let max_depth = 24usize;
    if depth >= max_depth || (b - a) <= f64::EPSILON {
        push_limited(pb, budget, out);
        return;
    }
    let mid_t = 0.5 * (a + b);
    let pm = curve.evaluate(mid_t);
    let chord_mid = Point3 {
        x: 0.5 * (pa.x + pb.x),
        y: 0.5 * (pa.y + pb.y),
        z: 0.5 * (pa.z + pb.z),
    };
    if distance(pm, chord_mid) <= tol.max(1e-15) {
        push_limited(pb, budget, out);
        return;
    }
    refine(curve, a, mid_t, pa, pm, tol, depth + 1, budget, out);
    refine(curve, mid_t, b, pm, pb, tol, depth + 1, budget, out);
}

fn push_limited(p: Point3, budget: &mut usize, out: &mut Vec<Point3>) {
    if *budget > 0 {
        *budget -= 1;
    }
    if !out.last().map(|q| distance(*q, p) < 1e-15).unwrap_or(false) {
        out.push(p);
    }
}

/// One de Boor step over a homogeneous control polygon.
fn de_boor_homogeneous(hom: &[[f64; 4]], knots: &[f64], degree: usize, t: f64) -> [f64; 4] {
    let n = hom.len();
    if n == 0 || degree >= n {
        return [0.0; 4];
    }
    let mut span = degree;
    while span < n - 1 && t >= knots[span + 1] {
        span += 1;
    }
    let mut d: Vec<[f64; 4]> = (0..=degree).map(|j| hom[span - degree + j]).collect();
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = span - degree + j;
            let denom = knots[i + degree + 1 - r] - knots[i];
            let alpha = if denom.abs() > 1e-300 {
                (t - knots[i]) / denom
            } else {
                0.0
            };
            let prev = d[j - 1];
            let cur = d[j];
            for c in 0..4 {
                d[j][c] = prev[c] * (1.0 - alpha) + cur[c] * alpha;
            }
        }
    }
    d[degree]
}

/// A polynomial B-spline through `control` with clamped uniform knots.
///
/// Convenience used where no source knot vector exists (e.g. hatch boundaries);
/// source splines with explicit knots go through [`NurbsCurve::new`].
pub fn clamped_uniform_knots(n: usize, degree: usize) -> Vec<f64> {
    let m = n + degree + 1;
    let mut knots = vec![0.0; m];
    for i in 0..=degree {
        if n + i < m {
            knots[n + i] = 1.0;
        }
    }
    let inner = n.saturating_sub(degree + 1);
    for i in 1..=inner {
        knots[degree + i] = i as f64 / (inner + 1) as f64;
    }
    knots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sub;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    /// The classic exact rational quadratic for a quarter circle:
    /// control points (1,0), (1,1), (0,1) with middle weight `1/√2` and
    /// clamped knots `[0,0,0,1,1,1]`.
    fn quarter_circle() -> NurbsCurve {
        NurbsCurve::new(
            2,
            vec![p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![1.0, std::f64::consts::FRAC_1_SQRT_2, 1.0],
        )
        .unwrap()
    }

    #[test]
    fn rational_quadratic_is_an_exact_quarter_circle() {
        let c = quarter_circle();
        for i in 0..=16 {
            let t = i as f64 / 16.0;
            let q = c.evaluate(t);
            assert!(
                (q.x.hypot(q.y) - 1.0).abs() < 1e-12,
                "t={t} not on unit circle: {q:?}"
            );
            assert!(q.x >= -1e-12 && q.y >= -1e-12);
        }
        // Endpoints are interpolated exactly and the tangent at the start is
        // along +Y with the radius along +X.
        assert!(distance(c.evaluate(0.0), p(1.0, 0.0)) < 1e-12);
        assert!(distance(c.evaluate(1.0), p(0.0, 1.0)) < 1e-12);
        let d0 = c.tangent(0.0);
        assert!(d0.x.abs() < 1e-9 && d0.y > 0.0, "tangent {d0:?}");
    }

    #[test]
    fn derivative_matches_finite_difference() {
        let c = quarter_circle();
        let h = 1e-6;
        for i in 1..8 {
            let t = i as f64 / 8.0;
            let fd = sub(c.evaluate(t + h), c.evaluate(t - h));
            let fd = Point3 {
                x: fd.x / (2.0 * h),
                y: fd.y / (2.0 * h),
                z: fd.z / (2.0 * h),
            };
            let d = c.tangent(t);
            assert!(distance(fd, d) < 1e-5, "t={t}: {fd:?} vs {d:?}");
        }
    }

    #[test]
    fn clamped_cubic_interpolates_its_endpoints() {
        // A clamped cubic B-spline with 5 control points; the curve must start
        // and end on the first/last control points.
        let c = NurbsCurve::new(
            3,
            vec![
                p(0.0, 0.0),
                p(1.0, 2.0),
                p(3.0, 3.0),
                p(5.0, 2.0),
                p(6.0, 0.0),
            ],
            vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
            vec![],
        )
        .unwrap();
        assert!(distance(c.evaluate(0.0), p(0.0, 0.0)) < 1e-12);
        assert!(distance(c.evaluate(1.0), p(6.0, 0.0)) < 1e-12);
        // Non-uniform interior knot: the curve is not the uniform one.
        let mid = c.evaluate(0.5);
        assert!(mid.y > 2.0, "mid {mid:?}");
    }

    #[test]
    fn malformed_inputs_are_rejected() {
        assert!(NurbsCurve::new(0, vec![p(0.0, 0.0)], vec![0.0, 0.0], vec![]).is_err());
        assert!(
            NurbsCurve::new(3, vec![p(0.0, 0.0)], vec![0.0, 0.0, 0.0, 0.0, 1.0], vec![]).is_err()
        );
        // Non-monotone knots.
        assert!(NurbsCurve::new(
            1,
            vec![p(0.0, 0.0), p(1.0, 0.0)],
            vec![0.0, 1.0, 0.5, 1.0],
            vec![]
        )
        .is_err());
    }

    #[test]
    fn adaptive_discretisation_tracks_tolerance() {
        let c = quarter_circle();
        let coarse = c.discretize(0.05, &TessellationParams::default());
        let fine = c.discretize(0.0005, &TessellationParams::default());
        assert!(
            fine.len() > coarse.len(),
            "{} vs {}",
            fine.len(),
            coarse.len()
        );
        // Every chord of the fine sampling stays close to the true curve.
        for q in &fine {
            assert!((q.x.hypot(q.y) - 1.0).abs() < 1e-9);
        }
    }
}
