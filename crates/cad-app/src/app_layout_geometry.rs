//! Pure viewport/page-setup geometry helpers for the layout batch.
//!
//! Split out of `app_layout` so that module stays inside the 1000-line code
//! gate. Nothing here touches application state: the functions only build or
//! resolve values the command handlers then validate and write.

use super::*;
use cad_db::{DrawingDatabase, PaperViewport};

/// The centre of a viewport clip's axis-aligned paper rectangle.
pub(super) fn clip_center(clip: &[Point3]) -> Option<Point3> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in clip {
        if !point.x.is_finite() || !point.y.is_finite() {
            return None;
        }
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return None;
    }
    Some(Point3 {
        x: (min_x + max_x) * 0.5,
        y: (min_y + max_y) * 0.5,
        z: 0.0,
    })
}

/// Validate one **polygonal** paper-space clip boundary.
///
/// `prefix` is the stable diagnostic namespace of the caller (`vpclip` or
/// `mview`), so both commands refuse with the same shapes and their own code:
///
/// * `{prefix}.degenerate_polygon` — fewer than three points, a non-finite
///   coordinate, a repeated vertex, or a zero-area (collinear) boundary;
/// * `{prefix}.non_coplanar` — the points do not lie on one paper plane (the
///   clip is a 2D paper boundary, so a z spread cannot be honoured);
/// * `{prefix}.self_intersecting` — two non-adjacent edges touch or cross.
///
/// Tolerances are relative to the polygon's own extent so a millimetre clip and
/// a metre clip behave the same. Nothing here mutates: the caller validates
/// first and writes through `set_layout_viewports` afterwards.
pub(super) fn validate_clip_polygon(points: &[Point3], prefix: &str) -> CadResult<()> {
    if points.len() < 3 {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.degenerate_polygon: a polygonal clip needs at least three points; got {}",
            points.len()
        )));
    }
    let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for point in points {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "{prefix}.degenerate_polygon: a polygonal clip point is not finite"
            )));
        }
        for (axis, value) in [point.x, point.y, point.z].into_iter().enumerate() {
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    let extent = (max[0] - min[0]).max(max[1] - min[1]).max(1.0);
    let tolerance = 1e-9 * extent;
    if max[2] - min[2] > tolerance {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.non_coplanar: the polygonal clip spans z {}..{}; a paper clip boundary must \
             lie on one plane",
            min[2], max[2]
        )));
    }
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        let (a, b) = (points[index], points[next]);
        if (a.x - b.x).abs() <= tolerance && (a.y - b.y).abs() <= tolerance {
            return Err(CadError::InvalidInput(format!(
                "{prefix}.degenerate_polygon: clip vertices {index} and {next} coincide"
            )));
        }
    }
    let count = points.len();
    for first in 0..count {
        let first_next = (first + 1) % count;
        for second in (first + 1)..count {
            let second_next = (second + 1) % count;
            // Adjacent edges share a vertex by construction; only an edge that
            // touches another edge it does not share a vertex with is a
            // self-intersection.
            if second == first_next || second_next == first {
                continue;
            }
            if segments_intersect(
                points[first],
                points[first_next],
                points[second],
                points[second_next],
                tolerance,
            ) {
                return Err(CadError::InvalidInput(format!(
                    "{prefix}.self_intersecting: clip edges {first}-{first_next} and \
                     {second}-{second_next} cross or touch"
                )));
            }
        }
    }
    // A symmetric bow-tie has zero signed area; it is reported as a
    // self-intersection above, so only collinear/degenerate boundaries reach
    // this check.
    let signed_area = polygon_signed_area(points);
    if signed_area.abs() <= tolerance * extent {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.degenerate_polygon: the polygonal clip has zero area (collinear vertices)"
        )));
    }
    Ok(())
}

/// Twice the signed area of the polygon projected onto the paper plane.
fn polygon_signed_area(points: &[Point3]) -> f64 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

/// Whether two 2D segments touch or cross, within `tolerance`.
///
/// Collinear overlap and endpoint touching count as an intersection: a polygon
/// boundary that merely grazes itself is not a simple polygon.
fn segments_intersect(a1: Point3, a2: Point3, b1: Point3, b2: Point3, tolerance: f64) -> bool {
    let cross = |origin: Point3, first: Point3, second: Point3| {
        (first.x - origin.x) * (second.y - origin.y) - (first.y - origin.y) * (second.x - origin.x)
    };
    let within = |value: f64, limit: f64| value.abs() <= limit;
    let d1 = cross(b1, b2, a1);
    let d2 = cross(b1, b2, a2);
    let d3 = cross(a1, a2, b1);
    let d4 = cross(a1, a2, b2);
    let extent = ((b2.x - b1.x).abs())
        .max((b2.y - b1.y).abs())
        .max((a2.x - a1.x).abs())
        .max((a2.y - a1.y).abs())
        .max(1.0);
    let limit = tolerance * extent;
    let opposite = |first: f64, second: f64| {
        (!within(first, limit) && !within(second, limit)) && first.signum() != second.signum()
    };
    if opposite(d1, d2) && opposite(d3, d4) {
        return true;
    }
    // Collinear/touching cases: a vertex of one segment lies on the other.
    let on_segment = |point: Point3, start: Point3, end: Point3, collinear: f64| {
        within(collinear, limit)
            && point.x >= start.x.min(end.x) - limit
            && point.x <= start.x.max(end.x) + limit
            && point.y >= start.y.min(end.y) - limit
            && point.y <= start.y.max(end.y) + limit
    };
    on_segment(a1, b1, b2, d1)
        || on_segment(a2, b1, b2, d2)
        || on_segment(b1, a1, a2, d3)
        || on_segment(b2, a1, a2, d4)
}

/// Build a `rows x cols` grid of axis-aligned viewports covering the sheet.
///
/// The cells are equal, cover the sheet from its own origin, and each clip is the
/// four paper corners `(minx,miny)`, `(maxx,miny)`, `(maxx,maxy)`, `(minx,maxy)`.
/// The stored transform follows the importer's paper→model convention with
/// `model_per_paper = 1.0` and translation `anchor - paper_center`: the model
/// **origin** is the anchor, so `model_to_paper` maps model origin to each cell
/// centre. This is a declared simple layout, not an AutoCAD Standard
/// configuration.
pub(super) fn grid_viewports(
    drawing: &DrawingDatabase,
    layout_id: LayoutId,
    rows: u32,
    cols: u32,
    total: u32,
) -> Vec<PaperViewport> {
    let (width, height) = {
        let (width, height) = drawing.plot_settings_for(layout_id).rotated_size();
        if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
            (width, height)
        } else {
            (210.0, 297.0)
        }
    };
    let mut viewports = Vec::with_capacity(total as usize);
    for row in 0..rows {
        for col in 0..cols {
            let min_x = width * f64::from(col) / f64::from(cols);
            let max_x = width * f64::from(col + 1) / f64::from(cols);
            let min_y = height * f64::from(row) / f64::from(rows);
            let max_y = height * f64::from(row + 1) / f64::from(rows);
            let paper_center = Point3 {
                x: (min_x + max_x) * 0.5,
                y: (min_y + max_y) * 0.5,
                z: 0.0,
            };
            // The view centre is the model origin; the stored paper→model
            // translation is `anchor - paper_center` with a unit scale.
            let mut matrix = Transform3::identity().matrix;
            matrix[0][0] = 1.0;
            matrix[1][1] = 1.0;
            matrix[0][3] = -paper_center.x;
            matrix[1][3] = -paper_center.y;
            viewports.push(PaperViewport {
                clip: vec![
                    Point3 {
                        x: min_x,
                        y: min_y,
                        z: 0.0,
                    },
                    Point3 {
                        x: max_x,
                        y: min_y,
                        z: 0.0,
                    },
                    Point3 {
                        x: max_x,
                        y: max_y,
                        z: 0.0,
                    },
                    Point3 {
                        x: min_x,
                        y: max_y,
                        z: 0.0,
                    },
                ],
                model_to_paper: Transform3 { matrix },
                completeness: Completeness::Complete,
                layer_overrides: std::collections::BTreeMap::new(),
            });
        }
    }
    viewports
}

/// Resolve a `VPSCALE` argument against the drawing's scale table.
///
/// A ratio (`a:b`, both finite, non-zero) is accepted even when the drawing does
/// not list it; a bare word must resolve through the scale table. Zero and
/// non-finite values are refused before the table is consulted.
pub(super) fn resolve_viewport_scale(
    drawing: &DrawingDatabase,
    name: &str,
) -> CadResult<(String, f64)> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CadError::InvalidInput(
            "vpscale.invalid_scale: VPSCALE needs a scale name or ratio".into(),
        ));
    }
    if let Some((paper, drawing_units)) = name.split_once(':') {
        let paper = paper.trim().parse::<f64>().map_err(|_| {
            CadError::InvalidInput(format!("vpscale.invalid_scale: {:?} is not a ratio", name))
        })?;
        let drawing_units = drawing_units.trim().parse::<f64>().map_err(|_| {
            CadError::InvalidInput(format!("vpscale.invalid_scale: {:?} is not a ratio", name))
        })?;
        if !paper.is_finite() || !drawing_units.is_finite() {
            return Err(CadError::InvalidInput(
                "vpscale.non_finite_scale: VPSCALE needs finite ratio components".into(),
            ));
        }
        if paper == 0.0 || drawing_units == 0.0 {
            return Err(CadError::InvalidInput(
                "vpscale.zero_scale: VPSCALE ratio components must be non-zero".into(),
            ));
        }
        if paper < 0.0 || drawing_units < 0.0 {
            return Err(CadError::InvalidInput(
                "vpscale.invalid_scale: VPSCALE ratio components must be positive".into(),
            ));
        }
        // Prefer the table's named factor when the drawing lists the same scale;
        // otherwise the parsed ratio is the standard scale the caller asked for.
        if let Some(scale) = drawing.scale_by_name(name) {
            return Ok((scale.name.clone(), scale.factor()));
        }
        return Ok((name.to_string(), paper / drawing_units));
    }
    if let Ok(value) = name.parse::<f64>() {
        if !value.is_finite() {
            return Err(CadError::InvalidInput(
                "vpscale.non_finite_scale: VPSCALE needs a finite scale".into(),
            ));
        }
        if value == 0.0 {
            return Err(CadError::InvalidInput(
                "vpscale.zero_scale: VPSCALE needs a non-zero scale".into(),
            ));
        }
        if value < 0.0 {
            return Err(CadError::InvalidInput(
                "vpscale.invalid_scale: VPSCALE needs a positive scale".into(),
            ));
        }
    }
    drawing
        .scale_by_name(name)
        .map(|scale| (scale.name.clone(), scale.factor()))
        .ok_or_else(|| {
            CadError::InvalidInput(format!(
                "vpscale.unknown_scale: {:?} is not in the drawing's scale table and is not a \
                 finite ratio",
                name
            ))
        })
}
