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
