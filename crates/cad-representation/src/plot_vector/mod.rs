//! Vector plot export: paper-space display geometry → path document → SVG/PDF.
//!
//! This module is the vector counterpart of the raster `plot` path. It takes the
//! same CPU [`DisplayRepresentation`] the renderer builds (for a paper-space
//! layout, that is [`build_paper_space`], which has already applied every
//! viewport's model→paper transform and expanded INSERTs) plus the [`PlotPage`]
//! the pure planner produced, and turns it into a small, self-contained vector
//! document that can be serialised as SVG or a minimal single-page PDF.
//!
//! ## No GPU
//!
//! Every function here is pure CPU: no wgpu device, no window, no surface. The
//! SVG/PDF writers are hand-written string/byte builders (no third-party
//! crates), so a caller can exercise the whole path in a unit test without a
//! software Vulkan adapter.
//!
//! ## Coordinate system
//!
//! The document coordinate system is **page millimetres, origin at the
//! bottom-left, y pointing up** — the CAD paper convention. [`VectorPage`]
//! derives it from a [`PlotPage`] by undoing the planner's pixel scale and y
//! flip, so the physical size is DPI-independent up to the plan's own pixel
//! rounding. SVG uses y-down user units, so the SVG writer flips y about the
//! document height; PDF user space is already y-up, so the PDF writer emits
//! millimetres directly behind a `72/25.4` scale matrix.
//!
//! ## Honesty
//!
//! A primitive that cannot be expressed as paths — an unshaped
//! [`DisplayPrimitive::Text`], an [`DisplayPrimitive::Image`], an unexpanded
//! [`DisplayPrimitive::Instance`], or a mesh with no triangles — is **never
//! silently dropped**. Each case adds a stable diagnostic and downgrades the
//! document's [`Completeness`] so the caller can report the gap. A degenerate
//! or non-finite polyline is reported the same way.

mod collect;
mod entry;
mod format;
mod model;
mod page;
mod pdf;
mod svg;

pub use collect::build_vector_document;
pub use entry::{render_plot_pdf, render_plot_svg};
pub use model::{VectorDocument, VectorPath};
pub use page::VectorPage;
pub use pdf::render_pdf;
pub use svg::render_svg;

/// Stable diagnostic codes attached to [`VectorDocument::diagnostics`].
///
/// They are part of the contract: callers may branch on the code and the human
/// message may change without breaking them.
pub mod vector_diagnostic {
    /// A polyline had fewer than two usable points.
    pub const DEGENERATE_POLYLINE: &str = "vector.degenerate_polyline";
    /// A mapped coordinate was not finite and could not be written.
    pub const NON_FINITE: &str = "vector.non_finite";
    /// A mesh carried no triangles.
    pub const MESH_WITHOUT_TRIANGLES: &str = "vector.mesh_without_triangles";
    /// A mesh triangle referenced a vertex that does not exist.
    pub const MESH_BAD_INDEX: &str = "vector.mesh_bad_index";
    /// A mesh had per-vertex colours the path model cannot carry.
    pub const MESH_VERTEX_COLORS_IGNORED: &str = "vector.mesh_vertex_colors_ignored";
    /// A text primitive was not shaped into outlines (no font available).
    pub const TEXT_UNSHAPED: &str = "vector.text_unshaped";
    /// An image primitive cannot be represented as paths here.
    pub const IMAGE_UNSUPPORTED: &str = "vector.image_unsupported";
    /// An INSERT instance reached the vector stage unexpanded.
    pub const UNEXPANDED_INSTANCE: &str = "vector.unexpanded_instance";
    /// A WIPEOUT mask is deliberately skipped: vector output has no background
    /// flood, and ADR 0022 D4 does not cover masks in plot output. Never silent.
    pub const MASK_UNSUPPORTED: &str = "vector.mask_unsupported";
}

#[cfg(test)]
mod tests;
