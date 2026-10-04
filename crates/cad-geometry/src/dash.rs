//! Linetype dash subdivision (spec §3.2 / §7.1).
//!
//! A linetype is an ordered list of elements measured along the line: a
//! **positive** length is a dash, a **negative** length is a gap, and **zero**
//! is a dot. This module turns one open polyline into the dash sub-polylines
//! that a renderer actually draws, working in *arc length* so straight
//! segments and tessellated curves behave identically.
//!
//! The function is deliberately total: a degenerate or non-finite pattern (or a
//! zero-length polyline) never panics and never returns a partially mangled
//! result. Instead it reports a [`DashOutcome`] so a caller can fall back to
//! the continuous line with an explicit `Partial` reason rather than silently
//! drawing nothing.

use cad_domain::Point3;

/// Why a dash subdivision could not be produced exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashIssue {
    /// The pattern has no elements; the line is continuous.
    EmptyPattern,
    /// The pattern's cycle length is zero (all elements are zero) or non-finite.
    DegenerateCycle,
    /// Every element is a gap; the line would be invisible.
    AllGap,
    /// The scale factor is non-finite or not strictly positive.
    InvalidScale,
    /// The bounded dash expansion cannot complete; partial truncation is forbidden.
    ExpansionLimit,
}

impl DashIssue {
    /// Stable diagnostic reason string, suitable for a `Completeness::Partial`.
    pub fn reason(&self) -> &'static str {
        match self {
            DashIssue::EmptyPattern => "linetype pattern has no elements",
            DashIssue::DegenerateCycle => "linetype pattern has a degenerate cycle length",
            DashIssue::AllGap => "linetype pattern contains only gaps",
            DashIssue::InvalidScale => "linetype scale is not a positive finite number",
            DashIssue::ExpansionLimit => {
                "linetype subdivision exceeded its work limit or made no numeric progress"
            }
        }
    }
}

/// The result of subdividing one polyline by a dash pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum DashOutcome {
    /// The polyline was split into visible dash sub-polylines. May be empty
    /// when the line is shorter than the first dash element.
    Dashed(Vec<Vec<Point3>>),
    /// Subdivision was not possible; the caller must draw the input unchanged
    /// and report `issue` as a `Partial` reason.
    Continuous(DashIssue),
}

/// Independent endpoint pairs for opaque display batching. The same dash/gap
/// placement is retained without allocating a vector for each visible run.
pub fn dash_polyline_segments(
    points: &[Point3],
    pattern: &[f64],
    scale: f64,
) -> Result<Vec<Point3>, DashIssue> {
    if pattern.is_empty() {
        return Err(DashIssue::EmptyPattern);
    }
    if !scale.is_finite() || scale <= 0.0 {
        return Err(DashIssue::InvalidScale);
    }
    let slots: Vec<f64> = pattern.iter().map(|e| e * scale).collect();
    let cycle: f64 = slots.iter().map(|e| e.abs()).sum();
    if !cycle.is_finite() || cycle <= 0.0 || points.len() < 2 {
        return Err(DashIssue::DegenerateCycle);
    }
    if slots.iter().all(|e| *e < 0.0) {
        return Err(DashIssue::AllGap);
    }
    let mut lengths = Vec::with_capacity(points.len());
    lengths.push(0.0);
    for pair in points.windows(2) {
        let a = pair[0];
        let b = pair[1];
        let length = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt();
        if !length.is_finite() {
            return Err(DashIssue::DegenerateCycle);
        }
        lengths.push(lengths.last().copied().unwrap() + length);
    }
    let total = *lengths.last().unwrap();
    if !total.is_finite() || total <= 0.0 {
        return Err(DashIssue::DegenerateCycle);
    }
    let at = |distance: f64| point_at(points, &lengths, distance).expect("validated arc index");
    let mut out = Vec::new();
    let mut emit_span = |start: f64, end: f64| {
        let mut previous = at(start);
        let first = lengths.partition_point(|d| *d <= start);
        let last = lengths.partition_point(|d| *d < end);
        for point in points[first..last]
            .iter()
            .copied()
            .chain(std::iter::once(at(end)))
        {
            if !same_point(&previous, &point) {
                out.extend_from_slice(&[previous, point]);
                previous = point;
            }
        }
    };
    // Without a gap the entire polyline is visible, regardless of positive
    // slot boundaries. Avoid inventing extra joins or iterating tiny slots.
    if slots.iter().all(|e| *e >= 0.0) {
        emit_span(0.0, total);
        return Ok(out);
    }
    let mut slot = slots.iter().position(|e| *e >= 0.0).unwrap();
    let mut distance = 0.0;
    let mut visible_start = None;
    let mut runs = 0usize;
    while distance < total {
        let len = slots[slot];
        if len < 0.0 {
            if let Some(start) = visible_start.take() {
                emit_span(start, distance.min(total));
                runs += 1;
                if runs > 1_000_000 {
                    return Err(DashIssue::ExpansionLimit);
                }
            }
        } else if len > 0.0 {
            visible_start.get_or_insert(distance);
        }
        if len != 0.0 {
            let next = distance + len.abs();
            if !next.is_finite() || next <= distance {
                return Err(DashIssue::ExpansionLimit);
            }
            distance = next.min(total);
        }
        slot = (slot + 1) % slots.len();
    }
    if let Some(start) = visible_start {
        if runs >= 1_000_000 {
            return Err(DashIssue::ExpansionLimit);
        }
        emit_span(start, total);
    }
    Ok(out)
}

/// Total arc length of an open polyline (0 for fewer than two points, or any
/// non-finite segment which is ignored).
pub fn polyline_length(points: &[Point3]) -> f64 {
    let mut total = 0.0f64;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let seg = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt();
        if seg.is_finite() {
            total += seg;
        }
    }
    total
}

/// A point at arc-length `distance` along the open polyline.
///
/// Distances before the start clamp to the first point, distances past the end
/// to the last point. `None` only when a segment length is non-finite.
fn point_at(points: &[Point3], arc: &[f64], distance: f64) -> Option<Point3> {
    if distance <= 0.0 {
        return points.first().copied();
    }
    let end = arc.partition_point(|d| *d < distance);
    if end >= points.len() {
        return points.last().copied();
    }
    let (a, b) = (points[end - 1], points[end]);
    let seg = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt();
    let t = if seg > 0.0 {
        ((distance - arc[end - 1]) / seg).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some(Point3 {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        z: a.z + (b.z - a.z) * t,
    })
}

/// Vertices strictly inside `(start, end]` of the polyline arc, in order.
fn vertices_in_span<'a>(points: &'a [Point3], arc: &[f64], start: f64, end: f64) -> &'a [Point3] {
    let first = arc.partition_point(|d| *d <= start);
    let last = arc.partition_point(|d| *d <= end);
    &points[first..last]
}

fn dedup_run(run: &mut Vec<Point3>) {
    run.dedup_by(|a, b| {
        (a.x - b.x).abs() < 1e-12 && (a.y - b.y).abs() < 1e-12 && (a.z - b.z).abs() < 1e-12
    });
}

fn append_run(current: &mut Vec<Point3>, run: Vec<Point3>) {
    if run.len() < 2 {
        return;
    }
    if let Some(last) = current.last() {
        if same_point(last, &run[0]) {
            current.extend(run.into_iter().skip(1));
            return;
        }
    }
    current.extend(run);
}

fn same_point(a: &Point3, b: &Point3) -> bool {
    (a.x - b.x).abs() < 1e-12 && (a.y - b.y).abs() < 1e-12 && (a.z - b.z).abs() < 1e-12
}

/// Repeatedly apply `pattern` (scaled by `scale`) along the arc length of an
/// open polyline.
///
/// The pattern restarts for every polyline (each fragment is an independent
/// entity), matching AutoCAD's `A` alignment for open geometry. A leading gap
/// is skipped so the line always begins with the first dash. A dot (zero
/// element) emits a degenerate run that a line-only caller drops; this keeps
/// dot-only patterns active without producing an infinite loop.
pub fn dash_polyline(points: &[Point3], pattern: &[f64], scale: f64) -> DashOutcome {
    if pattern.is_empty() {
        return DashOutcome::Continuous(DashIssue::EmptyPattern);
    }
    if !scale.is_finite() || scale <= 0.0 {
        return DashOutcome::Continuous(DashIssue::InvalidScale);
    }
    let cycle: f64 = pattern.iter().map(|e| e.abs()).sum();
    if !cycle.is_finite() || cycle <= 0.0 {
        return DashOutcome::Continuous(DashIssue::DegenerateCycle);
    }
    if pattern.iter().all(|e| *e < 0.0) {
        return DashOutcome::Continuous(DashIssue::AllGap);
    }
    if points.len() < 2 {
        return DashOutcome::Continuous(DashIssue::DegenerateCycle);
    }
    let total = polyline_length(points);
    if !total.is_finite() || total <= 0.0 {
        return DashOutcome::Continuous(DashIssue::DegenerateCycle);
    }
    // Index arc lengths once. Re-scanning from the first vertex for every
    // microscopic dash made curved/polyline subdivision quadratic.
    let mut arc = Vec::with_capacity(points.len());
    arc.push(0.0);
    for pair in points.windows(2) {
        let distance = ((pair[1].x - pair[0].x).powi(2)
            + (pair[1].y - pair[0].y).powi(2)
            + (pair[1].z - pair[0].z).powi(2))
        .sqrt();
        let next = arc.last().copied().unwrap_or(0.0) + distance;
        if !next.is_finite() {
            return DashOutcome::Continuous(DashIssue::DegenerateCycle);
        }
        arc.push(next);
    }

    // Pre-compute each cycle's slots, skipping gaps so the first slot is always
    // a dash/dot. `slots` holds the element lengths of one repetition in order.
    let slots: Vec<f64> = pattern.iter().map(|e| e * scale).collect();
    if slots.iter().all(|e| e.abs() == 0.0) {
        return DashOutcome::Continuous(DashIssue::DegenerateCycle);
    }
    if slots.iter().all(|e| *e < 0.0) {
        return DashOutcome::Continuous(DashIssue::AllGap);
    }
    // A pattern whose leading elements are gaps is normal (e.g. `[gap, dash]`);
    // start at the first non-gap slot so the line begins with a dash, without
    // advancing past it (AutoCAD's `A` alignment starts at the first marker).
    let first_visible = slots
        .iter()
        .position(|e| *e >= 0.0)
        .expect("checked at least one non-gap");

    let mut out: Vec<Vec<Point3>> = Vec::new();
    let mut current: Vec<Point3> = Vec::new();
    let mut distance = 0.0;
    let mut slot = first_visible;
    // Bound the number of emitted runs so a tiny pattern over a huge line
    // cannot allocate without limit.
    const MAX_RUNS: usize = 1_000_000;
    let mut runs = 0usize;

    while distance <= total && runs < MAX_RUNS {
        let len = slots[slot];
        if len == 0.0 {
            // Dot: a zero-length run. Drawn as nothing by the line renderer but
            // counted so a dot-only pattern terminates.
            if let Some(p) = point_at(points, &arc, distance) {
                append_run(&mut current, vec![p, p]);
            }
            slot += 1;
        } else if len < 0.0 {
            // Gap: close the running dash and advance. A run of coincident
            // points (the residue of a dot) is not a line and is dropped.
            if current.len() >= 2 && polyline_length(&current) > 1e-12 {
                out.push(std::mem::take(&mut current));
                runs += 1;
            } else {
                current.clear();
            }
            distance += len.abs();
            slot += 1;
        } else {
            // Dash: sample it, breaking at every interior vertex.
            let start = distance;
            let end = distance + len;
            let mut run: Vec<Point3> = Vec::new();
            if let Some(p) = point_at(points, &arc, start) {
                run.push(p);
            }
            run.extend_from_slice(vertices_in_span(points, &arc, start, end));
            if let Some(p) = point_at(points, &arc, end) {
                run.push(p);
            }
            dedup_run(&mut run);
            append_run(&mut current, run);
            distance = end;
            slot += 1;
        }
        if slot >= slots.len() {
            slot = 0;
        }
    }

    if current.len() >= 2 && polyline_length(&current) > 1e-12 {
        out.push(current);
    }
    out.retain(|run| run.len() >= 2 && polyline_length(run) > 1e-12);
    DashOutcome::Dashed(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_dashes_match_run_segments_across_corners_and_duplicate_vertices() {
        let points = vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 3.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 3.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 3.0,
                y: 7.0,
                z: 0.0,
            },
        ];
        for pattern in [
            vec![2.0, -1.0],
            vec![-1.0, 2.0],
            vec![0.0, -1.0],
            vec![2.0, 0.0, -1.0],
        ] {
            let DashOutcome::Dashed(runs) = dash_polyline(&points, &pattern, 1.0) else {
                panic!("valid pattern rejected")
            };
            // Zero-length dot residues have no line rasterization footprint.
            let expected: Vec<Point3> = runs
                .iter()
                .flat_map(|run| {
                    run.windows(2)
                        .filter(|pair| !same_point(&pair[0], &pair[1]))
                        .flatten()
                        .copied()
                })
                .collect();
            assert_eq!(
                dash_polyline_segments(&points, &pattern, 1.0).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn packed_subdivision_reports_invalid_input_and_work_exhaustion() {
        assert_eq!(
            dash_polyline_segments(&line(10.0), &[f64::NAN], 1.0),
            Err(DashIssue::DegenerateCycle)
        );
        assert_eq!(
            dash_polyline_segments(&line(10.0), &[1.0, -1.0], 0.0),
            Err(DashIssue::InvalidScale)
        );
        assert_eq!(
            dash_polyline_segments(&line(10.0), &[0.000001, -0.000001], 1.0),
            Err(DashIssue::ExpansionLimit)
        );
    }

    fn line(len: f64) -> Vec<Point3> {
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: len,
                y: 0.0,
                z: 0.0,
            },
        ]
    }

    fn lengths(outcome: &DashOutcome) -> Vec<f64> {
        match outcome {
            DashOutcome::Dashed(runs) => runs
                .iter()
                .map(|run| {
                    let mut l = 0.0;
                    l += polyline_length(run);
                    l
                })
                .collect(),
            DashOutcome::Continuous(_) => Vec::new(),
        }
    }

    #[test]
    fn simple_dash_space_splits_a_straight_line() {
        // 10-unit line, dash 2 / gap 1 (cycle 3): dashes at 0-2, 3-5, 6-8, 9-10.
        let outcome = dash_polyline(&line(10.0), &[2.0, -1.0], 1.0);
        let lens = lengths(&outcome);
        assert_eq!(lens.len(), 4, "got {lens:?}");
        assert!((lens[0] - 2.0).abs() < 1e-9);
        assert!((lens[1] - 2.0).abs() < 1e-9);
        assert!((lens[2] - 2.0).abs() < 1e-9);
        assert!((lens[3] - 1.0).abs() < 1e-9, "last dash is clipped");
    }

    #[test]
    fn packed_dashes_preserve_visible_segments_and_corners() {
        let points = vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 3.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 3.0,
                y: 4.0,
                z: 0.0,
            },
        ];
        for pattern in [vec![2.0, -1.0], vec![-1.0, 2.0], vec![0.0, -1.0, 2.0]] {
            let DashOutcome::Dashed(runs) = dash_polyline(&points, &pattern, 1.0) else {
                panic!("valid pattern");
            };
            let expected: Vec<_> = runs
                .iter()
                .flat_map(|run| {
                    run.windows(2)
                        .filter(|pair| !same_point(&pair[0], &pair[1]))
                        .flatten()
                        .copied()
                })
                .collect();
            assert_eq!(
                dash_polyline_segments(&points, &pattern, 1.0).unwrap(),
                expected
            );
        }
        let packed = dash_polyline_segments(&points, &[1.0, 1.0], 1.0).unwrap();
        assert_eq!(packed, vec![points[0], points[1], points[1], points[2]]);
    }

    #[test]
    fn packed_dashes_reject_invalid_and_over_budget_expansion() {
        assert_eq!(
            dash_polyline_segments(&line(10.0), &[1.0, f64::NAN], 1.0),
            Err(DashIssue::DegenerateCycle)
        );
        assert_eq!(
            dash_polyline_segments(&line(10.0), &[1.0, -1.0], 0.0),
            Err(DashIssue::InvalidScale)
        );
        assert_eq!(
            dash_polyline_segments(&line(2_000_003.0), &[1.0, -1.0], 1.0),
            Err(DashIssue::ExpansionLimit)
        );
    }

    #[test]
    fn gap_count_matches_expectation() {
        // 7-unit line, dash 1 / gap 1 (cycle 2): dashes at [0,1],[2,3],[4,5],[6,7].
        let outcome = dash_polyline(&line(7.0), &[1.0, -1.0], 1.0);
        let lens = lengths(&outcome);
        assert_eq!(lens.len(), 4, "got {lens:?}");
        for l in lens {
            assert!((l - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn scale_multiplies_the_pattern() {
        // Same 10-unit line, but scale 2 -> dash 4 / gap 2.
        let outcome = dash_polyline(&line(10.0), &[2.0, -1.0], 2.0);
        let lens = lengths(&outcome);
        assert_eq!(lens.len(), 2, "got {lens:?}");
        assert!((lens[0] - 4.0).abs() < 1e-9);
        assert!((lens[1] - 4.0).abs() < 1e-9);
    }

    #[test]
    fn leading_gap_is_skipped_so_the_line_starts_with_a_dash() {
        // Pattern [gap 1, dash 2] over 5 units -> dash markers at 0-2.
        let outcome = dash_polyline(&line(5.0), &[-1.0, 2.0], 1.0);
        let lens = lengths(&outcome);
        // Dash [0,2]; then gap [2,3]; dash [3,5] clipped.
        assert_eq!(lens.len(), 2, "got {lens:?}");
        assert!((lens[0] - 2.0).abs() < 1e-9);
        assert!((lens[1] - 2.0).abs() < 1e-9, "got {lens:?}");
    }

    #[test]
    fn dot_pattern_terminates_and_yields_no_line_runs() {
        // A pattern of a dot and a gap is a real dotted linetype; it must not
        // hang and must not fabricate a dash.
        let outcome = dash_polyline(&line(5.0), &[0.0, -1.0], 1.0);
        match outcome {
            DashOutcome::Dashed(runs) => {
                // Every run collapsed to a dot and was removed.
                assert!(runs.is_empty(), "dots are not line runs: {runs:?}");
            }
            other => panic!("expected Dashed, got {other:?}"),
        }
    }

    #[test]
    fn dot_space_leaves_isolated_dashes() {
        // dash 1 / gap 1 / dot / gap 1 -> cycle 3.
        let outcome = dash_polyline(&line(9.0), &[1.0, -1.0, 0.0, -1.0], 1.0);
        let lens = lengths(&outcome);
        assert_eq!(lens.len(), 3, "got {lens:?}");
        for l in lens {
            assert!((l - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn curve_is_dashed_by_arc_length() {
        // Quarter circle radius 4: arc length ~ 6.283. With dash/gap 1/1 we
        // expect about 3-4 dash runs and each of length 1.
        let r = 4.0f64;
        let mut points = Vec::new();
        for i in 0..=64 {
            let t = std::f64::consts::FRAC_PI_2 * i as f64 / 64.0;
            points.push(Point3 {
                x: r * t.cos(),
                y: r * t.sin(),
                z: 0.0,
            });
        }
        let arc = polyline_length(&points);
        let outcome = dash_polyline(&points, &[1.0, -1.0], 1.0);
        let lens = lengths(&outcome);
        assert!(lens.len() >= 3, "got {lens:?}");
        // All but the last dash are ~1 unit of arc; the last is clipped by the
        // end of the curve.
        for l in &lens[..lens.len() - 1] {
            assert!((l - 1.0).abs() < 0.05, "dash length {l} off arc length");
        }
        let dashed_total: f64 = lens.iter().sum();
        assert!(dashed_total <= arc + 1e-6);
    }

    #[test]
    fn zero_pattern_falls_back_to_continuous() {
        assert_eq!(
            dash_polyline(&line(5.0), &[0.0, 0.0], 1.0),
            DashOutcome::Continuous(DashIssue::DegenerateCycle)
        );
        assert_eq!(
            dash_polyline(&line(5.0), &[], 1.0),
            DashOutcome::Continuous(DashIssue::EmptyPattern)
        );
    }

    #[test]
    fn all_gap_pattern_falls_back_to_continuous() {
        assert_eq!(
            dash_polyline(&line(5.0), &[-1.0, -2.0], 1.0),
            DashOutcome::Continuous(DashIssue::AllGap)
        );
    }

    #[test]
    fn invalid_scale_falls_back_to_continuous() {
        assert_eq!(
            dash_polyline(&line(5.0), &[1.0, -1.0], 0.0),
            DashOutcome::Continuous(DashIssue::InvalidScale)
        );
        assert_eq!(
            dash_polyline(&line(5.0), &[1.0, -1.0], f64::NAN),
            DashOutcome::Continuous(DashIssue::InvalidScale)
        );
        assert_eq!(
            dash_polyline(&line(5.0), &[1.0, -1.0], -1.0),
            DashOutcome::Continuous(DashIssue::InvalidScale)
        );
    }

    #[test]
    fn degenerate_line_falls_back_to_continuous() {
        assert_eq!(
            dash_polyline(&line(0.0), &[1.0, -1.0], 1.0),
            DashOutcome::Continuous(DashIssue::DegenerateCycle)
        );
    }

    #[test]
    fn non_finite_pattern_falls_back_to_continuous() {
        assert_eq!(
            dash_polyline(&line(5.0), &[f64::NAN, -1.0], 1.0),
            DashOutcome::Continuous(DashIssue::DegenerateCycle)
        );
        assert_eq!(
            dash_polyline(&line(5.0), &[f64::INFINITY], 1.0),
            DashOutcome::Continuous(DashIssue::DegenerateCycle)
        );
    }

    #[test]
    fn dash_follows_polyline_vertices() {
        // An L-shaped polyline of total length 4; dash 3 / gap 1 should produce
        // one run that turns the corner and one short run at the end.
        let points = vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 2.0,
                y: 2.0,
                z: 0.0,
            },
        ];
        let outcome = dash_polyline(&points, &[3.0, -1.0], 1.0);
        match outcome {
            DashOutcome::Dashed(runs) => {
                assert_eq!(runs.len(), 1, "one clipped dash fits in 4 units");
                assert_eq!(runs[0].len(), 3, "corner vertex is preserved");
                assert_eq!(runs[0][1].x, 2.0);
                assert_eq!(runs[0][1].y, 0.0);
            }
            other => panic!("expected Dashed, got {other:?}"),
        }
    }
}
