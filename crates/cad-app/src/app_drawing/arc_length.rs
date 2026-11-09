//! The adaptive chord-length table shared by BREAK, DIVIDE and LENGTHEN.
//!
//! Split out of `break_curve.rs` so every module stays inside the code-size
//! limit. A curve with no analytic arc-length form is sampled on `[0, 1]` and the
//! table inverts a target length into its parameter.

use cad_domain::Point3;
use cad_geometry::distance;

/// A chord-length table over `[0, 1]`, adaptively sampled to a scale-relative
/// chord tolerance.
pub(crate) struct ArcLengthTable {
    parameters: Vec<f64>,
    /// `cumulative[i]` is the chord length from `0` to `parameters[i]`.
    cumulative: Vec<f64>,
}

impl ArcLengthTable {
    pub(super) fn sample(point_at: &impl Fn(f64) -> Point3) -> Self {
        // Seed with a coarse grid so a highly curved feature cannot be skipped by
        // the first bisection, then refine every interval by its chord error.
        const SEEDS: usize = 16;
        let mut parameters = vec![0.0];
        let mut points = vec![point_at(0.0)];
        let seed_points: Vec<Point3> = (0..=SEEDS)
            .map(|index| point_at(index as f64 / SEEDS as f64))
            .collect();
        let extent = bounding_extent(&seed_points);
        let tolerance = (extent * 1e-6).max(1e-12);
        for index in 0..SEEDS {
            let a = index as f64 / SEEDS as f64;
            let b = (index + 1) as f64 / SEEDS as f64;
            refine_arc(
                point_at,
                a,
                seed_points[index],
                b,
                seed_points[index + 1],
                12,
                tolerance,
                &mut parameters,
                &mut points,
            );
        }
        let mut cumulative = Vec::with_capacity(parameters.len());
        cumulative.push(0.0);
        for index in 1..points.len() {
            let step = distance(points[index - 1], points[index]);
            let next = cumulative.last().copied().unwrap_or(0.0) + step;
            cumulative.push(next);
        }
        Self {
            parameters,
            cumulative,
        }
    }

    /// The total sampled chord length.
    pub(super) fn total(&self) -> f64 {
        *self.cumulative.last().unwrap_or(&0.0)
    }

    /// The parameter at `target` arc length (linear inside the sampled segment).
    pub(super) fn parameter_at(&self, target: f64) -> f64 {
        let total = *self.cumulative.last().unwrap_or(&0.0);
        if !target.is_finite() || target <= 0.0 || total <= 0.0 {
            return 0.0;
        }
        if target >= total {
            return 1.0;
        }
        let upper = self
            .cumulative
            .partition_point(|length| *length < target)
            .max(1);
        let lower = upper - 1;
        let span = self.cumulative[upper] - self.cumulative[lower];
        if !span.is_finite() || span <= 0.0 {
            return self.parameters[upper];
        }
        let fraction = (target - self.cumulative[lower]) / span;
        self.parameters[lower] + (self.parameters[upper] - self.parameters[lower]) * fraction
    }
}

/// Refine `[a, b]` until every chord is within `tolerance` of the sampled curve.
#[allow(clippy::too_many_arguments)]
fn refine_arc(
    point_at: &impl Fn(f64) -> Point3,
    a: f64,
    point_a: Point3,
    b: f64,
    point_b: Point3,
    depth: u32,
    tolerance: f64,
    parameters: &mut Vec<f64>,
    points: &mut Vec<Point3>,
) {
    let midpoint = 0.5 * (a + b);
    let point_mid = point_at(midpoint);
    let chord = distance(point_a, point_b);
    let split = distance(point_a, point_mid) + distance(point_mid, point_b);
    if depth == 0 || split - chord <= tolerance {
        parameters.push(b);
        points.push(point_b);
        return;
    }
    refine_arc(
        point_at,
        a,
        point_a,
        midpoint,
        point_mid,
        depth - 1,
        tolerance,
        parameters,
        points,
    );
    refine_arc(
        point_at,
        midpoint,
        point_mid,
        b,
        point_b,
        depth - 1,
        tolerance,
        parameters,
        points,
    );
}

/// The diagonal of the bounding box of a point set (a scale for the tolerance).
fn bounding_extent(points: &[Point3]) -> f64 {
    let mut min = Point3 {
        x: f64::INFINITY,
        y: f64::INFINITY,
        z: f64::INFINITY,
    };
    let mut max = Point3 {
        x: f64::NEG_INFINITY,
        y: f64::NEG_INFINITY,
        z: f64::NEG_INFINITY,
    };
    for point in points {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }
    distance(min, max)
}
