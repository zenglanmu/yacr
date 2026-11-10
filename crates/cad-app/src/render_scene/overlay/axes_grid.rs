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

/// Build the two-level world-space reference grid overlay for `bounds`.
///
/// The style follows AutoCAD's lined model-space grid (`GRIDSTYLE = 0`, the
/// modern default): a **minor** line at the adaptive step and a brighter
/// **major** line every [`GRID_MAJOR_INTERVAL`] intervals (AutoCAD's
/// `GRIDMAJOR` initial value, `5`, so four minor lines sit between two majors).
/// Minor and major lines are separate `Lines` batches because the renderer
/// carries one colour per batch: minors use [`GRID_COLOR`], majors
/// [`GRID_MAJOR_COLOR`]. Both carry [`AXES_GRID_DRAW_ORDER`], so the world axes
/// (`+1`) still draw above the grid. Like the axes, the grid draws below the
/// selection highlight and every transient affordance.
///
/// The per-axis line counts are capped separately
/// ([`GRID_MAX_MINOR_PER_AXIS`], [`GRID_MAX_MAJOR_PER_AXIS`]) so the two levels
/// together can never exceed [`GRID_MAX_LINES`] segments; a pathological extent
/// samples across the whole range instead of shrinking into one corner.
pub fn grid_overlay(bounds: (Point3, Point3), document: DocumentId) -> VisualOverlay {
    let (min, max) = bounds;
    let span = (max.x - min.x).abs().max((max.y - min.y).abs());
    // At large pan offsets, a nominal zoom-in step can be smaller than one
    // representable f64 increment at the coordinates. Keep adjacent ticks
    // distinguishable instead of emitting duplicate lines.
    let coordinate_scale = [min.x, min.y, max.x, max.y]
        .into_iter()
        .map(f64::abs)
        .fold(1.0_f64, f64::max);
    let step = grid_step(span).max(coordinate_scale * f64::EPSILON * 8.0);
    let major_step = step * f64::from(GRID_MAJOR_INTERVAL);
    let xs_minor = minor_grid_positions(
        min.x,
        max.x,
        step,
        GRID_MAJOR_INTERVAL,
        GRID_MAX_MINOR_PER_AXIS,
    );
    let ys_minor = minor_grid_positions(
        min.y,
        max.y,
        step,
        GRID_MAJOR_INTERVAL,
        GRID_MAX_MINOR_PER_AXIS,
    );
    let xs_major = grid_positions(min.x, max.x, major_step, GRID_MAX_MAJOR_PER_AXIS);
    let ys_major = grid_positions(min.y, max.y, major_step, GRID_MAX_MAJOR_PER_AXIS);
    let mut minor_points: Vec<Point3> = Vec::with_capacity((xs_minor.len() + ys_minor.len()) * 2);
    for x in xs_minor {
        minor_points.push(Point3 {
            x,
            y: min.y,
            z: 0.0,
        });
        minor_points.push(Point3 {
            x,
            y: max.y,
            z: 0.0,
        });
    }
    for y in ys_minor {
        minor_points.push(Point3 {
            x: min.x,
            y,
            z: 0.0,
        });
        minor_points.push(Point3 {
            x: max.x,
            y,
            z: 0.0,
        });
    }
    let mut major_points: Vec<Point3> = Vec::with_capacity((xs_major.len() + ys_major.len()) * 2);
    for x in xs_major {
        major_points.push(Point3 {
            x,
            y: min.y,
            z: 0.0,
        });
        major_points.push(Point3 {
            x,
            y: max.y,
            z: 0.0,
        });
    }
    for y in ys_major {
        major_points.push(Point3 {
            x: min.x,
            y,
            z: 0.0,
        });
        major_points.push(Point3 {
            x: max.x,
            y,
            z: 0.0,
        });
    }
    let mut out = VisualOverlay::default();
    if let Some(batch) = line_batch(
        &minor_points,
        GRID_COLOR,
        document,
        AXES_GRID_DRAW_ORDER,
        "overlay.grid",
    ) {
        out.batches.push(batch);
    }
    if let Some(batch) = line_batch(
        &major_points,
        GRID_MAJOR_COLOR,
        document,
        AXES_GRID_DRAW_ORDER,
        "overlay.grid-major",
    ) {
        out.batches.push(batch);
    }
    out
}

/// Pick a "nice" 1/2/5 × 10ⁿ **minor** grid step targeting roughly
/// [`GRID_TARGET_MINOR_DIVISIONS`] divisions across the larger view extent.
///
/// The nice-step chooser yields between about half and all of the target, so the
/// minor spacing stays readable while zooming and the majors
/// ([`GRID_MAJOR_INTERVAL`] minors each) give a stable block structure — the same
/// adaptive idea as AutoCAD's grid, which coarsens the drawn spacing as you zoom
/// out instead of emitting an unreadable wall of lines. Rounding straight to a
/// power of ten can jump from ~30 to ~60 divisions for a small change in extent;
/// snapping to the 1/2/5 sequence keeps the count in that band. A non-finite or
/// degenerate extent falls back to `1.0`; the caller's count caps bound the
/// result regardless.
fn grid_step(extent: f64) -> f64 {
    if !extent.is_finite() || extent <= 0.0 {
        return 1.0;
    }
    let target = extent / GRID_TARGET_MINOR_DIVISIONS;
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

/// Target count of **minor** divisions across the larger view extent.
///
/// With the default [`GRID_MAJOR_INTERVAL`] of `5` this gives roughly 6–12 major
/// blocks across the view, so the hierarchy is visible without the minors
/// becoming an unreadable wall.
const GRID_TARGET_MINOR_DIVISIONS: f64 = 60.0;

/// The minor multiples of `step` inside `[min, max]`, excluding the major
/// positions (indices that are multiples of `interval`).
///
/// `grid_positions` already bounds the count; filtering afterwards can only
/// shrink it. An index is recovered by `position / step` (every position is an
/// exact integer multiple of `step`), so a major coincidence is detected
/// without a tolerance that would fail at large pan offsets.
fn minor_grid_positions(min: f64, max: f64, step: f64, interval: u32, cap: usize) -> Vec<f64> {
    if interval <= 1 {
        // `GRIDMAJOR = 1` means every line is major; there is no minor level.
        return Vec::new();
    }
    grid_positions(min, max, step, cap)
        .into_iter()
        .filter(|position| !is_major_grid_position(*position, step, interval))
        .collect()
}

/// Whether `position` sits on a major line for the given minor `step` and
/// `interval`.
///
/// Every generated position is `index * step`, so the index is recovered by a
/// rounded division; `i64` remainder then decides exactly. Positions beyond the
/// exact-integer range are conservatively treated as minor (they are far outside
/// any view the caps can produce).
fn is_major_grid_position(position: f64, step: f64, interval: u32) -> bool {
    if interval <= 1 {
        return true;
    }
    if !position.is_finite() || !step.is_finite() || step <= 0.0 {
        return false;
    }
    let index = (position / step).round();
    if !index.is_finite() || index.abs() > 9.0e15 {
        return false;
    }
    (index as i64).rem_euclid(i64::from(interval)) == 0
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
