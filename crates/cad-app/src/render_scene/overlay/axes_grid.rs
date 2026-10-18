use super::*;

/// Build the world-space X/Y axes overlay for `bounds`.
///
/// `bounds` is the drawing's model-space `(min, max)` from
/// [`cad_db::DrawingDatabase::bounds`]. Both axes are one `Lines` batch of two
/// segments on the `z = 0` work plane: the X axis at `y = 0` and the Y axis at
/// `x = 0`. Each axis is drawn **symmetrically about the origin**, spanning
/// `[-extent, extent]` on that axis where `extent` is the larger absolute bound.
/// Clipping each axis to the one-sided model range made the origin cross read as
/// a lopsided "L"-ish shape whenever the drawing sat mostly on one side of the
/// origin (the reported "crosshair left arm too short"); mirroring keeps both
/// arms equal so it reads as a proper crosshair. The axes are reference geometry
/// and draw below the selection highlight.
pub fn axes_overlay(bounds: (Point3, Point3), document: DocumentId) -> VisualOverlay {
    let (min, max) = bounds;
    let extent_x = min.x.abs().max(max.x.abs());
    let extent_y = min.y.abs().max(max.y.abs());
    let points = vec![
        Point3 {
            x: -extent_x,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: extent_x,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: -extent_y,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: extent_y,
            z: 0.0,
        },
    ];
    let mut out = VisualOverlay::default();
    if let Some(batch) = line_batch(
        &points,
        AXES_COLOR,
        document,
        AXES_GRID_DRAW_ORDER + 1,
        "overlay.axes",
    ) {
        out.batches.push(batch);
    }
    out
}

/// Build the world-space reference grid overlay for `bounds`.
///
/// Grid lines are evenly spaced at a power-of-ten step chosen from each axis'
/// extent ([`grid_step`]); the whole grid is one `Lines` batch whose consecutive
/// vertex pairs are the individual segments. The per-axis line count is capped
/// by [`GRID_MAX_LINES`] so a pathological extent cannot grow the batch without
/// bound. Like the axes, the grid draws below the selection highlight.
pub fn grid_overlay(bounds: (Point3, Point3), document: DocumentId) -> VisualOverlay {
    let (min, max) = bounds;
    let per_axis = GRID_MAX_LINES / 2;
    let span = (max.x - min.x).abs().max((max.y - min.y).abs());
    // At large pan offsets, a nominal zoom-in step can be smaller than one
    // representable f64 increment at the coordinates. Keep adjacent ticks
    // distinguishable instead of emitting duplicate lines.
    let coordinate_scale = [min.x, min.y, max.x, max.y]
        .into_iter()
        .map(f64::abs)
        .fold(1.0_f64, f64::max);
    let step = grid_step(span).max(coordinate_scale * f64::EPSILON * 8.0);
    let xs = grid_positions(min.x, max.x, step, per_axis);
    let ys = grid_positions(min.y, max.y, step, per_axis);
    let mut points: Vec<Point3> = Vec::with_capacity((xs.len() + ys.len()) * 2);
    for x in xs {
        points.push(Point3 {
            x,
            y: min.y,
            z: 0.0,
        });
        points.push(Point3 {
            x,
            y: max.y,
            z: 0.0,
        });
    }
    for y in ys {
        points.push(Point3 {
            x: min.x,
            y,
            z: 0.0,
        });
        points.push(Point3 {
            x: max.x,
            y,
            z: 0.0,
        });
    }
    let mut out = VisualOverlay::default();
    if let Some(batch) = line_batch(
        &points,
        GRID_COLOR,
        document,
        AXES_GRID_DRAW_ORDER,
        "overlay.grid",
    ) {
        out.batches.push(batch);
    }
    out
}

/// Pick a "nice" 1/2/5 × 10ⁿ grid step targeting roughly ten divisions.
///
/// Rounding straight to a power of ten can jump from ~15 to ~40 divisions for a
/// small change in extent; snapping to the 1/2/5 sequence keeps the grid between
/// about 7 and 15 divisions so an explicitly enabled grid stays readable. A
/// non-finite or degenerate extent falls back to `1.0`; the caller's count cap
/// bounds the result regardless.
fn grid_step(extent: f64) -> f64 {
    if !extent.is_finite() || extent <= 0.0 {
        return 1.0;
    }
    let target = extent / 12.0;
    let magnitude = 10f64.powf(target.log10().floor());
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return 1.0;
    }
    let normalized = target / magnitude;
    let nice = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = nice * magnitude;
    if step.is_finite() && step > 0.0 {
        step
    } else {
        1.0
    }
}

/// The multiples of `step` inside `[min, max]`, capped at `cap` entries.
///
/// When the raw count exceeds `cap`, every `stride`-th multiple is kept so the
/// grid still spans the whole range instead of shrinking into one corner.
pub(super) fn grid_positions(min: f64, max: f64, step: f64, cap: usize) -> Vec<f64> {
    let mut out = Vec::new();
    if cap == 0
        || !(min.is_finite() && max.is_finite() && step.is_finite())
        || step <= 0.0
        || max <= min
    {
        return out;
    }
    let start = (min / step).ceil();
    let end = (max / step).floor();
    if !(start.is_finite() && end.is_finite()) || end < start {
        return out;
    }
    let count = (end - start + 1.0) as usize;
    let stride = count.div_ceil(cap).max(1);
    let mut index = start;
    while index <= end && out.len() < cap {
        out.push(index * step);
        index += stride as f64;
    }
    out
}
