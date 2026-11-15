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
//! completeness verdict. From those this module reconstructs a **real paper clip
//! boundary** ([`PaperClip`]) and a **view transform with a view centre** (the
//! model point shown at the clip envelope's centre), then applies that transform
//! to model geometry. Anything whose state cannot be reconstructed exactly is
//! reported as [`ViewportState::Unsupported`] with a stable, machine-readable
//! reason code rather than drawn at a guessed ratio.
//!
//! ## Clip encodings
//!
//! Three encodings are accepted; the order below is the disambiguation rule:
//!
//! 1. **Three-point legacy clip** (`clip = [a, b, view_center]`, and only with a
//!    translation-free transform): `a` and `b` are *opposite* paper corners and
//!    the third point is the **model** view centre. The third point is a model
//!    point, so the shape only means anything when the transform carries no
//!    translation; that is the test that tells it apart from a triangle clip.
//! 2. **Axis-aligned four-corner rectangle**: `clip = [c0, c1, c2, c3]` are the
//!    four paper corners in rectangle order. This keeps the dedicated half-plane
//!    clippers and therefore exactly the geometry and diagnostics it always had.
//! 3. **A general simple polygon** with at least three points — what polygonal
//!    `MVIEW`/`VPCLIP` write, and what a rotated rectangle or any imported
//!    non-rectangular boundary resolves to. It is validated as simple
//!    (non self-intersecting), planar and non-degenerate; a boundary that fails
//!    is refused by name ([`viewport_reason::POLYGON_SELF_INTERSECTING`],
//!    [`viewport_reason::POLYGON_NON_PLANAR`],
//!    [`viewport_reason::POLYGON_DEGENERATE`], [`viewport_reason::CLIP_ARITY`]).
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
//! ## Clipping each primitive
//!
//! An axis-aligned rectangle clips with half-planes: lines per segment
//! (Liang–Barsky), meshes per triangle (Sutherland–Hodgman with colour/normal
//! interpolation), images with their texture coordinates interpolated, and
//! analytic conics to exact parameter intervals — see [`clip_mesh_to_rect`],
//! [`clip_image_quad_to_rect`] and [`clip_conic_to_rect`].
//!
//! Any other simple polygon — concave included — clips with crossings: a line
//! segment is split at every boundary crossing and each span is kept by the
//! even-odd point-in-polygon rule, a conic's exact boundary parameters are solved
//! in closed form, and filled geometry is intersected with the clip's convex
//! triangulation (so `subject ∩ clip` is exact, never its bounding box). See
//! [`clip_polyline_to_polygon`], [`clip_conic_to_polygon`],
//! [`clip_mesh_to_polygon`] and [`clip_polygon_to_polygon`].
//!
//! A view transform that is twisted (rotation), sheared, mirrored, non-uniform,
//! off the paper plane or singular is unsupported here and reported, never
//! applied as an approximation.
//!
//! Paper-space measurement is wired in `cad-measure` through
//! [`ViewportTransform::paper_to_model`]; this module only provides the single
//! verified source for that inverse.

mod build;
mod clip;
mod clip_polygon;
mod image;
mod polygon;
mod selection;
mod view;

pub use build::build_paper_space;
pub use clip::{
    clip_conic_to_rect, clip_mesh_to_polygon, clip_mesh_to_rect, clip_polygon_to_rect,
    clip_polyline_to_rect,
};
pub use clip_polygon::{
    clip_conic, clip_conic_to_polygon, clip_filled_polygon, clip_mesh, clip_polygon_to_polygon,
    clip_polyline, clip_polyline_to_polygon,
};
pub use image::clip_image_quad_to_rect;
pub(crate) use image::{clip_image_polygon_to_rect, image_polygon_is_convex};
pub use polygon::PaperClip;
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
    /// The clip has fewer than three points, so it is not a polygon.
    pub const CLIP_ARITY: &str = "viewport.clip_arity";
    /// A clip or transform coordinate is not finite.
    pub const NON_FINITE: &str = "viewport.non_finite";
    /// Historical: the four paper corners did not form an axis-aligned
    /// rectangle. A rotated rectangle is a simple polygon and is drawn now, so
    /// this code is **no longer produced**; it is retained so external matchers
    /// keep compiling.
    pub const ROTATED_CLIP: &str = "viewport.rotated_clip";
    /// The paper rectangle has zero width or height.
    pub const DEGENERATE_RECT: &str = "viewport.degenerate_rect";
    /// A legacy three-point clip stored adjacent (non-opposite) corners.
    pub const ADJACENT_CORNERS: &str = "viewport.adjacent_corners";
    /// The clip polygon has zero area or a repeated vertex.
    pub const POLYGON_DEGENERATE: &str = "viewport.polygon_degenerate";
    /// The clip polygon's edges cross or touch a non-adjacent edge.
    pub const POLYGON_SELF_INTERSECTING: &str = "viewport.polygon_self_intersecting";
    /// The clip polygon does not lie on one paper plane.
    pub const POLYGON_NON_PLANAR: &str = "viewport.polygon_non_planar";
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
    /// Historical: a legacy three-point clip also carried a non-zero transform
    /// translation. Such a clip is a triangle viewport now (the third point is a
    /// paper vertex, not a model anchor), so this code is **no longer produced**;
    /// it is retained so external matchers keep compiling.
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
    /// An analytic conic has non-finite parameters and cannot be clipped.
    pub const CONIC_UNCLIPPABLE: &str = "viewport.clip_conic_unclippable";
}

#[cfg(test)]
mod tests;
