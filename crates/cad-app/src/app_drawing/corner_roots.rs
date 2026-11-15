//! Shared one-parameter root scan for the numeric FILLET forms.
//!
//! The ELLIPSE form ([`super::corner_ellipse`]) and the SPLINE form
//! ([`super::corner_spline`]) reduce a FILLET to roots of one black-box residual
//! `f(t) = distance(centre(t), other) − R` over a participant's own parameter
//! domain. That residual is smooth but has no closed form once one side is a
//! general B-spline, so the search is numeric — and it is written once here so
//! both forms sample, bracket and refine identically (and therefore pick the same
//! kind of winner deterministically).
//!
//! Two root shapes are covered:
//!
//! * a **sign change** between adjacent samples is a transverse crossing and is
//!   bisected to the parameter tolerance;
//! * a **tangential** solution does not change sign — the residual only touches
//!   zero — so it is a local minimum of `|residual|`. The residual is quadratic
//!   there, which locates θ only to `~√ε`, so Newton runs on `residual′ = 0`
//!   (transverse at a minimum) instead. The caller keeps the acceptance test: a
//!   stationary point that does not actually vanish is not a tangency.
//!
//! Nothing here decides feasibility, trimming or refusal: it returns parameter
//! candidates only.

/// Root candidates of `residual` over `[low, high]` from `samples + 1` uniform
/// samples (sign changes bisected, tangential minima refined).
///
/// Samples where the residual is not finite are skipped, so a caller may signal
/// "no geometry here" with `None` (an undefined normal, a projection that fails)
/// without inventing a bracket. The returned order follows the sample order, so
/// callers that keep the first winner stay deterministic.
pub(super) fn scan_roots(
    residual: &dyn Fn(f64) -> Option<f64>,
    low: f64,
    high: f64,
    samples: usize,
) -> Vec<f64> {
    if !low.is_finite() || !high.is_finite() || high <= low || samples == 0 {
        return Vec::new();
    }
    let mut values: Vec<(f64, f64)> = Vec::with_capacity(samples + 1);
    for index in 0..=samples {
        let parameter = low + (high - low) * (index as f64) / (samples as f64);
        values.push((parameter, residual(parameter).unwrap_or(f64::NAN)));
    }
    let mut roots = Vec::new();
    for window in values.windows(2) {
        let (low_parameter, low_value) = window[0];
        let (high_parameter, high_value) = window[1];
        if !low_value.is_finite() || !high_value.is_finite() {
            continue;
        }
        if low_value == 0.0 && high_value == 0.0 {
            roots.push(low_parameter);
            continue;
        }
        if low_value * high_value < 0.0 {
            if let Some(root) = bisect_root(residual, low_parameter, high_parameter) {
                roots.push(root);
            }
        }
    }
    for index in 1..values.len().saturating_sub(1) {
        let (low_parameter, low_value) = values[index - 1];
        let (_, middle_value) = values[index];
        let (high_parameter, high_value) = values[index + 1];
        if !low_value.is_finite() || !middle_value.is_finite() || !high_value.is_finite() {
            continue;
        }
        if middle_value.abs() <= low_value.abs() && middle_value.abs() <= high_value.abs() {
            if let Some(root) = refine_stationary_root(residual, low_parameter, high_parameter) {
                roots.push(root);
            }
        }
    }
    roots
}

/// Bisect a sign-changing bracket of `residual` (80 halvings, then the last
/// midpoint: the parameter tolerance of a double-precision domain).
pub(super) fn bisect_root(
    residual: &dyn Fn(f64) -> Option<f64>,
    low: f64,
    high: f64,
) -> Option<f64> {
    let (mut low, mut high) = (low, high);
    let mut low_value = residual(low)?;
    let high_value = residual(high)?;
    if !low_value.is_finite() || !high_value.is_finite() {
        return None;
    }
    for _ in 0..80 {
        let middle = 0.5 * (low + high);
        let middle_value = residual(middle)?;
        if !middle_value.is_finite() {
            return None;
        }
        if middle_value == 0.0 {
            return Some(middle);
        }
        if low_value * middle_value < 0.0 {
            high = middle;
        } else {
            low = middle;
            low_value = middle_value;
        }
    }
    Some(0.5 * (low + high))
}

/// Newton on `residual′ = 0` inside a bracket around a tangential minimum.
///
/// The derivatives are central differences of the same residual (the residual is
/// a black box for both profiled forms), and the step is accepted only while the
/// second derivative is positive — a maximum or a flat window stops the loop and
/// the caller's acceptance test rejects it.
pub(super) fn refine_stationary_root(
    residual: &dyn Fn(f64) -> Option<f64>,
    low: f64,
    high: f64,
) -> Option<f64> {
    let mut parameter = 0.5 * (low + high);
    if !parameter.is_finite() {
        return None;
    }
    for _ in 0..20 {
        let step_size = 1e-6 * (1.0 + parameter.abs());
        let middle = residual(parameter)?;
        let plus = residual(parameter + step_size)?;
        let minus = residual(parameter - step_size)?;
        if !middle.is_finite() || !plus.is_finite() || !minus.is_finite() {
            break;
        }
        let first = (plus - minus) / (2.0 * step_size);
        let second = (plus - 2.0 * middle + minus) / (step_size * step_size);
        if !first.is_finite() || !second.is_finite() || second <= 0.0 {
            break;
        }
        let step = first / second;
        if !step.is_finite() {
            break;
        }
        parameter -= step;
        if step.abs() <= 1e-15 * (1.0 + parameter.abs()) {
            break;
        }
    }
    parameter.is_finite().then_some(parameter)
}

/// Golden-section minimum of a scalar function on `[low, high]`.
///
/// Used for the one shape the residual scan cannot bracket: the distance between
/// two *profiled* curves, which is non-negative and has no sign to change at a
/// crossing. A transverse crossing makes that distance a V, so the bracket is
/// narrowed by comparing two interior points (not by a derivative) and the
/// caller accepts the result only when the minimum is within its own tolerance.
pub(super) fn minimise_scalar(
    value: &dyn Fn(f64) -> f64,
    low: f64,
    high: f64,
    iterations: usize,
) -> f64 {
    let ratio = 0.5 * (5.0_f64.sqrt() - 1.0);
    let (mut low, mut high) = (low, high);
    for _ in 0..iterations {
        let span = high - low;
        if !span.is_finite() || span <= 0.0 {
            break;
        }
        let lower = high - ratio * span;
        let upper = low + ratio * span;
        if value(lower) <= value(upper) {
            high = upper;
        } else {
            low = lower;
        }
    }
    0.5 * (low + high)
}
