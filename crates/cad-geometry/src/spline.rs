//! Fit-point spline construction: AutoCAD-style fit points -> stored NURBS.
//!
//! This module is the numerical core behind the `SPLINE` command
//! (`SPLMETHOD`=0 fit, `SPLDEGREE`=3, `SPLKNOTS`=0 chord). It is a *pure*
//! function: given an ordered list of fit points it returns a clamped cubic
//! B-spline `(knots, control_points)` that interpolates those points. It does
//! not touch [`cad_domain::SemanticGeometry`], the database or any command
//! state; wiring the returned pair into
//! `SemanticGeometry::Spline { degree: 3, knots, control_points, weights: [1.0; n] }`
//! is the caller's job.
//!
//! # Method (what "chord, interpolating" means here)
//!
//! * The parameter of fit point `Q_i` is its **cumulative chord length** as a
//!   fraction of the total polyline length, so `param_0 = 0`, `param_{n-1} = 1`
//!   and `param` is strictly increasing (`SPLKNOTS`=0). See
//!   [`chord_parameters`].
//! * The knot vector is **clamped** (end multiplicity `degree + 1 = 4`) and the
//!   remaining interior knots are the standard averaging of the data parameters
//!   (Piegl & Tiller, *The NURBS Book*, eq. 9.8). Its length is exactly
//!   `control_points.len() + degree + 1`.
//! * The control points solve the square collocation system
//!   `sum_k N_{k,3}(param_i) P_k = Q_i` for `i = 0..n-1`. The basis is
//!   evaluated with the Cox–de Boor recursion and the banded (`O(n)`) system
//!   is solved without a dense `O(n^3)` factorisation.
//!
//! The result is mind-independent of anything but the fit points: the same
//! input always yields bit-identical output.
//!
//! # End conditions (implementation-level choice, *not* AutoCAD parity)
//!
//! With a clamped knot vector and one control point per fit point the
//! interpolation conditions already form a square system, so there is **no
//! free degree of freedom for a separate end-derivative or "natural" (zero
//! curvature) constraint**. The chosen scheme is the textbook clamped global
//! interpolation: the curve starts exactly at `Q_0` and ends exactly at
//! `Q_{n-1}` and its end tangents/curvature are whatever that system produces.
//! We deliberately do **not** claim this matches AutoCAD: ADR 0015 decision F
//! records that AutoCAD's end conditions are unverified.
//!
//! # Degenerate input
//!
//! Non-finite coordinates are refused. Chord parameterisation is undefined for
//! a zero-length chord, so any chord shorter than
//! [`MIN_RELATIVE_CHORD`] of the total polyline length (which includes an
//! exact duplicate point, relative length `0`) makes the collocation system
//! too ill-conditioned to trust and is **refused explicitly** rather than
//! silently de-duplicated or returned as garbage. A collinear (but
//! non-degenerate) run is valid and succeeds.

use cad_domain::{CadError, CadResult, Point3};

#[cfg(test)]
use crate::nurbs::NurbsCurve;
use crate::{distance, is_finite};

/// Degree of the fit-interpolated spline (AutoCAD `SPLDEGREE` default).
pub const FIT_SPLINE_DEGREE: u32 = 3;

/// Minimum number of fit points [`fit_points_to_cubic_spline`] accepts.
///
/// A degree-3 spline needs `degree + 1 = 4` control points, and
/// `cad-db`'s `validate` enforces `control_points.len() >= degree + 1`; because
/// this construction uses one control point per fit point the same bound
/// applies to the fit-point count. Do not relax the database bound to admit
/// smaller inputs — ADR 0015 decision B1/F chose the explicit refusal.
pub const MIN_FIT_POINTS_FOR_CUBIC_SPLINE: usize = FIT_SPLINE_DEGREE as usize + 1;

/// Smallest chord (as a fraction of the total polyline length) that keeps the
/// chord-parameterised collocation system well conditioned. A shorter chord —
/// including an exact duplicate fit point, whose relative length is `0` — is
/// refused explicitly instead of producing an oscillating interpolant or a
/// silent `NaN`.
const MIN_RELATIVE_CHORD: f64 = 1e-9;

/// Pivot magnitude below which the banded elimination is considered
/// numerically singular. Reaching this is a conditioning refusal, never a
/// silent garbage result.
const MIN_PIVOT: f64 = 1e-12;

/// Number of control points [`fit_points_to_cubic_spline`] produces for
/// `fit_point_count` fit points, or `None` when the input is too small.
///
/// Exposed so the command lane can pre-check the `>= 4` rule without
/// duplicating [`MIN_FIT_POINTS_FOR_CUBIC_SPLINE`]. For a fit-interpolating
/// clamped spline the control-point count equals the fit-point count.
pub fn cubic_spline_control_point_count(fit_point_count: usize) -> Option<usize> {
    (fit_point_count >= MIN_FIT_POINTS_FOR_CUBIC_SPLINE).then_some(fit_point_count)
}

/// The normalised cumulative-chord parameters of `fit_points` (`SPLKNOTS`=0).
///
/// Returns `n` values in `[0, 1]`, strictly increasing, with the first exactly
/// `0` and the last exactly `1`. Refuses fewer than two points, non-finite
/// coordinates, an all-coincident run, and any chord shorter than
/// [`MIN_RELATIVE_CHORD`] of the total length (see the module docs).
pub fn chord_parameters(fit_points: &[Point3]) -> CadResult<Vec<f64>> {
    let n = fit_points.len();
    if n < 2 {
        return Err(CadError::InvalidInput(
            "chord parameterisation needs at least 2 fit points".into(),
        ));
    }
    if !fit_points.iter().all(|p| is_finite(*p)) {
        return Err(CadError::InvalidInput(
            "spline fit point is non-finite".into(),
        ));
    }
    let mut chords = Vec::with_capacity(n - 1);
    let mut total = 0.0;
    for w in fit_points.windows(2) {
        let d = distance(w[0], w[1]);
        chords.push(d);
        total += d;
    }
    if total <= 0.0 {
        return Err(CadError::InvalidInput(
            "spline fit points are all coincident (total chord length is zero)".into(),
        ));
    }
    if let Some((i, _)) = chords
        .iter()
        .enumerate()
        .find(|(_, d)| **d / total < MIN_RELATIVE_CHORD)
    {
        return Err(CadError::InvalidInput(format!(
            "spline has a degenerate chord at fit-point segment {i}: chord parameterisation \
             is undefined for coincident points and below {MIN_RELATIVE_CHORD:e} of the total \
             length the interpolation is not numerically trustworthy"
        )));
    }
    let mut params = Vec::with_capacity(n);
    let mut acc = 0.0;
    params.push(0.0);
    for d in &chords {
        acc += d;
        params.push(acc / total);
    }
    // The last chord closes the run exactly; pin the endpoint against rounding
    // so the clamped knot vector interpolates the final fit point exactly.
    *params.last_mut().expect("n >= 2") = 1.0;
    Ok(params)
}

/// Turn ordered fit points into `(knots, control_points)` for a degree-3,
/// clamped, chord-parameterised, interpolating B-spline.
///
/// Feed the result (with `weights = vec![1.0; control_points.len()]`) into
/// `SemanticGeometry::Spline { degree: 3, knots, control_points, weights }`.
///
/// # Errors
///
/// * fewer than [`MIN_FIT_POINTS_FOR_CUBIC_SPLINE`] fit points;
/// * non-finite coordinates;
/// * all-coincident points, or any chord shorter than [`MIN_RELATIVE_CHORD`]
///   of the total length (duplicate points included);
/// * a numerically singular collocation system (conditioning refusal).
pub fn fit_points_to_cubic_spline(fit_points: &[Point3]) -> CadResult<(Vec<f64>, Vec<Point3>)> {
    let n = fit_points.len();
    if cubic_spline_control_point_count(n).is_none() {
        return Err(CadError::InvalidInput(format!(
            "cubic fit spline needs at least {MIN_FIT_POINTS_FOR_CUBIC_SPLINE} fit points, got {n}"
        )));
    }
    let params = chord_parameters(fit_points)?;
    let degree = FIT_SPLINE_DEGREE as usize;
    let knots = build_clamped_averaged_knots(&params, degree);
    let control_points = solve_collocation(fit_points, &params, &knots, degree)?;
    Ok((knots, control_points))
}

/// Clamped knot vector with `n + degree + 1` entries: `degree + 1` leading
/// zeros, `degree + 1` trailing ones, and interior knots equal to the average
/// of `degree` consecutive data parameters (Piegl & Tiller eq. 9.8).
fn build_clamped_averaged_knots(params: &[f64], degree: usize) -> Vec<f64> {
    let n = params.len();
    let p = degree;
    let m = n + p + 1;
    let mut knots = vec![0.0; m];
    for k in knots.iter_mut().skip(m - p - 1) {
        *k = 1.0;
    }
    // j runs over the interior knots; `n - p - 1` is empty for the minimal n = 4.
    for j in 1..=(n - p - 1) {
        let s: f64 = params[j..j + p].iter().sum();
        knots[j + p] = s / p as f64;
    }
    knots
}

/// The knot span index containing `t` for a clamped vector (Piegl & Tiller
/// A2.1). `n_ctrl` is the number of control points (`knots.len() - degree - 1`).
fn find_span(knots: &[f64], degree: usize, n_ctrl: usize, t: f64) -> usize {
    if t >= knots[n_ctrl] {
        return n_ctrl - 1;
    }
    if t <= knots[degree] {
        return degree;
    }
    let mut low = degree;
    let mut high = n_ctrl;
    let mut mid = (low + high) / 2;
    while t < knots[mid] || t >= knots[mid + 1] {
        if t < knots[mid] {
            high = mid;
        } else {
            low = mid;
        }
        mid = (low + high) / 2;
    }
    mid
}

/// The `degree + 1` non-zero Cox–de Boor basis functions at `t` (Piegl &
/// Tiller A2.2), written into `basis`. `left`/`right` are caller-owned scratch
/// buffers of length `degree + 1`.
fn basis_functions(
    span: usize,
    t: f64,
    degree: usize,
    knots: &[f64],
    basis: &mut [f64],
    left: &mut [f64],
    right: &mut [f64],
) {
    basis[0] = 1.0;
    for j in 1..=degree {
        left[j] = t - knots[span + 1 - j];
        right[j] = knots[span + j] - t;
        let mut saved = 0.0;
        for r in 0..j {
            let denom = right[r + 1] + left[j - r];
            let temp = if denom.abs() > 0.0 {
                basis[r] / denom
            } else {
                0.0
            };
            basis[r] = saved + right[r + 1] * temp;
            saved = left[j - r] * temp;
        }
        basis[j] = saved;
    }
}

/// Solve the banded collocation system `N P = Q` for the control points.
///
/// The matrix is `n x n` with half-bandwidth `degree` (each row has at most
/// `degree + 1` non-zeros), stored banded so the elimination is `O(n)` in time
/// and memory. Elimination runs without pivoting: the B-spline collocation
/// matrix is totally positive, so the diagonal is a valid pivot chain; if a
/// pivot ever falls below [`MIN_PIVOT`] the input is reported as ill-conditioned
/// instead of returning garbage.
fn solve_collocation(
    fit_points: &[Point3],
    params: &[f64],
    knots: &[f64],
    degree: usize,
) -> CadResult<Vec<Point3>> {
    let n = fit_points.len();
    let p = degree;
    let w = 2 * p + 1;

    let mut band = vec![0.0f64; n * w];
    let mut rhs = vec![[0.0f64; 3]; n];

    let mut basis = vec![0.0f64; p + 1];
    let mut left = vec![0.0f64; p + 1];
    let mut right = vec![0.0f64; p + 1];

    for i in 0..n {
        let t = params[i];
        let span = find_span(knots, p, n, t);
        basis_functions(span, t, p, knots, &mut basis, &mut left, &mut right);
        for (j, b) in basis.iter().enumerate() {
            let col = span - p + j;
            // `col - i` stays within `[-p, p]` for clamped averaged knots.
            let k = (col as isize - i as isize) + p as isize;
            debug_assert!((0..w as isize).contains(&k));
            band[i * w + k as usize] += *b;
        }
        rhs[i] = [fit_points[i].x, fit_points[i].y, fit_points[i].z];
    }

    for k in 0..n {
        let piv = band[k * w + p];
        if piv.abs() < MIN_PIVOT {
            return Err(CadError::InvalidInput(
                "cubic fit spline collocation system is numerically singular".into(),
            ));
        }
        let i_max = (k + p).min(n - 1);
        for i in (k + 1)..=i_max {
            let off = i - k;
            let a_ik = band[i * w + (p - off)];
            if a_ik == 0.0 {
                continue;
            }
            let factor = a_ik / piv;
            band[i * w + (p - off)] = 0.0;
            for dj in 1..=p {
                let j = k + dj;
                if j >= n {
                    break;
                }
                let a_kj = band[k * w + (p + dj)];
                if a_kj != 0.0 {
                    band[i * w + (p - off + dj)] -= factor * a_kj;
                }
            }
            let rk = rhs[k];
            for (v, r) in rhs[i].iter_mut().zip(rk) {
                *v -= factor * r;
            }
        }
    }

    let mut x = vec![[0.0f64; 3]; n];
    for k in (0..n).rev() {
        let mut s = rhs[k];
        let j_max = (k + p).min(n - 1);
        for j in (k + 1)..=j_max {
            let a_kj = band[k * w + (p + (j - k))];
            for (sc, xc) in s.iter_mut().zip(x[j]) {
                *sc -= a_kj * xc;
            }
        }
        let piv = band[k * w + p];
        x[k] = [s[0] / piv, s[1] / piv, s[2] / piv];
    }

    Ok(x.iter()
        .map(|v| Point3 {
            x: v[0],
            y: v[1],
            z: v[2],
        })
        .collect())
}

/// Evaluate the stored spline as the geometry engine would: build a
/// polynomial `NurbsCurve` with unit weights. Test-only helper.
#[cfg(test)]
fn evaluate_curve(knots: &[f64], control_points: &[Point3], t: f64) -> Point3 {
    let weights = vec![1.0; control_points.len()];
    let curve = NurbsCurve::new(
        FIT_SPLINE_DEGREE as usize,
        control_points.to_vec(),
        knots.to_vec(),
        weights,
    )
    .expect("valid cubic spline");
    curve.evaluate(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn interpolate(fit: &[Point3]) -> (Vec<f64>, Vec<Point3>, Vec<f64>) {
        let (knots, cps) = fit_points_to_cubic_spline(fit).expect("constructs");
        let params = chord_parameters(fit).expect("params");
        (knots, cps, params)
    }

    #[test]
    fn control_point_count_helper_encodes_the_minimum() {
        assert_eq!(MIN_FIT_POINTS_FOR_CUBIC_SPLINE, 4);
        assert_eq!(cubic_spline_control_point_count(0), None);
        assert_eq!(cubic_spline_control_point_count(3), None);
        assert_eq!(cubic_spline_control_point_count(4), Some(4));
        assert_eq!(cubic_spline_control_point_count(7), Some(7));
    }

    #[test]
    fn fit_spline_rejects_fewer_than_four_fit_points() {
        for n in 0..4usize {
            let fit: Vec<Point3> = (0..n).map(|i| p(i as f64, 0.0)).collect();
            assert!(
                fit_points_to_cubic_spline(&fit).is_err(),
                "n = {n} must be refused"
            );
        }
    }

    #[test]
    fn fit_spline_knots_are_clamped_and_length_matches_n_plus_degree_plus_one() {
        for n in [4usize, 5, 8] {
            let fit: Vec<Point3> = (0..n).map(|i| p(i as f64, ((i * i) % 5) as f64)).collect();
            let (knots, cps) = fit_points_to_cubic_spline(&fit).expect("constructs");
            let degree = FIT_SPLINE_DEGREE as usize;
            // The importer's `Complete` shape: knots.len() == cp + degree + 1.
            assert_eq!(knots.len(), cps.len() + degree + 1);
            assert_eq!(cps.len(), n);
            // End multiplicity degree + 1 on both ends.
            assert!(knots[..=degree].iter().all(|k| *k == 0.0), "n={n}");
            assert!(knots[n..].iter().all(|k| *k == 1.0), "n={n}");
            // Non-decreasing, and interior knots strictly increasing (C2).
            assert!(knots.windows(2).all(|w| w[1] >= w[0]));
            assert!(knots[degree..=n].windows(2).all(|w| w[1] > w[0]));
        }
    }

    #[test]
    fn fit_spline_interpolates_fit_points_at_chord_parameters() {
        let fit = vec![
            p(0.0, 0.0),
            p(1.0, 2.0),
            p(2.5, 2.5),
            p(4.0, 1.0),
            p(5.5, -1.0),
            p(6.0, 0.5),
        ];
        let (knots, cps, params) = interpolate(&fit);
        for (i, q) in fit.iter().enumerate() {
            let got = evaluate_curve(&knots, &cps, params[i]);
            assert!(
                distance(got, *q) < 1e-9,
                "fit point {i}: got {got:?} want {q:?}"
            );
        }
        // Endpoints are exactly the first/last fit points.
        assert!(distance(evaluate_curve(&knots, &cps, 0.0), fit[0]) < 1e-12);
        assert!(distance(evaluate_curve(&knots, &cps, 1.0), fit[fit.len() - 1]) < 1e-12);
    }

    #[test]
    fn fit_spline_uses_chord_not_uniform_parameterisation() {
        // Strongly unequal spacing: most of the length is in the last chord.
        let fit = vec![p(0.0, 0.0), p(1.0, 0.0), p(1.1, 0.0), p(11.0, 0.0)];
        let (_knots, cps, params) = interpolate(&fit);
        let n = fit.len();
        // The parameters are definitely not the uniform index sequence.
        for (i, t) in params.iter().enumerate() {
            let uniform = i as f64 / (n - 1) as f64;
            if i != 0 && i != n - 1 {
                assert!(
                    (t - uniform).abs() > 0.1,
                    "param {i} = {t} too close to uniform {uniform}"
                );
            }
        }
        assert!((params[2] - 1.1 / 11.0).abs() < 1e-12);
        // Evaluating the same curve at the uniform parameters does *not*
        // reproduce the fit points: only the chord parameters do.
        let (knots, _) = fit_points_to_cubic_spline(&fit).expect("constructs");
        let mut worst = 0.0f64;
        for (i, q) in fit.iter().enumerate() {
            let uniform = i as f64 / (n - 1) as f64;
            worst = worst.max(distance(evaluate_curve(&knots, &cps, uniform), *q));
        }
        assert!(
            worst > 0.5,
            "uniform indexing should visibly miss the fit points, worst = {worst}"
        );
    }

    #[test]
    fn fit_spline_is_c2_continuous_at_interior_knots() {
        let fit = vec![
            p(0.0, 0.0),
            p(1.0, 1.0),
            p(2.0, 0.5),
            p(3.0, 2.0),
            p(4.0, 1.0),
            p(5.0, 0.0),
        ];
        let (knots, cps, _) = interpolate(&fit);
        let degree = FIT_SPLINE_DEGREE as usize;
        // Interior knots (multiplicity 1) -> C2 by construction; assert the
        // second derivative agrees across each with a one-sided finite
        // difference. A one-sided second difference has O(h) truncation error
        // proportional to the third derivative (which *does* jump at the knot,
        // C2 but not C3), so the tolerance is relative to the curvature rather
        // than absolute. A genuine C1-only discontinuity would be O(f''), i.e.
        // orders of magnitude larger than this bound.
        let h = 1e-5f64;
        for knot in knots.iter().take(fit.len()).skip(degree + 1) {
            let u = *knot;
            let fm2 = evaluate_curve(&knots, &cps, u - 2.0 * h);
            let fm1 = evaluate_curve(&knots, &cps, u - h);
            let f0 = evaluate_curve(&knots, &cps, u);
            let fp1 = evaluate_curve(&knots, &cps, u + h);
            let fp2 = evaluate_curve(&knots, &cps, u + 2.0 * h);
            let left = (f0.x - 2.0 * fm1.x + fm2.x) / (h * h);
            let right = (fp2.x - 2.0 * fp1.x + f0.x) / (h * h);
            assert!(
                (left - right).abs() < 1e-2 * left.abs().max(1.0),
                "second derivative jumps at knot {u}: {left} vs {right}"
            );
            let left_y = (f0.y - 2.0 * fm1.y + fm2.y) / (h * h);
            let right_y = (fp2.y - 2.0 * fp1.y + f0.y) / (h * h);
            assert!(
                (left_y - right_y).abs() < 1e-2 * left_y.abs().max(1.0),
                "second derivative jumps at knot {u}: {left_y} vs {right_y}"
            );
        }
    }

    #[test]
    fn fit_spline_rejects_non_finite_duplicate_and_coincident_points() {
        // Non-finite coordinate.
        assert!(fit_points_to_cubic_spline(&[
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(f64::NAN, 0.0),
            p(2.0, 0.0),
        ])
        .is_err());
        assert!(fit_points_to_cubic_spline(&[
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(2.0, 0.0),
            p(3.0, f64::INFINITY),
        ])
        .is_err());
        // Consecutive duplicate point (zero chord).
        assert!(
            fit_points_to_cubic_spline(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 0.0), p(2.0, 0.0),])
                .is_err()
        );
        // All points coincident.
        assert!(fit_points_to_cubic_spline(&[p(1.0, 1.0); 4]).is_err());
    }

    #[test]
    fn fit_spline_accepts_a_straight_collinear_run() {
        let fit = vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0), p(3.0, 0.0)];
        let (knots, cps, params) = interpolate(&fit);
        // A cubic can represent the straight line exactly, so it is reproduced.
        for (i, q) in fit.iter().enumerate() {
            let got = evaluate_curve(&knots, &cps, params[i]);
            assert!(distance(got, *q) < 1e-9, "point {i}: {got:?}");
            assert!(got.y.abs() < 1e-9, "point {i} left the line: {got:?}");
        }
    }

    #[test]
    fn fit_spline_refuses_near_coincident_points_explicitly() {
        // Spacing 1e-9 in a 1.0-scale drawing: below MIN_RELATIVE_CHORD, so the
        // construction refuses rather than returning an oscillating interpolant.
        let fit = vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0 + 1e-9, 0.0), p(2.0, 0.0)];
        let err = fit_points_to_cubic_spline(&fit).expect_err("must refuse");
        match err {
            CadError::InvalidInput(msg) => assert!(msg.contains("degenerate chord"), "{msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(chord_parameters(&fit).is_err());
    }

    #[test]
    fn fit_spline_handles_a_mild_cluster_without_refusing() {
        // A cluster well above the guard must still interpolate accurately.
        let fit = vec![p(0.0, 0.0), p(1.0, 0.5), p(1.001, 0.6), p(2.0, 0.0)];
        let (knots, cps, params) = interpolate(&fit);
        for (i, q) in fit.iter().enumerate() {
            let got = evaluate_curve(&knots, &cps, params[i]);
            assert!(
                distance(got, *q) < 1e-6,
                "cluster point {i}: got {got:?} want {q:?}"
            );
        }
    }
}
