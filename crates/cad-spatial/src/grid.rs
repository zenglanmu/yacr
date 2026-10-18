//! Shared uniform X/Y grid arithmetic (ADR-0012 §7: "复用/扩展
//! `GridSpatialIndex` 的网格算术").
//!
//! Both the standalone [`crate::GridSpatialIndex`] and the pick broad phase's
//! two-level [`crate::ChunkGridIndex`] derive a uniform X/Y grid over the
//! drawing extent and map a point to a cell. This module owns that arithmetic —
//! the extent/aspect sizing, the `(col, row)` mapping and the cell origin — so
//! the two structures cannot drift apart.
//!
//! Each caller keeps its **own sizing inputs** (target cell count, degenerate
//! extent/cell epsilon, per-dimension cap): the pick grid is sized coarsely from
//! a target entries-per-chunk while `GridSpatialIndex` uses a fixed fine cap, and
//! that difference is measured and load-bearing (see `chunk.rs`). Only the
//! arithmetic is shared, not the policy.

use cad_domain::Point3;

/// Degenerate-extent floor shared by both grids: keeps the cell finite and
/// positive for a single-point drawing extent.
pub(crate) const MIN_CELL_EXTENT: f64 = 1e-9;

/// The derived layout of a uniform X/Y grid.
///
/// A degenerate/empty extent is represented by `cols == 0 && rows == 0`
/// ([`GridLayout::empty`]); [`GridLayout::cell_of`] must not be called then.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GridLayout {
    pub(crate) origin_x: f64,
    pub(crate) origin_y: f64,
    pub(crate) cell: f64,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
}

impl GridLayout {
    /// The layout of an empty grid (no cells). The cell extent is 1.0 only so
    /// the value is well-formed; no query runs against it.
    pub(crate) fn empty() -> Self {
        GridLayout {
            origin_x: 0.0,
            origin_y: 0.0,
            cell: 1.0,
            cols: 0,
            rows: 0,
        }
    }

    /// The cell `(col, row)` containing `(x, y)`, clamped to the grid.
    pub(crate) fn cell_of(&self, x: f64, y: f64) -> (usize, usize) {
        let cx =
            (((x - self.origin_x) / self.cell).floor() as isize).clamp(0, self.cols as isize - 1);
        let cy =
            (((y - self.origin_y) / self.cell).floor() as isize).clamp(0, self.rows as isize - 1);
        (cx as usize, cy as usize)
    }

    /// The lower-left `(x, y)` corner of cell `(col, row)`.
    pub(crate) fn cell_origin(&self, col: usize, row: usize) -> (f64, f64) {
        (
            self.origin_x + col as f64 * self.cell,
            self.origin_y + row as f64 * self.cell,
        )
    }

    /// The grid origin as a point (X/Y; Z is always 0 — the grid is planar).
    pub(crate) fn origin(&self) -> Point3 {
        Point3 {
            x: self.origin_x,
            y: self.origin_y,
            z: 0.0,
        }
    }

    /// The number of cells (`cols * rows`).
    pub(crate) fn cell_count(&self) -> usize {
        self.cols * self.rows
    }
}

/// The per-caller sizing inputs for [`layout`].
#[derive(Clone, Copy)]
pub(crate) struct GridSizing {
    /// Floor for the extent and aspect ratio (`GridSpatialIndex` uses a larger
    /// value than the pick grid; both keep their measured behaviour).
    pub(crate) extent_eps: f64,
    /// Floor for the derived cell extent.
    pub(crate) cell_eps: f64,
    /// Per-dimension cap on `cols`/`rows`.
    pub(crate) max_dim: usize,
}

/// Derive a uniform X/Y grid layout over the finite extent `min..max`.
///
/// The grid is as square as the extent aspect allows, with `cols * rows`
/// roughly `target_cells`; `target_cells` is clamped to at least 1.
pub(crate) fn layout(
    min: Point3,
    max: Point3,
    target_cells: f64,
    sizing: GridSizing,
) -> GridLayout {
    let ex = (max.x - min.x).max(sizing.extent_eps);
    let ey = (max.y - min.y).max(sizing.extent_eps);
    let target = target_cells.max(1.0);
    let aspect = (ex / ey).max(sizing.extent_eps);
    let cols = ((target * aspect).sqrt().ceil() as usize).clamp(1, sizing.max_dim);
    let rows = ((target / cols as f64).ceil() as usize).clamp(1, sizing.max_dim);
    let cell = (ex / cols as f64)
        .max(ey / rows as f64)
        .max(sizing.cell_eps);
    GridLayout {
        origin_x: min.x,
        origin_y: min.y,
        cell,
        cols,
        rows,
    }
}
