//! HATCH fill geometry (spec §3.2): polygon triangulation and pattern lines.
//!
//! Imported hatches are reduced to closed loops in hatch-plane 2D coordinates;
//! this module turns them into triangle indices (solid fill) or pattern
//! polylines (line patterns) using plain even-odd scanline clipping. It is
//! deliberately free of any DWG types so it can be unit-tested directly.
//!
//! The implementation is split across submodules: `common` (shared helpers),
//! `simplify` (boundary reduction), `fill` (solid triangulation), `pattern`
//! (line patterns) and `gradient` (gradient colouring). This module re-exports
//! the public surface as `cad_domain::hatch::...`; it lives in `cad-domain`
//! because the stored display derivation sits with `SemanticGeometry::Hatch`.

mod common;
mod fill;
mod gradient;
mod pattern;
mod simplify;

/// A closed polygon in hatch-plane coordinates.
pub type Loop = Vec<[f64; 2]>;

pub use fill::{fill_rings, triangulate, FillError, FillMesh, MAX_FILL_POINTS, MAX_FILL_TRIANGLES};
pub use gradient::{
    gradient_vertex_colors, normalize_stops, sample_gradient, GradientDef, GradientKind,
    GradientStop,
};
pub use pattern::{pattern_polylines, PatternLine};
pub use simplify::simplify;

#[cfg(test)]
mod tests;
