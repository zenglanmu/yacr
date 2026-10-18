//! Model/paper space selection and paper-viewport transforms (spec §3.3, F04).
//!
//! Model space is the primary path. A drawing may also contain *paper space*
//! layouts; each layout owns paper-space geometry plus a list of viewports that
//! look into model space. This module is the pure, host-independent logic for
//! choosing a space and for turning a viewport into a concrete paper-space
//! view matrix, with an explicit refusal for viewport states this build cannot
//! draw correctly.
//!
//! ## Why this is its own step (audit B22)
//!
//! A [`PaperViewport`] stores a clip definition plus one `Transform3` and a
//! completeness verdict. From those this module reconstructs a **real
//! four-corner paper rectangle** and a **view transform with a view centre**
//! (the model point shown at the rectangle centre), then applies that transform
//! to model geometry. Anything whose state cannot be reconstructed exactly is
//! reported as [`ViewportState::Unsupported`] with a stable, machine-readable
//! reason code rather than drawn at a guessed ratio.
//!
//! ## Clip encodings
//!
//! Two encodings are accepted; both resolve to the same four paper corners:
//!
//! 1. **Four-corner clip (preferred)**: `clip = [c0, c1, c2, c3]` are the four
//!    paper-space corners of the viewport. The transform is the view transform;
//!    its view centre is recovered from the geometry. The rectangle must be
//!    axis-aligned in paper space: a rotated/twisted clip is refused instead of
//!    being silently squared off.
//! 2. **Three-point legacy clip**: `clip = [a, b, view_center]`, where `a` and
//!    `b` are *opposite* paper corners and the third point is the model view
//!    centre. This is the shape the current importer writes (with adjacent
//!    corners, which is refused). Kept for callers that already build it so the
//!    representation API stays source-compatible. A legacy clip whose transform
//!    also carries a non-zero translation is ambiguous and is refused: the view
//!    centre belongs in `clip[2]`.
//!
//! ## Coordinate convention
//!
//! The stored `model_to_paper` is interpreted in the direction this codebase
//! actually writes it: a **paper → model** transform whose linear scale is
//! *model units per paper unit* (`view_height / paper_height`, the audit's B22
//! direction). The applied **model → paper** map is therefore its inverse, so
//! `paper_per_model = paper_height / view_height` (see
//! [`paper_per_model_from_view`]). A viewport states
//!
//! ```text
//! paper = (model - view_center) * paper_per_model + paper_center
//! ```
//!
//! and its four paper corners are clipped exactly with Liang–Barsky. Line
//! geometry clips per segment, mesh geometry clips per triangle
//! (Sutherland–Hodgman, with colour/normal interpolation) and an image quad
//! clips with its texture coordinates interpolated — see
//! [`clip_mesh_to_rect`] and [`clip_image_quad_to_rect`]. A view transform that
//! is twisted (rotation), sheared, mirrored, non-uniform, off the paper plane or
//! singular is unsupported here and reported, never applied as an approximation.
//!
//! Paper-space measurement is wired in `cad-measure` through
//! [`ViewportTransform::paper_to_model`]; this module only provides the single
//! verified source for that inverse.

mod build;
mod clip;
mod image;
mod selection;
mod view;

pub use build::build_paper_space;
pub use clip::{clip_mesh_to_rect, clip_polygon_to_rect, clip_polyline_to_rect};
pub use image::clip_image_quad_to_rect;
pub use selection::{
    enumerate_layouts, ClipRefusal, LayoutDescriptor, SpaceSelection, ViewportState,
    ViewportUnsupported,
};
pub use view::{paper_per_model_from_view, viewport_transform, ViewportTransform};

/// Stable reason codes attached to [`ViewportState::Unsupported`].
///
/// They are part of the contract: callers may branch on the code, and the
/// human-readable message may change without breaking them.
pub mod viewport_reason {
    /// The importer reported a complex / non-rectangular clip.
    pub const COMPLEX_CLIP: &str = "viewport.complex_clip";
    /// The clip point count is neither three (legacy) nor four (rectangle).
    pub const CLIP_ARITY: &str = "viewport.clip_arity";
    /// A clip or transform coordinate is not finite.
    pub const NON_FINITE: &str = "viewport.non_finite";
    /// The four paper corners do not form an axis-aligned rectangle.
    pub const ROTATED_CLIP: &str = "viewport.rotated_clip";
    /// The paper rectangle has zero width or height.
    pub const DEGENERATE_RECT: &str = "viewport.degenerate_rect";
    /// A legacy three-point clip stored adjacent (non-opposite) corners.
    pub const ADJACENT_CORNERS: &str = "viewport.adjacent_corners";
    /// The stored scale is missing, zero or negative.
    pub const SCALE: &str = "viewport.scale";
    /// The view transform carries a twist (in-plane rotation).
    pub const TWISTED_TRANSFORM: &str = "viewport.twisted_transform";
    /// The view transform is sheared or non-uniform.
    pub const NON_UNIFORM: &str = "viewport.non_uniform";
    /// The view transform mirrors the view.
    pub const MIRROR: &str = "viewport.mirror";
    /// The view direction is not perpendicular to the paper plane.
    pub const OFF_PLANE_VIEW: &str = "viewport.off_plane_view";
    /// The viewport uses a perspective projection.
    pub const PERSPECTIVE: &str = "viewport.perspective";
    /// A legacy three-point clip also carried a non-zero transform translation.
    pub const TRANSLATION_MISMATCH: &str = "viewport.translation_mismatch";
}

/// Stable reason codes attached to a per-primitive viewport-clip degradation.
///
/// A code means the primitive could **not** be clipped exactly and the original
/// (unclipped) geometry was kept and reported — never silently dropped, and
/// never claimed as clipped.
pub mod clip_reason {
    /// Mesh geometry could not be clipped exactly (missing index or non-finite).
    pub const MESH_UNCLIPPABLE: &str = "viewport.clip_mesh_unclippable";
    /// Unshaped text has no glyph outline until a font shapes it.
    pub const TEXT_FONT_DEPENDENT: &str = "viewport.clip_text_font_dependent";
    /// The image quad maps to a non-finite position and cannot be clipped.
    pub const IMAGE_DEGENERATE: &str = "viewport.clip_image_degenerate";
    /// An entity clip and a viewport cut both apply, but the entity clip is not
    /// convex so the two could not be intersected exactly. The entity clip is
    /// kept and the viewport cut is not applied on top of it.
    pub const IMAGE_INTERSECTION_UNSUPPORTED: &str = "viewport.clip_image_intersection_unsupported";
    /// A primitive with no clip representation reached the viewport (instance).
    pub const UNSUPPORTED_PRIMITIVE: &str = "viewport.clip_unsupported_primitive";
}

#[cfg(test)]
mod tests;
