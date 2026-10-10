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

use crate::{
    DisplayFragment, DisplayPrimitive, DisplayRepresentation, ImageVertex, ProviderRegistry,
    RepresentationContext,
};
use cad_db::{DbEntity, DrawingDatabase, PaperViewport};
use cad_domain::*;

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

/// A precise, stable reason a primitive could not be clipped exactly.
///
/// The caller keeps the unclipped primitive and reports `Partial`; it must not
/// drop it silently and must not claim it was clipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipRefusal {
    /// Stable machine code, one of [`clip_reason`].
    pub reason: &'static str,
    /// Human-readable explanation. Never empty.
    pub message: String,
}

impl ClipRefusal {
    pub fn new(reason: &'static str, message: impl Into<String>) -> Self {
        ClipRefusal {
            reason,
            message: message.into(),
        }
    }
}

/// A user-facing choice of drawing space: model space or one paper-space layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceSelection {
    Model,
    Paper(LayoutId),
}

impl SpaceSelection {
    /// The layout a paper selection refers to, if any.
    pub fn layout(&self) -> Option<LayoutId> {
        match self {
            SpaceSelection::Model => None,
            SpaceSelection::Paper(id) => Some(*id),
        }
    }

    /// Whether this selection is model space.
    pub fn is_model(&self) -> bool {
        matches!(self, SpaceSelection::Model)
    }
}

/// A layout as exposed to a caller that wants to switch spaces.
///
/// `supported` is false when at least one of the layout's viewports cannot be
/// drawn correctly; `reason` then names the first blocking cause so the caller
/// can report it instead of showing a wrong or empty sheet. The reason is
/// prefixed with a stable [`viewport_reason`] code and then `: ` for
/// machine-readable branching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDescriptor {
    pub id: LayoutId,
    pub name: String,
    /// True when every viewport in the layout is drawable by this build.
    pub supported: bool,
    /// Empty when `supported`; otherwise `"<code>: <message>"`.
    pub reason: String,
    /// Number of viewports in the layout (0 for a paper sheet with no windows).
    pub viewport_count: usize,
}

/// Build the ordered layout list a space-switch affordance offers.
///
/// The order is the database's own (a `BTreeMap`, so deterministic by id).
/// Model space is *not* included: it is always available and is represented by
/// [`SpaceSelection::Model`]. A layout with no viewports is still `supported`
/// (it simply draws its own paper geometry).
pub fn enumerate_layouts(database: &DrawingDatabase) -> Vec<LayoutDescriptor> {
    database
        .layouts()
        .map(|layout| {
            let mut reason = String::new();
            let mut viewport_count = 0usize;
            for viewport in &layout.viewports {
                viewport_count += 1;
                if reason.is_empty() {
                    if let ViewportState::Unsupported(why) = viewport_transform(viewport) {
                        reason = why.to_string();
                    }
                }
            }
            LayoutDescriptor {
                id: layout.id,
                name: layout.name.clone(),
                supported: reason.is_empty(),
                reason,
                viewport_count,
            }
        })
        .collect()
}

/// A concrete, stable reason a viewport cannot be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportUnsupported {
    /// Stable machine code, one of [`viewport_reason`].
    pub code: &'static str,
    /// Human-readable explanation. Never empty.
    pub message: String,
}

impl ViewportUnsupported {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ViewportUnsupported {
            code,
            message: message.into(),
        }
    }

    /// The human-readable explanation (without the code prefix).
    pub fn reason(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ViewportUnsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// The result of interpreting one stored [`PaperViewport`].
#[derive(Debug, Clone, PartialEq)]
pub enum ViewportState {
    /// A drawable rectangular viewport with an exact transform. Boxed so the
    /// enum stays small; the unsupported reason is payload-light.
    Supported(Box<ViewportTransform>),
    /// The viewport cannot be drawn correctly by this build. Never drawn at a
    /// guessed ratio; the geometry is simply omitted and the reason reported.
    Unsupported(ViewportUnsupported),
}

/// Paper units per model unit for a viewport whose visible model height is
/// `view_height` and whose paper height is `paper_height`.
///
/// This is the direction audit B22 demands: the drawing scale `1:100` means one
/// paper unit is 100 model units, so `paper_per_model = paper_height /
/// view_height` (here `1/100`). The stored scalar is its reciprocal.
///
/// Returns `None` for a non-finite or non-positive input.
pub fn paper_per_model_from_view(view_height: f64, paper_height: f64) -> Option<f64> {
    if !view_height.is_finite() || view_height <= 0.0 {
        return None;
    }
    if !paper_height.is_finite() || paper_height <= 0.0 {
        return None;
    }
    Some(paper_height / view_height)
}

/// An exact model→paper mapping for one rectangular viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportTransform {
    /// The four paper-space corners, counter-clockwise from the minimum corner.
    pub paper_corners: [Point3; 4],
    /// Paper-space rectangle centre.
    pub paper_center: [f64; 2],
    /// Paper-space rectangle half-extent (both components strictly positive).
    pub paper_half: [f64; 2],
    /// Model-space point that maps to `paper_center` (the view centre).
    pub model_anchor: Point3,
    /// Paper units per model unit (strictly positive, uniform).
    pub paper_per_model: f64,
    /// Applied model→paper transform, including the view-centre translation.
    pub to_paper: Transform3,
    /// Exact inverse paper→model transform. The single source `cad-measure`
    /// uses for model measurement inside a viewport.
    pub to_model: Transform3,
}

impl ViewportTransform {
    /// Map a model-space point onto the paper sheet.
    ///
    /// The z component is carried through unchanged so 2D paper output keeps
    /// the viewport's plane; clipping uses only x/y.
    pub fn model_to_paper(&self, p: Point3) -> Point3 {
        self.to_paper.apply_point(p)
    }

    /// The inverse paper→model mapping.
    pub fn paper_to_model(&self, p: Point3) -> Point3 {
        self.to_model.apply_point(p)
    }

    /// Express the mapping as a [`Transform3`] so existing primitive transforms
    /// can be reused (uniform scale + translation, z unchanged).
    pub fn as_transform3(&self) -> Transform3 {
        self.to_paper
    }

    /// The paper-space rectangle as `(min, max)`.
    pub fn paper_bounds(&self) -> ([f64; 2], [f64; 2]) {
        (
            [
                self.paper_center[0] - self.paper_half[0],
                self.paper_center[1] - self.paper_half[1],
            ],
            [
                self.paper_center[0] + self.paper_half[0],
                self.paper_center[1] + self.paper_half[1],
            ],
        )
    }
}

fn finite_point(p: &Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// An axis-aligned paper rectangle recovered from a viewport clip.
struct PaperRect {
    corners: [Point3; 4],
    center: [f64; 2],
    half: [f64; 2],
}

fn ordered_rect(min: [f64; 2], max: [f64; 2]) -> [Point3; 4] {
    [
        Point3 {
            x: min[0],
            y: min[1],
            z: 0.0,
        },
        Point3 {
            x: max[0],
            y: min[1],
            z: 0.0,
        },
        Point3 {
            x: max[0],
            y: max[1],
            z: 0.0,
        },
        Point3 {
            x: min[0],
            y: max[1],
            z: 0.0,
        },
    ]
}

fn rect_from_min_max(min: [f64; 2], max: [f64; 2]) -> PaperRect {
    let width = max[0] - min[0];
    let height = max[1] - min[1];
    PaperRect {
        corners: ordered_rect(min, max),
        center: [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5],
        half: [width * 0.5, height * 0.5],
    }
}

/// Interpret a clip as a real four- or three-point paper rectangle.
///
/// The returned `Option<Point3>` is `Some` for the legacy three-point form,
/// where the third point is the explicit model view centre.
fn parse_paper_clip(clip: &[Point3]) -> Result<(PaperRect, Option<Point3>), ViewportUnsupported> {
    match clip.len() {
        4 => Ok((parse_four_corner_clip(clip)?, None)),
        3 => {
            let a = clip[0];
            let b = clip[1];
            let anchor = clip[2];
            if !finite_point(&a) || !finite_point(&b) || !finite_point(&anchor) {
                return Err(ViewportUnsupported::new(
                    viewport_reason::NON_FINITE,
                    "viewport clip has non-finite coordinates",
                ));
            }
            Ok((parse_opposite_corners(a, b)?, Some(anchor)))
        }
        n => Err(ViewportUnsupported::new(
            viewport_reason::CLIP_ARITY,
            format!(
                "viewport clip must be four rectangle corners (or three legacy points); found {n}"
            ),
        )),
    }
}

/// Recover a rectangle from two *opposite* corners (legacy three-point form).
fn parse_opposite_corners(a: Point3, b: Point3) -> Result<PaperRect, ViewportUnsupported> {
    let width = (b.x - a.x).abs();
    let height = (b.y - a.y).abs();
    if width <= 1e-12 && height <= 1e-12 {
        return Err(ViewportUnsupported::new(
            viewport_reason::DEGENERATE_RECT,
            "viewport clip corners coincide; the paper rectangle is degenerate",
        ));
    }
    if width <= 1e-12 || height <= 1e-12 {
        return Err(ViewportUnsupported::new(
            viewport_reason::ADJACENT_CORNERS,
            format!(
                "viewport clip corners are adjacent, not opposite (width {width}, height {height}); \
                 the missing paper dimension is not recoverable from a three-point clip"
            ),
        ));
    }
    let min = [a.x.min(b.x), a.y.min(b.y)];
    let max = [a.x.max(b.x), a.y.max(b.y)];
    Ok(rect_from_min_max(min, max))
}

/// Validate that four points are exactly the corners of an axis-aligned
/// rectangle. A rotated or twisted clip is refused, never squared off.
fn parse_four_corner_clip(clip: &[Point3]) -> Result<PaperRect, ViewportUnsupported> {
    for p in clip {
        if !finite_point(p) {
            return Err(ViewportUnsupported::new(
                viewport_reason::NON_FINITE,
                "viewport clip has non-finite coordinates",
            ));
        }
    }
    let min_x = clip.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let max_x = clip.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let min_y = clip.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_y = clip.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    let width = max_x - min_x;
    let height = max_y - min_y;
    if width <= 1e-12 || height <= 1e-12 {
        return Err(ViewportUnsupported::new(
            viewport_reason::DEGENERATE_RECT,
            format!("viewport paper rectangle is degenerate (width {width}, height {height})"),
        ));
    }
    // Every corner must sit on the bounding box, and all four combinations must
    // be present; anything else is a rotated/twisted or duplicated clip.
    let tol = 1e-9 * width.max(height);
    let mut seen = [false; 4];
    for p in clip {
        let on_left = (p.x - min_x).abs() <= tol;
        let on_right = (p.x - max_x).abs() <= tol;
        let on_bottom = (p.y - min_y).abs() <= tol;
        let on_top = (p.y - max_y).abs() <= tol;
        if !(on_left || on_right) || !(on_bottom || on_top) {
            return Err(ViewportUnsupported::new(
                viewport_reason::ROTATED_CLIP,
                "viewport paper clip is rotated or twisted; only an axis-aligned rectangle is supported",
            ));
        }
        seen[usize::from(on_right) + 2 * usize::from(on_top)] = true;
    }
    if !seen.iter().all(|present| *present) {
        return Err(ViewportUnsupported::new(
            viewport_reason::ROTATED_CLIP,
            "viewport paper clip corners do not form a simple axis-aligned rectangle",
        ));
    }
    Ok(rect_from_min_max([min_x, min_y], [max_x, max_y]))
}

/// The view transform as stored: paper → model, model units per paper unit.
struct StoredView {
    model_per_paper: f64,
    translation: [f64; 3],
}

/// Validate the stored transform and extract its uniform scale + translation.
///
/// Only an axis-perpendicular, in-plane, positive uniform scale is drawable
/// here. Every other state gets a specific, stable reason.
fn parse_stored_view(transform: &Transform3) -> Result<StoredView, ViewportUnsupported> {
    let m = &transform.matrix;
    if !m.iter().flatten().all(|c| c.is_finite()) {
        return Err(ViewportUnsupported::new(
            viewport_reason::NON_FINITE,
            "viewport transform has non-finite coefficients",
        ));
    }
    let a = m[0][0];
    let b = m[1][1];
    let c = m[2][2];
    let magnitude = a.abs().max(b.abs()).max(c.abs()).max(1e-300);
    let tol = 1e-9 * magnitude;

    if a < 0.0 || b < 0.0 {
        return Err(ViewportUnsupported::new(
            viewport_reason::MIRROR,
            "viewport transform mirrors the view; mirrored viewports are not supported",
        ));
    }
    if a <= 0.0 || b <= 0.0 || c == 0.0 {
        return Err(ViewportUnsupported::new(
            viewport_reason::SCALE,
            format!("viewport scale must be finite, positive and non-zero; found diagonal ({a}, {b}, {c})"),
        ));
    }
    if m[0][1].abs() > tol || m[1][0].abs() > tol {
        return Err(ViewportUnsupported::new(
            viewport_reason::TWISTED_TRANSFORM,
            "viewport transform is rotated/twisted; only a paper-axis-aligned view is supported",
        ));
    }
    if m[0][2].abs() > tol || m[1][2].abs() > tol || m[2][0].abs() > tol || m[2][1].abs() > tol {
        return Err(ViewportUnsupported::new(
            viewport_reason::OFF_PLANE_VIEW,
            "viewport view direction is not perpendicular to the paper plane",
        ));
    }
    if (a - b).abs() > tol {
        return Err(ViewportUnsupported::new(
            viewport_reason::NON_UNIFORM,
            "viewport transform is non-uniform or sheared; only a uniform scale is supported",
        ));
    }
    Ok(StoredView {
        model_per_paper: a,
        translation: [m[0][3], m[1][3], m[2][3]],
    })
}

/// Inverse of a model→paper planar map `paper = s * model + t` (z unchanged).
fn invert_planar(to_paper: &Transform3, s: f64) -> Transform3 {
    let mut m = Transform3::identity().matrix;
    m[0][0] = 1.0 / s;
    m[1][1] = 1.0 / s;
    m[0][3] = -to_paper.matrix[0][3] / s;
    m[1][3] = -to_paper.matrix[1][3] / s;
    Transform3 { matrix: m }
}

/// Map an importer-supplied partial reason onto a stable [`viewport_reason`]
/// code, so an imported unsupported viewport reports the same machine-readable
/// code the representation layer uses for its own refusals. Unknown reasons
/// stay [`viewport_reason::COMPLEX_CLIP`] (the generic "not drawable" code).
fn classify_import_reason(reason: &str) -> &'static str {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("twist") {
        viewport_reason::TWISTED_TRANSFORM
    } else if lower.contains("perpendicular") || lower.contains("off_plane") {
        viewport_reason::OFF_PLANE_VIEW
    } else if lower.contains("perspective") {
        viewport_reason::PERSPECTIVE
    } else if lower.contains("view_height") || lower.contains("scale") {
        viewport_reason::SCALE
    } else if lower.contains("non-finite") || lower.contains("not finite") {
        viewport_reason::NON_FINITE
    } else {
        viewport_reason::COMPLEX_CLIP
    }
}

/// Interpret a stored viewport exactly, or refuse it with a stable reason.
///
/// See the module documentation for the accepted clip encodings and the
/// direction convention of `model_to_paper`. Everything else — a complex clip,
/// a clip with the wrong arity, a rotated paper clip, a degenerate rectangle,
/// adjacent legacy corners, a missing/zero/negative scale, a twisted, sheared,
/// mirrored, non-uniform or off-plane view, or non-finite data — is
/// [`ViewportState::Unsupported`].
pub fn viewport_transform(viewport: &PaperViewport) -> ViewportState {
    if let Completeness::Partial(reasons) = &viewport.completeness {
        let why = reasons
            .first()
            .cloned()
            .unwrap_or_else(|| "partial viewport state".to_string());
        return ViewportState::Unsupported(ViewportUnsupported::new(
            classify_import_reason(&why),
            format!("viewport is not fully supported: {why}"),
        ));
    }

    let (rect, legacy_anchor) = match parse_paper_clip(&viewport.clip) {
        Ok(parsed) => parsed,
        Err(why) => return ViewportState::Unsupported(why),
    };
    let stored = match parse_stored_view(&viewport.model_to_paper) {
        Ok(stored) => stored,
        Err(why) => return ViewportState::Unsupported(why),
    };

    let paper_per_model = 1.0 / stored.model_per_paper;
    let center = rect.center;

    let (model_anchor, to_paper, to_model) = match legacy_anchor {
        Some(anchor) => {
            // Legacy: the view centre lives in clip[2]; a transform translation
            // as well would make the two sources disagree, so refuse it.
            let tmax = stored
                .translation
                .iter()
                .fold(0.0f64, |acc, v| acc.max(v.abs()));
            if tmax > 1e-9 {
                return ViewportState::Unsupported(ViewportUnsupported::new(
                    viewport_reason::TRANSLATION_MISMATCH,
                    "legacy three-point viewport carries a transform translation as well as clip[2]; \
                     the view centre is ambiguous",
                ));
            }
            let mut matrix = Transform3::identity().matrix;
            matrix[0][0] = paper_per_model;
            matrix[1][1] = paper_per_model;
            matrix[0][3] = center[0] - anchor.x * paper_per_model;
            matrix[1][3] = center[1] - anchor.y * paper_per_model;
            let to_paper = Transform3 { matrix };
            let to_model = invert_planar(&to_paper, paper_per_model);
            (anchor, to_paper, to_model)
        }
        None => {
            // Four-corner clip: the stored paper→model transform positions the
            // view; the paper centre maps to the view centre.
            let model_per_paper = stored.model_per_paper;
            let tx = stored.translation[0];
            let ty = stored.translation[1];
            let mut model = Transform3::identity().matrix;
            model[0][0] = model_per_paper;
            model[1][1] = model_per_paper;
            model[0][3] = tx;
            model[1][3] = ty;
            let to_model = Transform3 { matrix: model };
            let mut paper = Transform3::identity().matrix;
            paper[0][0] = paper_per_model;
            paper[1][1] = paper_per_model;
            paper[0][3] = -tx * paper_per_model;
            paper[1][3] = -ty * paper_per_model;
            let to_paper = Transform3 { matrix: paper };
            let anchor = Point3 {
                x: model_per_paper * center[0] + tx,
                y: model_per_paper * center[1] + ty,
                z: 0.0,
            };
            (anchor, to_paper, to_model)
        }
    };

    ViewportState::Supported(Box::new(ViewportTransform {
        paper_corners: rect.corners,
        paper_center: rect.center,
        paper_half: rect.half,
        model_anchor,
        paper_per_model,
        to_paper,
        to_model,
    }))
}

/// Clip one transformed polyline to a paper rectangle.
///
/// `paper` are points already in paper coordinates; the rectangle is
/// `[center - half, center + half]`. Each segment is clipped independently with
/// Liang–Barsky, and runs of surviving segments are joined into polylines so a
/// curve crossing the window yields a continuous clipped run. A polyline fully
/// outside yields no pieces; a polyline inside is returned unchanged.
pub fn clip_polyline_to_rect(
    paper: &[Point3],
    center: [f64; 2],
    half: [f64; 2],
) -> Vec<Vec<Point3>> {
    let (min, max) = (
        [center[0] - half[0], center[1] - half[1]],
        [center[0] + half[0], center[1] + half[1]],
    );
    let mut runs: Vec<Vec<Point3>> = Vec::new();
    let mut current: Vec<Point3> = Vec::new();
    let push_point = |current: &mut Vec<Point3>, p: Point3| {
        if current.last().map(|last| *last == p).unwrap_or(false) {
            return;
        }
        current.push(p);
    };
    for segment in paper.windows(2) {
        match clip_segment(segment[0], segment[1], min, max) {
            Some((a, b)) => {
                push_point(&mut current, a);
                push_point(&mut current, b);
            }
            None => {
                if current.len() >= 2 {
                    runs.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
            }
        }
    }
    if current.len() >= 2 {
        runs.push(current);
    }
    runs
}

/// Liang–Barsky clip of one segment to an axis-aligned rectangle.
///
/// Returns `None` when the segment is fully outside, `Some((a, b))` with the
/// (possibly shortened) endpoints otherwise. A degenerate/zero-length segment
/// inside the rectangle is returned as an inside point.
fn clip_segment(a: Point3, b: Point3, min: [f64; 2], max: [f64; 2]) -> Option<(Point3, Point3)> {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    let checks = [
        (-dx, a.x - min[0]),
        (dx, max[0] - a.x),
        (-dy, a.y - min[1]),
        (dy, max[1] - a.y),
    ];
    for (p, q) in checks {
        if p == 0.0 {
            // Parallel to this edge: outside when already beyond it.
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                if r > t0 {
                    t0 = r;
                }
            } else {
                if r < t0 {
                    return None;
                }
                if r < t1 {
                    t1 = r;
                }
            }
        }
    }
    let lerp = |t: f64| Point3 {
        x: a.x + dx * t,
        y: a.y + dy * t,
        z: a.z + (b.z - a.z) * t,
    };
    Some((lerp(t0), lerp(t1)))
}

/// Sutherland–Hodgman clip of a convex polygon against an axis-aligned paper
/// rectangle `[center - half, center + half]`.
///
/// Returns the surviving convex polygon (empty when the input lies fully
/// outside). Consecutive duplicate vertices are removed so the result can be
/// fan-triangulated safely. The viewport windows this build accepts are exactly
/// axis-aligned rectangles (a non-rectangular clip is refused upstream in
/// [`viewport_transform`]), which is the convex window this routine handles.
pub fn clip_polygon_to_rect(
    polygon: &[[f64; 2]],
    center: [f64; 2],
    half: [f64; 2],
) -> Vec<[f64; 2]> {
    if polygon.is_empty() {
        return Vec::new();
    }
    let min = [center[0] - half[0], center[1] - half[1]];
    let max = [center[0] + half[0], center[1] + half[1]];
    let mut out = polygon.to_vec();
    out = clip_points_half_plane(out, 0, min[0], true);
    out = clip_points_half_plane(out, 0, max[0], false);
    out = clip_points_half_plane(out, 1, min[1], true);
    out = clip_points_half_plane(out, 1, max[1], false);
    dedupe_points(&mut out);
    out
}

fn clip_points_half_plane(
    input: Vec<[f64; 2]>,
    axis: usize,
    bound: f64,
    keep_above: bool,
) -> Vec<[f64; 2]> {
    if input.is_empty() {
        return Vec::new();
    }
    let inside = |p: [f64; 2]| {
        if keep_above {
            p[axis] >= bound
        } else {
            p[axis] <= bound
        }
    };
    let n = input.len();
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..n {
        let current = input[i];
        let previous = input[(i + n - 1) % n];
        let current_in = inside(current);
        let previous_in = inside(previous);
        if current_in {
            if !previous_in {
                out.push(intersect_axis(previous, current, axis, bound));
            }
            out.push(current);
        } else if previous_in {
            out.push(intersect_axis(previous, current, axis, bound));
        }
    }
    out
}

fn intersect_axis(a: [f64; 2], b: [f64; 2], axis: usize, bound: f64) -> [f64; 2] {
    let denom = b[axis] - a[axis];
    let t = if denom.abs() > 1e-300 {
        (bound - a[axis]) / denom
    } else {
        0.0
    };
    let other = 1 - axis;
    let mut out = a;
    out[axis] = bound;
    out[other] = a[other] + (b[other] - a[other]) * t;
    out
}

fn dedupe_points(points: &mut Vec<[f64; 2]>) {
    points.dedup();
    if points.len() >= 2 && points.first() == points.last() {
        points.pop();
    }
}

/// One triangle vertex carrying every attribute the mesh can interpolate.
#[derive(Clone, Copy)]
struct ClipVertex {
    pos: [f64; 2],
    z: f64,
    normal: Option<Point3>,
    color: Option<[u8; 3]>,
}

impl ClipVertex {
    /// Whether two vertices are identical in every clipped attribute, so a
    /// duplicate produced by clipping can be removed without losing data.
    fn same(&self, other: &ClipVertex) -> bool {
        self.pos == other.pos
            && self.z == other.z
            && self.normal == other.normal
            && self.color == other.color
    }

    fn lerp(&self, other: &ClipVertex, t: f64) -> ClipVertex {
        ClipVertex {
            pos: [
                self.pos[0] + (other.pos[0] - self.pos[0]) * t,
                self.pos[1] + (other.pos[1] - self.pos[1]) * t,
            ],
            z: self.z + (other.z - self.z) * t,
            normal: match (self.normal, other.normal) {
                (Some(a), Some(b)) => Some(Point3 {
                    x: a.x + (b.x - a.x) * t,
                    y: a.y + (b.y - a.y) * t,
                    z: a.z + (b.z - a.z) * t,
                }),
                _ => None,
            },
            color: match (self.color, other.color) {
                (Some(a), Some(b)) => Some([
                    lerp_channel(a[0], b[0], t),
                    lerp_channel(a[1], b[1], t),
                    lerp_channel(a[2], b[2], t),
                ]),
                _ => None,
            },
        }
    }
}

fn lerp_channel(a: u8, b: u8, t: f64) -> u8 {
    let value = a as f64 + (b as f64 - a as f64) * t;
    value.round().clamp(0.0, 255.0) as u8
}

fn clip_clip_vertices(
    input: Vec<ClipVertex>,
    axis: usize,
    bound: f64,
    keep_above: bool,
) -> Vec<ClipVertex> {
    if input.is_empty() {
        return Vec::new();
    }
    let inside = |p: [f64; 2]| {
        if keep_above {
            p[axis] >= bound
        } else {
            p[axis] <= bound
        }
    };
    let n = input.len();
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..n {
        let current = input[i];
        let previous = input[(i + n - 1) % n];
        let current_in = inside(current.pos);
        let previous_in = inside(previous.pos);
        if current_in {
            if !previous_in {
                out.push(intersect_clip_vertices(previous, current, axis, bound));
            }
            out.push(current);
        } else if previous_in {
            out.push(intersect_clip_vertices(previous, current, axis, bound));
        }
    }
    out
}

fn intersect_clip_vertices(a: ClipVertex, b: ClipVertex, axis: usize, bound: f64) -> ClipVertex {
    let denom = b.pos[axis] - a.pos[axis];
    let t = if denom.abs() > 1e-300 {
        (bound - a.pos[axis]) / denom
    } else {
        0.0
    };
    let mut out = a.lerp(&b, t);
    // Snap exactly onto the clip plane so the boundary is exact, not rounded.
    out.pos[axis] = bound;
    out
}

fn push_clip_vertices(
    vertices: &mut Vec<Point3>,
    normals: &mut Vec<Point3>,
    colors: &mut Vec<[u8; 3]>,
    source: &[ClipVertex],
    has_normals: bool,
    has_colors: bool,
) -> Result<u32, ClipRefusal> {
    let base = u32::try_from(vertices.len()).map_err(|_| {
        ClipRefusal::new(
            clip_reason::MESH_UNCLIPPABLE,
            "clipped mesh exceeds 32-bit vertex addressing",
        )
    })?;
    if vertices.len().saturating_add(source.len()) > u32::MAX as usize {
        return Err(ClipRefusal::new(
            clip_reason::MESH_UNCLIPPABLE,
            "clipped mesh exceeds 32-bit vertex addressing",
        ));
    }
    for vertex in source {
        vertices.push(Point3 {
            x: vertex.pos[0],
            y: vertex.pos[1],
            z: vertex.z,
        });
        if has_normals {
            normals.push(crate::transform::normalize(
                vertex.normal.unwrap_or_default(),
            ));
        }
        if has_colors {
            colors.push(vertex.color.unwrap_or([0, 0, 0]));
        }
    }
    Ok(base)
}

/// Clip every triangle of `mesh` against the paper rectangle `[center ± half]`.
///
/// Each triangle is clipped independently with Sutherland–Hodgman, then
/// fan-triangulated; per-vertex z, normal and sRGB colour are interpolated at
/// the new edge vertices, so a partially visible gradient facet keeps its
/// correct colour ramp. A triangle wholly inside is reused verbatim; a triangle
/// wholly outside disappears.
///
/// `Ok(None)` means the whole mesh lies outside the window (an exact omission).
/// `Err` means the mesh cannot be clipped exactly (a missing index, a non-finite
/// coordinate, or a 32-bit vertex overflow); the caller must keep the original
/// geometry and report the refusal instead of dropping or faking it.
pub fn clip_mesh_to_rect(
    mesh: &Mesh,
    center: [f64; 2],
    half: [f64; 2],
) -> Result<Option<Mesh>, ClipRefusal> {
    let min = [center[0] - half[0], center[1] - half[1]];
    let max = [center[0] + half[0], center[1] + half[1]];
    let has_normals = mesh.normals.len() == mesh.vertices.len();
    let has_colors = mesh.colors.len() == mesh.vertices.len();
    let has_sources = mesh.face_sources.len() == mesh.triangles.len();

    for vertex in &mesh.vertices {
        if !finite_point(vertex) {
            return Err(ClipRefusal::new(
                clip_reason::MESH_UNCLIPPABLE,
                "mesh has a non-finite vertex; it cannot be clipped exactly",
            ));
        }
    }

    let mut vertices: Vec<Point3> = Vec::new();
    let mut normals: Vec<Point3> = Vec::new();
    let mut colors: Vec<[u8; 3]> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut face_sources: Vec<Option<SubElementId>> = Vec::new();

    for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
        let indices = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        if indices.iter().any(|index| *index >= mesh.vertices.len()) {
            return Err(ClipRefusal::new(
                clip_reason::MESH_UNCLIPPABLE,
                format!("mesh triangle {triangle_index} references a missing vertex"),
            ));
        }
        let source = if has_sources {
            mesh.face_sources[triangle_index].clone()
        } else {
            None
        };
        let originals = [0usize, 1, 2].map(|k| {
            let vi = indices[k];
            ClipVertex {
                pos: [mesh.vertices[vi].x, mesh.vertices[vi].y],
                z: mesh.vertices[vi].z,
                normal: has_normals.then(|| mesh.normals[vi]),
                color: has_colors.then(|| mesh.colors[vi]),
            }
        });

        // A triangle wholly inside is reused without splitting or re-welding.
        if originals.iter().all(|v| {
            v.pos[0] >= min[0] && v.pos[0] <= max[0] && v.pos[1] >= min[1] && v.pos[1] <= max[1]
        }) {
            let base = push_clip_vertices(
                &mut vertices,
                &mut normals,
                &mut colors,
                &originals,
                has_normals,
                has_colors,
            )?;
            triangles.push([base, base + 1, base + 2]);
            face_sources.push(source.clone());
            continue;
        }

        let mut polygon = originals.to_vec();
        polygon = clip_clip_vertices(polygon, 0, min[0], true);
        polygon = clip_clip_vertices(polygon, 0, max[0], false);
        polygon = clip_clip_vertices(polygon, 1, min[1], true);
        polygon = clip_clip_vertices(polygon, 1, max[1], false);
        polygon.dedup_by(|a, b| a.same(b));
        if polygon.len() >= 2 {
            let (first, last) = (polygon[0], polygon[polygon.len() - 1]);
            if first.same(&last) {
                polygon.pop();
            }
        }
        if polygon.len() < 3 {
            // A fully clipped (or degenerate) triangle contributes no area.
            continue;
        }
        let base = push_clip_vertices(
            &mut vertices,
            &mut normals,
            &mut colors,
            &polygon,
            has_normals,
            has_colors,
        )?;
        for k in 1..polygon.len() - 1 {
            triangles.push([base, base + k as u32, base + k as u32 + 1]);
            face_sources.push(source.clone());
        }
    }

    if triangles.is_empty() {
        return Ok(None);
    }
    Ok(Some(Mesh {
        vertices,
        triangles,
        normals: if has_normals { normals } else { Vec::new() },
        face_sources,
        colors: if has_colors { colors } else { Vec::new() },
    }))
}

/// Clip an image's unit-square quad, mapped through `transform`, to the paper
/// rectangle and interpolate the texture coordinates at the new vertices.
///
/// `Ok(vertices)` is the surviving convex polygon (empty when the image lies
/// fully outside the window, an exact omission). `Err` means the transform maps
/// the quad to a non-finite position, so it cannot be clipped.
pub fn clip_image_quad_to_rect(
    transform: &Transform3,
    center: [f64; 2],
    half: [f64; 2],
) -> Result<Vec<ImageVertex>, ClipRefusal> {
    let corners = [
        (Point3::default(), [0.0, 0.0]),
        (
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            [1.0, 0.0],
        ),
        (
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
            [1.0, 1.0],
        ),
        (
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            [0.0, 1.0],
        ),
    ];
    let mut polygon: Vec<ImageVertex> = Vec::with_capacity(8);
    for (corner, uv) in corners {
        let position = transform.apply_point(corner);
        if !finite_point(&position) {
            return Err(ClipRefusal::new(
                clip_reason::IMAGE_DEGENERATE,
                "image transform maps the quad to a non-finite position; it cannot be clipped",
            ));
        }
        polygon.push(ImageVertex { position, uv });
    }
    Ok(clip_image_polygon_to_rect(polygon, center, half))
}

/// Clip an image polygon (already in paper coordinates, carrying UVs) to the
/// paper rectangle, interpolating the texture coordinates at the new vertices.
///
/// The polygon must be convex (the renderer fan-triangulates it); Sutherland–
/// Hodgman against the four axis-aligned half-planes then yields the exact
/// intersection. An empty result means the polygon lies fully outside the
/// window — an exact omission.
fn clip_image_polygon_to_rect(
    mut polygon: Vec<ImageVertex>,
    center: [f64; 2],
    half: [f64; 2],
) -> Vec<ImageVertex> {
    let min = [center[0] - half[0], center[1] - half[1]];
    let max = [center[0] + half[0], center[1] + half[1]];
    polygon = clip_image_vertices(polygon, 0, min[0], true);
    polygon = clip_image_vertices(polygon, 0, max[0], false);
    polygon = clip_image_vertices(polygon, 1, min[1], true);
    polygon = clip_image_vertices(polygon, 1, max[1], false);
    polygon.dedup_by(|a, b| a.position == b.position && a.uv == b.uv);
    if polygon.len() >= 2 {
        let (first, last) = (polygon[0], polygon[polygon.len() - 1]);
        if first.position == last.position && first.uv == last.uv {
            polygon.pop();
        }
    }
    polygon
}

fn clip_image_vertices(
    input: Vec<ImageVertex>,
    axis: usize,
    bound: f64,
    keep_above: bool,
) -> Vec<ImageVertex> {
    if input.is_empty() {
        return Vec::new();
    }
    let inside = |p: [f64; 2]| {
        if keep_above {
            p[axis] >= bound
        } else {
            p[axis] <= bound
        }
    };
    let n = input.len();
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..n {
        let current = input[i];
        let previous = input[(i + n - 1) % n];
        let current_in = inside([current.position.x, current.position.y]);
        let previous_in = inside([previous.position.x, previous.position.y]);
        if current_in {
            if !previous_in {
                out.push(intersect_image_vertices(previous, current, axis, bound));
            }
            out.push(current);
        } else if previous_in {
            out.push(intersect_image_vertices(previous, current, axis, bound));
        }
    }
    out
}

fn intersect_image_vertices(
    a: ImageVertex,
    b: ImageVertex,
    axis: usize,
    bound: f64,
) -> ImageVertex {
    let a_coord = [a.position.x, a.position.y][axis];
    let b_coord = [b.position.x, b.position.y][axis];
    let denom = b_coord - a_coord;
    let t = if denom.abs() > 1e-300 {
        (bound - a_coord) / denom
    } else {
        0.0
    };
    let mut position = a.position;
    if axis == 0 {
        position.x = bound;
    } else {
        position.y = bound;
    }
    let other = 1 - axis;
    let a_other = [a.position.x, a.position.y][other];
    let b_other = [b.position.x, b.position.y][other];
    let other_value = a_other + (b_other - a_other) * t;
    if other == 0 {
        position.x = other_value;
    } else {
        position.y = other_value;
    }
    position.z = a.position.z + (b.position.z - a.position.z) * t;
    ImageVertex {
        position,
        uv: [
            a.uv[0] + (b.uv[0] - a.uv[0]) * t,
            a.uv[1] + (b.uv[1] - a.uv[1]) * t,
        ],
    }
}

/// Whether every corner of an image's unit-square quad is inside the window, so
/// the image needs no clip polygon at all.
fn image_quad_inside(transform: &Transform3, center: [f64; 2], half: [f64; 2]) -> bool {
    let min = [center[0] - half[0], center[1] - half[1]];
    let max = [center[0] + half[0], center[1] + half[1]];
    [
        Point3::default(),
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    ]
    .iter()
    .all(|corner| {
        let p = transform.apply_point(*corner);
        p.x >= min[0] && p.x <= max[0] && p.y >= min[1] && p.y <= max[1]
    })
}

/// Whether an image polygon (paper-space positions, at least three vertices) is
/// convex.
///
/// Consecutive collinear edges are allowed; a sign change between consecutive
/// edge cross products is a reflex vertex. The renderer fan-triangulates a clip
/// polygon assuming convexity, so only a convex polygon may be intersected with
/// the viewport rectangle by half-plane clipping.
fn image_polygon_is_convex(polygon: &[ImageVertex]) -> bool {
    let n = polygon.len();
    if n < 3 {
        return true;
    }
    let mut sign = 0i8;
    for i in 0..n {
        let a = polygon[i].position;
        let b = polygon[(i + 1) % n].position;
        let c = polygon[(i + 2) % n].position;
        let e1 = (b.x - a.x, b.y - a.y);
        let e2 = (c.x - b.x, c.y - b.y);
        let len = e1.0.hypot(e1.1) * e2.0.hypot(e2.1);
        if len <= 0.0 {
            // A repeated vertex: no turn to classify.
            continue;
        }
        let cross = e1.0 * e2.1 - e1.1 * e2.0;
        if cross.abs() <= 1e-9 * len {
            // Collinear: allowed, does not fix the winding sign.
            continue;
        }
        let s = if cross > 0.0 { 1 } else { -1 };
        if sign == 0 {
            sign = s;
        } else if sign != s {
            return false;
        }
    }
    true
}

/// Rebuild a fragment with a new primitive, preserving all source/style fields.
fn fragment_with(fragment: DisplayFragment, primitive: DisplayPrimitive) -> DisplayFragment {
    DisplayFragment {
        source: fragment.source,
        geometry_source: fragment.geometry_source,
        precision: fragment.precision,
        alpha: fragment.alpha,
        color: fragment.color,
        color_unresolved: fragment.color_unresolved,
        lineweight: fragment.lineweight,
        lineweight_unresolved: fragment.lineweight_unresolved,
        linetype: fragment.linetype,
        linetype_unresolved: fragment.linetype_unresolved,
        linetype_scale: fragment.linetype_scale,
        primitive,
    }
}

/// Record a clip degradation on the paper build: weaken completeness and attach
/// a diagnostic naming the stable reason.
fn report_clip_degradation(
    out: &mut DisplayRepresentation,
    refusal: &ClipRefusal,
    layout_id: LayoutId,
    viewport_index: usize,
) {
    let reason = format!("{}: {}", refusal.reason, refusal.message);
    out.completeness = weaker(
        &out.completeness,
        &Completeness::Partial(vec![reason.clone()]),
    );
    out.diagnostics.push(Diagnostic {
        object: None,
        code: "representation.viewport_clip_partial".into(),
        message: format!("layout {} viewport {viewport_index}: {reason}", layout_id.0),
    });
}

/// Build the display representation for one paper-space layout (spec §3.3, F04).
///
/// The result contains, in this order:
///
/// 1. the layout's own paper-space entities, drawn as-is;
/// 2. for each **supported** rectangular viewport, the model-space entities
///    mapped onto the sheet and clipped to the viewport rectangle.
///
/// A viewport whose state cannot be drawn exactly contributes no geometry and a
/// `Partial` report plus a `representation.viewport_unsupported` diagnostic
/// naming the stable reason — never a guessed transform. A layout that does not
/// exist is `Missing`. `visible` is the caller's layer-visibility predicate
/// (kept as a closure so this crate stays independent of `cad-app`).
///
/// For **line/fill** geometry each segment is clipped exactly (Liang–Barsky);
/// for a **mesh** each triangle is clipped exactly (Sutherland–Hodgman) with
/// per-vertex position, z, normal and colour interpolated at the new vertices;
/// for an **image** the unit-square quad is clipped and its texture coordinates
/// interpolated. When an image also carries an entity clip, that convex polygon
/// is intersected with the viewport rectangle (its UVs interpolated at the new
/// vertices) so the entity clip is never dropped or widened. Geometry the layer
/// window removes completely simply disappears. What cannot be clipped exactly
/// without faking it is kept unclipped and reported `Partial` with a stable
/// [`clip_reason`] code (an unshaped `Text` placeholder has no glyph geometry
/// until a font shapes it; a mesh with a bad index or non-finite vertex; a
/// non-convex entity image clip; an unexpanded instance) — never dropped
/// silently and never claimed as clipped.
pub fn build_paper_space(
    registry: &ProviderRegistry,
    database: &DrawingDatabase,
    layout_id: LayoutId,
    context: &RepresentationContext,
    visible: &dyn Fn(&DbEntity) -> bool,
) -> CadResult<DisplayRepresentation> {
    let Some(layout) = database.layout(layout_id) else {
        return Ok(DisplayRepresentation {
            fragments: Vec::new(),
            completeness: Completeness::Missing(vec![format!(
                "layout {} does not exist",
                layout_id.0
            )]),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "layout.missing".into(),
                message: format!("layout {} does not exist", layout_id.0),
            }],
        });
    };

    let mut out = DisplayRepresentation {
        fragments: Vec::new(),
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
    };

    // 1. The sheet's own geometry, already in paper coordinates.
    for entity in database.layout_entities(layout_id) {
        if !visible(entity) {
            continue;
        }
        let rep = registry.build_expanded(database, entity, context)?;
        merge(&mut out, rep);
    }

    // 2. Each viewport's window into model space.
    for (index, viewport) in layout.viewports.iter().enumerate() {
        let transform = match viewport_transform(viewport) {
            ViewportState::Supported(t) => t,
            ViewportState::Unsupported(why) => {
                // The sheet itself is still drawable, so the layout is Partial,
                // not Missing; the viewport's content is simply absent.
                out.completeness = weaker(
                    &out.completeness,
                    &Completeness::Partial(vec![why.to_string()]),
                );
                out.diagnostics.push(Diagnostic {
                    object: None,
                    code: "representation.viewport_unsupported".into(),
                    message: format!("layout {} viewport {index}: {why}", layout_id.0),
                });
                continue;
            }
        };
        let matrix = transform.as_transform3();
        let center = transform.paper_center;
        let half = transform.paper_half;
        for entity in database.model_space() {
            if !visible(entity) {
                continue;
            }
            let rep = registry.build_expanded(database, entity, context)?;
            out.completeness = weaker(&out.completeness, &rep.completeness);
            out.diagnostics.extend(rep.diagnostics);
            for fragment in rep.fragments {
                let transformed = fragment.primitive.transformed(&matrix);
                match transformed {
                    DisplayPrimitive::LineSegments(points) => {
                        if points.len() % 2 != 0 {
                            return Err(CadError::InvalidInput(
                                "line segments require endpoint pairs".into(),
                            ));
                        }
                        let mut clipped = Vec::new();
                        for pair in points.chunks_exact(2) {
                            for run in clip_polyline_to_rect(pair, center, half) {
                                clipped.extend(run);
                            }
                        }
                        if !clipped.is_empty() {
                            out.fragments.push(DisplayFragment {
                                source: fragment.source,
                                geometry_source: fragment.geometry_source,
                                precision: fragment.precision,
                                alpha: fragment.alpha,
                                color: fragment.color,
                                color_unresolved: fragment.color_unresolved,
                                lineweight: fragment.lineweight,
                                lineweight_unresolved: fragment.lineweight_unresolved,
                                linetype: fragment.linetype,
                                linetype_unresolved: fragment.linetype_unresolved,
                                linetype_scale: fragment.linetype_scale,
                                primitive: DisplayPrimitive::LineSegments(std::sync::Arc::from(
                                    clipped.into_boxed_slice(),
                                )),
                            });
                        }
                    }
                    DisplayPrimitive::Lines(points) => {
                        for run in clip_polyline_to_rect(&points, center, half) {
                            out.fragments.push(DisplayFragment {
                                source: fragment.source.clone(),
                                geometry_source: fragment.geometry_source.clone(),
                                precision: fragment.precision.clone(),
                                alpha: fragment.alpha,
                                color: fragment.color,
                                color_unresolved: fragment.color_unresolved,
                                lineweight: fragment.lineweight,
                                lineweight_unresolved: fragment.lineweight_unresolved,
                                linetype: fragment.linetype.clone(),
                                linetype_unresolved: fragment.linetype_unresolved,
                                linetype_scale: fragment.linetype_scale,
                                primitive: DisplayPrimitive::Lines(std::sync::Arc::from(
                                    run.into_boxed_slice(),
                                )),
                            });
                        }
                    }
                    DisplayPrimitive::Mesh(mesh) => match clip_mesh_to_rect(&mesh, center, half) {
                        Ok(Some(clipped)) => {
                            out.fragments.push(fragment_with(
                                fragment,
                                DisplayPrimitive::Mesh(std::sync::Arc::new(clipped)),
                            ));
                        }
                        // Every triangle lies outside the window: an exact
                        // omission, not a degradation.
                        Ok(None) => {}
                        Err(refusal) => {
                            report_clip_degradation(&mut out, &refusal, layout_id, index);
                            out.fragments
                                .push(fragment_with(fragment, DisplayPrimitive::Mesh(mesh)));
                        }
                    },
                    DisplayPrimitive::Text {
                        text,
                        origin,
                        font,
                        height,
                    } => {
                        // Glyph geometry exists only once a font shapes the
                        // text. An unshaped placeholder cannot be clipped here,
                        // so it is kept and reported, never dropped or faked.
                        let refusal = ClipRefusal::new(
                            clip_reason::TEXT_FONT_DEPENDENT,
                            "unshaped text glyph geometry depends on a font; the viewport cannot clip it exactly",
                        );
                        report_clip_degradation(&mut out, &refusal, layout_id, index);
                        out.fragments.push(fragment_with(
                            fragment,
                            DisplayPrimitive::Text {
                                text,
                                origin,
                                font,
                                height,
                            },
                        ));
                    }
                    DisplayPrimitive::Image {
                        resource,
                        transform: local,
                        clip,
                    } => match clip_image_quad_to_rect(&local, center, half) {
                        Ok(vertices) if vertices.is_empty() => {
                            // Fully outside the window: exact omission.
                        }
                        Ok(vertices) => {
                            // A clipped textured quad is represented exactly:
                            // positions plus interpolated UVs, so it is not a
                            // degradation. A wholly visible image keeps its
                            // simple unit-square transform. When an entity clip
                            // and a viewport cut both apply, the two convex
                            // polygons are intersected so the entity clip is
                            // never silently widened or dropped; `None` (outer)
                            // means the intersection is empty — an exact
                            // omission.
                            let effective: Option<Option<std::sync::Arc<[ImageVertex]>>> =
                                if image_quad_inside(&local, center, half) {
                                    // The whole quad is visible; any entity clip
                                    // still applies unchanged.
                                    Some(clip)
                                } else if let Some(entity_clip) = clip {
                                    if image_polygon_is_convex(&entity_clip) {
                                        let intersected = clip_image_polygon_to_rect(
                                            entity_clip.to_vec(),
                                            center,
                                            half,
                                        );
                                        if intersected.is_empty() {
                                            None
                                        } else {
                                            Some(Some(std::sync::Arc::from(
                                                intersected.into_boxed_slice(),
                                            )))
                                        }
                                    } else {
                                        // A provider handed us a non-convex clip
                                        // (its primitive contract says convex);
                                        // half-plane intersection would be wrong,
                                        // so keep the entity clip and report the
                                        // limitation rather than widen or drop it.
                                        let refusal = ClipRefusal::new(
                                            clip_reason::IMAGE_INTERSECTION_UNSUPPORTED,
                                            "a non-convex image clip cannot be intersected with the viewport window exactly; the entity clip is kept unclipped",
                                        );
                                        report_clip_degradation(
                                            &mut out, &refusal, layout_id, index,
                                        );
                                        Some(Some(entity_clip))
                                    }
                                } else {
                                    Some(Some(std::sync::Arc::from(vertices.into_boxed_slice())))
                                };
                            if let Some(clip) = effective {
                                out.fragments.push(fragment_with(
                                    fragment,
                                    DisplayPrimitive::Image {
                                        resource,
                                        transform: local,
                                        clip,
                                    },
                                ));
                            }
                        }
                        Err(refusal) => {
                            report_clip_degradation(&mut out, &refusal, layout_id, index);
                            out.fragments.push(fragment_with(
                                fragment,
                                DisplayPrimitive::Image {
                                    resource,
                                    transform: local,
                                    clip,
                                },
                            ));
                        }
                    },
                    other => {
                        // An unexpanded instance has no geometry to clip; keep
                        // it and report rather than fake a windowed version.
                        let refusal = ClipRefusal::new(
                            clip_reason::UNSUPPORTED_PRIMITIVE,
                            "this primitive cannot be clipped against the viewport window",
                        );
                        report_clip_degradation(&mut out, &refusal, layout_id, index);
                        out.fragments.push(fragment_with(fragment, other));
                    }
                }
            }
        }
    }

    Ok(out)
}

fn merge(out: &mut DisplayRepresentation, rep: DisplayRepresentation) {
    out.completeness = weaker(&out.completeness, &rep.completeness);
    out.diagnostics.extend(rep.diagnostics);
    out.fragments.extend(rep.fragments);
}

fn weaker(a: &Completeness, b: &Completeness) -> Completeness {
    fn rank(c: &Completeness) -> u8 {
        match c {
            Completeness::Complete => 3,
            Completeness::Partial(_) => 1,
            Completeness::Missing(_) => 0,
            Completeness::Unverified => 2,
        }
    }
    if rank(a) <= rank(b) {
        a.clone()
    } else {
        b.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{DbObject, DrawingDatabaseBuilder, Layer};

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn viewport(clip: Vec<Point3>, scale: f64) -> PaperViewport {
        PaperViewport {
            clip,
            model_to_paper: Transform3::scale(scale),
            completeness: Completeness::Complete,
        }
    }

    /// Paper→model transform `model = scale * paper + translation`, matching the
    /// direction the importer stores (`view_height / paper_height`).
    fn stored_view(scale: f64, translation: Point3) -> Transform3 {
        let mut m = Transform3::scale(scale).matrix;
        m[0][3] = translation.x;
        m[1][3] = translation.y;
        m[2][3] = translation.z;
        Transform3 { matrix: m }
    }

    /// The importer's shape: two opposite paper corners + the model view centre,
    /// with a model-units-per-paper-unit scale.
    fn simple_viewport() -> PaperViewport {
        // Paper rectangle (0,0)-(100,50), model centre at (10, 20), 1:100 => the
        // stored ratio is 100 model units per paper unit.
        viewport(vec![p(0.0, 0.0), p(100.0, 50.0), p(10.0, 20.0)], 100.0)
    }

    /// A real four-corner 1:100 viewport: paper (0,0)-(100,50), model view
    /// centre (10,20). The stored paper→model transform is `scale(100)` shifted
    /// so that `paper_center = (50,25)` maps to `(10,20)`.
    fn four_corner_viewport() -> PaperViewport {
        let translation = Point3 {
            x: 10.0 - 100.0 * 50.0,
            y: 20.0 - 100.0 * 25.0,
            z: 0.0,
        };
        PaperViewport {
            clip: vec![p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)],
            model_to_paper: stored_view(100.0, translation),
            completeness: Completeness::Complete,
        }
    }

    // ---- scale direction (B22) ---------------------------------------------

    #[test]
    fn paper_per_model_is_paper_height_over_view_height() {
        // 1:100: a 1000-unit-tall model region fits a 10-unit-tall paper window.
        assert_eq!(paper_per_model_from_view(1000.0, 10.0), Some(0.01));
        // The stored legacy scalar is the reciprocal.
        assert_eq!(
            paper_per_model_from_view(1000.0, 10.0).map(|s| 1.0 / s),
            Some(100.0)
        );
        assert_eq!(paper_per_model_from_view(0.0, 10.0), None);
        assert_eq!(paper_per_model_from_view(1000.0, f64::NAN), None);
    }

    // ---- supported forms ---------------------------------------------------

    #[test]
    fn four_corner_clip_yields_the_four_paper_corners() {
        let ViewportState::Supported(t) = viewport_transform(&four_corner_viewport()) else {
            panic!("expected a supported viewport");
        };
        assert_eq!(
            t.paper_corners,
            [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)]
        );
        assert_eq!(t.paper_center, [50.0, 25.0]);
        assert_eq!(t.paper_half, [50.0, 25.0]);
        assert_eq!(t.paper_bounds(), ([0.0, 0.0], [100.0, 50.0]));
        // 1:100 → paper 0.01 per model unit.
        assert!((t.paper_per_model - 0.01).abs() < 1e-12);
        // The view centre is the model point shown at the rectangle centre.
        assert_eq!(t.model_anchor, p(10.0, 20.0));
        assert_eq!(t.model_to_paper(p(10.0, 20.0)), p(50.0, 25.0));
    }

    #[test]
    fn known_one_to_one_hundred_viewport_distances() {
        let ViewportState::Supported(t) = viewport_transform(&four_corner_viewport()) else {
            panic!("expected a supported viewport");
        };
        // Paper width 100 ↔ model width 10000; paper height 50 ↔ model 5000.
        let ll = t.paper_to_model(p(0.0, 0.0));
        let lr = t.paper_to_model(p(100.0, 0.0));
        let ul = t.paper_to_model(p(0.0, 50.0));
        assert!((ll.x - lr.x).abs() - 10000.0 < 1e-6, "got {ll:?} {lr:?}");
        assert!((ll.y - ul.y).abs() - 5000.0 < 1e-6, "got {ll:?} {ul:?}");
        // The four paper corners round-trip through the inverse exactly.
        for corner in t.paper_corners {
            let model = t.paper_to_model(corner);
            let back = t.model_to_paper(model);
            assert!((back.x - corner.x).abs() < 1e-9);
            assert!((back.y - corner.y).abs() < 1e-9);
        }
        // One paper unit is exactly 100 model units.
        let a = t.paper_to_model(p(0.0, 0.0));
        let b = t.paper_to_model(p(1.0, 0.0));
        assert!((b.x - a.x - 100.0).abs() < 1e-9);
    }

    #[test]
    fn legacy_three_point_clip_still_derives_the_rectangle() {
        let ViewportState::Supported(t) = viewport_transform(&simple_viewport()) else {
            panic!("expected a supported viewport");
        };
        assert_eq!(t.paper_center, [50.0, 25.0]);
        assert_eq!(t.paper_half, [50.0, 25.0]);
        assert_eq!(
            t.paper_corners,
            [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)]
        );
        assert_eq!(t.model_anchor, p(10.0, 20.0));
        // The stored 100 is model-units-per-paper-unit; the mapping uses the
        // inverse (audit B22 "方向需纠正").
        assert_eq!(t.paper_per_model, 0.01);
    }

    /// Exactly the shape the importer now writes for a 1:100 top/plan viewport:
    /// paper (0,0)-(100,50), model view target (10,20), model_per_paper = 100.
    fn imported_one_to_one_hundred() -> PaperViewport {
        let center = Point3 {
            x: 50.0,
            y: 25.0,
            z: 0.0,
        };
        let target = Point3 {
            x: 10.0,
            y: 20.0,
            z: 0.0,
        };
        let scale = 100.0;
        let clip = vec![p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)];
        let mut m = Transform3::identity().matrix;
        m[0][0] = scale;
        m[1][1] = scale;
        m[0][3] = target.x - scale * center.x;
        m[1][3] = target.y - scale * center.y;
        PaperViewport {
            clip,
            model_to_paper: Transform3 { matrix: m },
            completeness: Completeness::Complete,
        }
    }

    #[test]
    fn imported_one_to_one_hundred_viewport_is_supported_and_correct() {
        let ViewportState::Supported(t) = viewport_transform(&imported_one_to_one_hundred()) else {
            panic!("the importer's four-corner 1:100 viewport must be supported");
        };
        // Four paper corners and the correct 1:100 scale direction.
        assert_eq!(
            t.paper_corners,
            [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)]
        );
        assert!((t.paper_per_model - 0.01).abs() < 1e-12);
        // The view target lands at the paper centre.
        assert_eq!(t.model_anchor, p(10.0, 20.0));
        assert_eq!(t.model_to_paper(p(10.0, 20.0)), p(50.0, 25.0));
        // One paper unit is 100 model units through the exact inverse.
        let a = t.paper_to_model(p(0.0, 0.0));
        let b = t.paper_to_model(p(1.0, 0.0));
        assert!((b.x - a.x - 100.0).abs() < 1e-9);
    }

    #[test]
    fn imported_rotated_viewport_is_partial_with_the_twist_code() {
        // The importer writes a Partial record (with the same clip/transform)
        // for a twisted view; the representation layer must surface the stable
        // twist code, not square the rectangle off.
        let mut vp = imported_one_to_one_hundred();
        vp.completeness = Completeness::Partial(vec!["viewport has a view twist".into()]);
        assert_eq!(unsupported_code(&vp), viewport_reason::TWISTED_TRANSFORM);
    }

    #[test]
    fn imported_off_plane_viewport_is_partial_with_the_off_plane_code() {
        let mut vp = imported_one_to_one_hundred();
        vp.completeness = Completeness::Partial(vec![
            "viewport view direction is not perpendicular to the paper plane".into(),
        ]);
        assert_eq!(unsupported_code(&vp), viewport_reason::OFF_PLANE_VIEW);
    }

    #[test]
    fn imported_perspective_viewport_is_partial_with_the_perspective_code() {
        let mut vp = imported_one_to_one_hundred();
        vp.completeness =
            Completeness::Partial(vec!["perspective viewport is not supported".into()]);
        assert_eq!(unsupported_code(&vp), viewport_reason::PERSPECTIVE);
    }

    #[test]
    fn imported_complex_clip_is_partial_with_the_complex_clip_code() {
        let mut vp = imported_one_to_one_hundred();
        vp.completeness =
            Completeness::Partial(vec!["non-rectangular viewport clip is not applied".into()]);
        assert_eq!(unsupported_code(&vp), viewport_reason::COMPLEX_CLIP);
    }

    #[test]
    fn model_to_paper_places_the_anchor_at_the_centre() {
        let ViewportState::Supported(t) = viewport_transform(&simple_viewport()) else {
            panic!("expected a supported viewport");
        };
        assert_eq!(t.model_to_paper(p(10.0, 20.0)), p(50.0, 25.0));
        // One paper unit right of centre is model x = 110.
        let right = t.model_to_paper(p(110.0, 20.0));
        assert!((right.x - 51.0).abs() < 1e-9);
        assert!((right.y - 25.0).abs() < 1e-9);
    }

    #[test]
    fn paper_to_model_is_the_exact_inverse() {
        let ViewportState::Supported(t) = viewport_transform(&simple_viewport()) else {
            panic!("expected a supported viewport");
        };
        let model = p(123.5, -77.25);
        let paper = t.model_to_paper(model);
        let back = t.paper_to_model(paper);
        assert!((back.x - model.x).abs() < 1e-9);
        assert!((back.y - model.y).abs() < 1e-9);
    }

    // ---- unsupported states ------------------------------------------------

    fn unsupported_code(viewport: &PaperViewport) -> &'static str {
        match viewport_transform(viewport) {
            ViewportState::Unsupported(why) => why.code,
            ViewportState::Supported(_) => panic!("expected an unsupported viewport"),
        }
    }

    #[test]
    fn complex_clip_is_refused_with_its_reason() {
        let mut vp = simple_viewport();
        vp.completeness = Completeness::Partial(vec!["complex viewport clip".into()]);
        let code = unsupported_code(&vp);
        assert_eq!(code, viewport_reason::COMPLEX_CLIP);
        let ViewportState::Unsupported(why) = viewport_transform(&vp) else {
            unreachable!()
        };
        assert!(why.reason().contains("complex viewport clip"));
    }

    #[test]
    fn rotated_four_corner_clip_is_refused() {
        // A 45°-rotated rectangle: all four corners have x strictly inside the
        // bounding box, so it cannot be represented as an axis-aligned rect.
        let mut vp = four_corner_viewport();
        vp.clip = vec![p(50.0, 0.0), p(100.0, 50.0), p(50.0, 100.0), p(0.0, 50.0)];
        assert_eq!(unsupported_code(&vp), viewport_reason::ROTATED_CLIP);
    }

    #[test]
    fn twisted_view_transform_is_refused() {
        let mut vp = four_corner_viewport();
        let c = std::f64::consts::FRAC_1_SQRT_2;
        vp.model_to_paper = Transform3 {
            matrix: [
                [100.0 * c, -100.0 * c, 0.0, 0.0],
                [100.0 * c, 100.0 * c, 0.0, 0.0],
                [0.0, 0.0, 100.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        assert_eq!(unsupported_code(&vp), viewport_reason::TWISTED_TRANSFORM);
    }

    #[test]
    fn mirrored_view_transform_is_refused() {
        let mut vp = four_corner_viewport();
        vp.model_to_paper = stored_view(-100.0, Point3::default());
        assert_eq!(unsupported_code(&vp), viewport_reason::MIRROR);
    }

    #[test]
    fn non_uniform_view_transform_is_refused() {
        let mut vp = four_corner_viewport();
        let mut m = Transform3::scale(100.0).matrix;
        m[1][1] = 200.0;
        vp.model_to_paper = Transform3 { matrix: m };
        assert_eq!(unsupported_code(&vp), viewport_reason::NON_UNIFORM);
    }

    #[test]
    fn off_plane_view_direction_is_refused() {
        // A shear of model z into paper x: a tilted 3D view we cannot flatten.
        let mut vp = four_corner_viewport();
        let mut m = Transform3::scale(100.0).matrix;
        m[0][2] = 5.0;
        vp.model_to_paper = Transform3 { matrix: m };
        assert_eq!(unsupported_code(&vp), viewport_reason::OFF_PLANE_VIEW);
    }

    #[test]
    fn legacy_clip_with_transform_translation_is_ambiguous_and_refused() {
        let mut vp = simple_viewport();
        vp.model_to_paper = stored_view(100.0, p(1.0, 2.0));
        assert_eq!(unsupported_code(&vp), viewport_reason::TRANSLATION_MISMATCH);
    }

    #[test]
    fn missing_scale_is_refused() {
        let vp = viewport(vec![p(0.0, 0.0), p(10.0, 10.0), p(0.0, 0.0)], 0.0);
        assert_eq!(unsupported_code(&vp), viewport_reason::SCALE);
    }

    #[test]
    fn adjacent_clip_corners_are_refused_as_not_opposite() {
        // The importer currently stores the two bottom corners (same y); the
        // paper height is not stored, so this must be refused, not guessed.
        let vp = viewport(vec![p(0.0, 0.0), p(10.0, 0.0), p(0.0, 0.0)], 100.0);
        assert_eq!(unsupported_code(&vp), viewport_reason::ADJACENT_CORNERS);
    }

    #[test]
    fn zero_size_rectangle_is_refused() {
        let vp = viewport(vec![p(0.0, 0.0), p(0.0, 0.0), p(0.0, 0.0)], 1.0);
        assert_eq!(unsupported_code(&vp), viewport_reason::DEGENERATE_RECT);
    }

    #[test]
    fn non_finite_clip_is_refused() {
        let vp = viewport(vec![p(0.0, 0.0), p(f64::NAN, 10.0), p(0.0, 0.0)], 100.0);
        assert_eq!(unsupported_code(&vp), viewport_reason::NON_FINITE);
    }

    #[test]
    fn wrong_arity_clip_is_refused() {
        let vp = viewport(vec![p(0.0, 0.0), p(10.0, 10.0)], 1.0);
        assert_eq!(unsupported_code(&vp), viewport_reason::CLIP_ARITY);
    }

    // ---- rectangular clipping ----------------------------------------------

    #[test]
    fn clipping_keeps_the_inside_part_of_a_crossing_segment() {
        // Rectangle [-1,1]²; a line from (-5,0) to (5,0) survives as (-1,0)-(1,0).
        let runs = clip_polyline_to_rect(&[p(-5.0, 0.0), p(5.0, 0.0)], [0.0, 0.0], [1.0, 1.0]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len(), 2);
        assert!((runs[0][0].x + 1.0).abs() < 1e-9);
        assert!((runs[0][1].x - 1.0).abs() < 1e-9);
    }

    #[test]
    fn clipping_drops_a_fully_outside_polyline() {
        let runs = clip_polyline_to_rect(
            &[p(2.0, 5.0), p(3.0, 6.0), p(4.0, 5.0)],
            [0.0, 0.0],
            [1.0, 1.0],
        );
        assert!(runs.is_empty());
    }

    #[test]
    fn clipping_keeps_a_fully_inside_polyline() {
        let inside = [p(-0.5, -0.5), p(0.0, 0.0), p(0.5, 0.5)];
        let runs = clip_polyline_to_rect(&inside, [0.0, 0.0], [1.0, 1.0]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0], inside.to_vec());
    }

    #[test]
    fn clipping_joins_continuous_segments_through_the_middle() {
        let runs = clip_polyline_to_rect(
            &[p(-3.0, 0.0), p(0.0, 0.0), p(3.0, 0.0)],
            [0.0, 0.0],
            [1.0, 1.0],
        );
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len(), 3);
        assert!((runs[0][0].x + 1.0).abs() < 1e-9);
        assert!((runs[0][2].x - 1.0).abs() < 1e-9);
    }

    #[test]
    fn clipping_splits_a_polyline_that_leaves_and_re_enters() {
        let runs = clip_polyline_to_rect(
            &[
                p(0.0, 0.0),
                p(0.5, 0.0),
                p(0.5, 5.0),
                p(-0.5, 5.0),
                p(-0.5, 0.0),
                p(0.0, 0.0),
            ],
            [0.0, 0.0],
            [1.0, 1.0],
        );
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|run| run.len() >= 2));
    }

    // ---- enumeration -------------------------------------------------------

    fn db_with_layouts() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: p(0.0, 0.0),
                end: p(1.0, 0.0),
            },
            draw_order: 0,
        })
        .unwrap();
        b.insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "Layout1".into(),
            viewports: vec![four_corner_viewport()],
        })
        .unwrap();
        b.insert_layout(cad_db::Layout {
            id: LayoutId(2),
            name: "Layout2".into(),
            viewports: vec![viewport(vec![p(0.0, 0.0), p(10.0, 10.0), p(0.0, 0.0)], 0.0)],
        })
        .unwrap();
        b.finish().unwrap()
    }

    #[test]
    fn enumeration_reports_ids_names_and_support() {
        let db = db_with_layouts();
        let layouts = enumerate_layouts(&db);
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].id, LayoutId(1));
        assert_eq!(layouts[0].name, "Layout1");
        assert!(layouts[0].supported);
        assert_eq!(layouts[0].viewport_count, 1);
        assert!(layouts[0].reason.is_empty());

        assert_eq!(layouts[1].id, LayoutId(2));
        assert!(!layouts[1].supported);
        // The reason is machine-readable: it starts with the stable code.
        assert!(layouts[1].reason.starts_with(viewport_reason::SCALE));
    }

    #[test]
    fn empty_database_has_no_layouts() {
        let db = DrawingDatabaseBuilder::new(DatabaseId(1)).finish().unwrap();
        assert!(enumerate_layouts(&db).is_empty());
    }

    #[test]
    fn space_selection_round_trips() {
        assert!(SpaceSelection::Model.is_model());
        assert_eq!(SpaceSelection::Model.layout(), None);
        let paper = SpaceSelection::Paper(LayoutId(3));
        assert!(!paper.is_model());
        assert_eq!(paper.layout(), Some(LayoutId(3)));
    }

    // ---- paper build -------------------------------------------------------

    fn context() -> RepresentationContext {
        RepresentationContext::new(
            DocumentId(1),
            TolerancePolicy::default(),
            TaskStamp::new(DocumentId(1), 0),
        )
    }

    #[test]
    fn paper_build_maps_and_clips_model_geometry_through_the_viewport() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        for (id, a, z) in [
            // y = 20000 is far above the 1:100 window (visible model y is
            // 20 ± 2500), so this line is clipped away entirely.
            (1u128, p(-5000.0, 20000.0), p(5000.0, 20000.0)),
            (2, p(9.0, 20.0), p(11.0, 20.0)),
        ] {
            b.insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(id),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry: SemanticGeometry::Line { start: a, end: z },
                draw_order: id as i64,
            })
            .unwrap();
        }
        b.insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "L1".into(),
            viewports: vec![simple_viewport()],
        })
        .unwrap();
        let db = b.finish().unwrap();

        let registry = ProviderRegistry::with_default_provider();
        let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
        assert_eq!(rep.completeness, Completeness::Complete);
        assert_eq!(rep.fragments.len(), 1);
        match &rep.fragments[0].primitive {
            DisplayPrimitive::Lines(pts) => {
                assert_eq!(pts.len(), 2);
                // Model x=9 => paper 50 + (9-10)*0.01 = 49.99; x=11 => 50.01.
                assert!((pts[0].x - 49.99).abs() < 1e-9, "got {:?}", pts[0]);
                assert!((pts[1].x - 50.01).abs() < 1e-9, "got {:?}", pts[1]);
                assert!((pts[0].y - 25.0).abs() < 1e-9);
            }
            _ => panic!("expected a clipped line"),
        }
    }

    #[test]
    fn paper_build_reports_an_unsupported_viewport_as_partial() {
        let db = db_with_layouts();
        let registry = ProviderRegistry::with_default_provider();
        // Layout2's only viewport has a zero scale: unsupported. The sheet is
        // still a sheet, so the result is Partial (not Missing) and no guessed
        // viewport geometry is emitted.
        let rep = build_paper_space(&registry, &db, LayoutId(2), &context(), &|_| true).unwrap();
        assert!(matches!(rep.completeness, Completeness::Partial(_)));
        assert!(rep.fragments.is_empty());
        assert!(rep
            .diagnostics
            .iter()
            .any(|d| d.code == "representation.viewport_unsupported"));
    }

    #[test]
    fn paper_build_draws_paper_entities_directly() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        // A paper-space entity belonging to layout 1, plus a layout with a
        // deliberately unsupported viewport (zero scale).
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Paper(LayoutId(1)),
            geometry: SemanticGeometry::Line {
                start: p(0.0, 0.0),
                end: p(5.0, 0.0),
            },
            draw_order: 0,
        })
        .unwrap();
        b.insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "L1".into(),
            viewports: vec![viewport(vec![p(0.0, 0.0), p(10.0, 10.0), p(0.0, 0.0)], 0.0)],
        })
        .unwrap();
        let db = b.finish().unwrap();

        let registry = ProviderRegistry::with_default_provider();
        let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
        // The paper entity is present verbatim even though the viewport is
        // unsupported; the unsupported viewport is reported, not guessed.
        assert_eq!(rep.fragments.len(), 1);
        match &rep.fragments[0].primitive {
            DisplayPrimitive::Lines(pts) => {
                assert_eq!(pts[0], p(0.0, 0.0));
                assert_eq!(pts[1], p(5.0, 0.0));
            }
            _ => panic!("expected the paper line"),
        }
        assert!(rep
            .diagnostics
            .iter()
            .any(|d| d.code == "representation.viewport_unsupported"));
    }

    #[test]
    fn paper_build_reports_a_rotated_viewport_as_partial_not_a_squared_off_guess() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "Rotated".into(),
            viewports: vec![PaperViewport {
                // A 45°-rotated paper rectangle: four corners, but not axis
                // aligned, so it must be refused rather than squared off.
                clip: vec![p(50.0, 0.0), p(100.0, 50.0), p(50.0, 100.0), p(0.0, 50.0)],
                model_to_paper: Transform3::scale(100.0),
                completeness: Completeness::Complete,
            }],
        })
        .unwrap();
        let db = b.finish().unwrap();
        let registry = ProviderRegistry::with_default_provider();
        let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
        assert!(matches!(rep.completeness, Completeness::Partial(_)));
        assert!(rep.fragments.is_empty());
        let diagnostic = rep
            .diagnostics
            .iter()
            .find(|d| d.code == "representation.viewport_unsupported")
            .expect("rotated viewport must be reported");
        assert!(
            diagnostic.message.contains(viewport_reason::ROTATED_CLIP),
            "got {}",
            diagnostic.message
        );
    }

    #[test]
    fn paper_build_for_a_missing_layout_is_missing_not_empty_success() {
        let db = db_with_layouts();
        let registry = ProviderRegistry::with_default_provider();
        let rep = build_paper_space(&registry, &db, LayoutId(99), &context(), &|_| true).unwrap();
        assert!(matches!(rep.completeness, Completeness::Missing(_)));
        assert!(rep.diagnostics.iter().any(|d| d.code == "layout.missing"));
    }

    #[test]
    fn paper_build_honours_the_visibility_predicate() {
        let db = db_with_layouts();
        let registry = ProviderRegistry::with_default_provider();
        // Everything hidden: layout 1 has no paper entities and the model line
        // is filtered, so nothing is drawn.
        let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| false).unwrap();
        assert!(rep.fragments.is_empty());
        assert_eq!(rep.completeness, Completeness::Complete);
    }

    // ---- per-triangle / per-quad viewport clipping -------------------------

    fn triangle_mesh(corners: [Point3; 3]) -> Mesh {
        Mesh {
            vertices: corners.to_vec(),
            triangles: vec![[0, 1, 2]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        }
    }

    fn mesh_area(mesh: &Mesh) -> f64 {
        mesh.triangles
            .iter()
            .map(|t| {
                let a = mesh.vertices[t[0] as usize];
                let b = mesh.vertices[t[1] as usize];
                let c = mesh.vertices[t[2] as usize];
                ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs() * 0.5
            })
            .sum()
    }

    #[test]
    fn glyph_outline_quad_is_clipped_by_the_line_path() {
        // Shaped text arrives as `Lines` glyph outlines, so a font-shaped glyph
        // is clipped by the same Liang–Barsky path as any other polyline. A
        // closed square contour straddling the window keeps only its inside runs.
        let contour = [
            p(-2.0, -1.0),
            p(2.0, -1.0),
            p(2.0, 1.0),
            p(-2.0, 1.0),
            p(-2.0, -1.0),
        ];
        let runs = clip_polyline_to_rect(&contour, [0.0, 0.0], [1.0, 1.0]);
        assert!(!runs.is_empty());
        for run in &runs {
            assert!(run.len() >= 2);
            for v in run {
                assert!(v.x >= -1.0 - 1e-9 && v.x <= 1.0 + 1e-9);
                assert!(v.y >= -1.0 - 1e-9 && v.y <= 1.0 + 1e-9);
            }
        }
    }

    #[test]
    fn clip_polygon_to_rect_keeps_inside_and_drops_outside() {
        let inside = clip_polygon_to_rect(
            &[[-0.5, -0.5], [0.5, -0.5], [0.0, 0.5]],
            [0.0, 0.0],
            [1.0, 1.0],
        );
        assert_eq!(inside.len(), 3);
        let outside = clip_polygon_to_rect(
            &[[2.0, 2.0], [3.0, 2.0], [2.5, 3.0]],
            [0.0, 0.0],
            [1.0, 1.0],
        );
        assert!(outside.is_empty());
    }

    #[test]
    fn mesh_triangle_fully_inside_is_reused_verbatim() {
        let mesh = triangle_mesh([p(-0.5, -0.5), p(0.5, -0.5), p(0.0, 0.5)]);
        let clipped = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
            .unwrap()
            .expect("a triangle inside the window survives");
        assert_eq!(clipped.triangles.len(), 1);
        assert_eq!(clipped.vertices.len(), 3);
        assert_eq!(clipped.vertices[0], p(-0.5, -0.5));
        assert_eq!(clipped.vertices[2], p(0.0, 0.5));
    }

    #[test]
    fn mesh_triangle_fully_outside_disappears() {
        let mesh = triangle_mesh([p(2.0, 2.0), p(3.0, 2.0), p(2.5, 3.0)]);
        assert!(clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
            .unwrap()
            .is_none());
    }

    #[test]
    fn mesh_triangle_partially_clipped_keeps_area_and_fan_triangles() {
        // Triangle (-2,-2),(2,-2),(0,2) ∩ [-1,1]² is a hexagon of area 3.5.
        let mesh = triangle_mesh([p(-2.0, -2.0), p(2.0, -2.0), p(0.0, 2.0)]);
        let clipped = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
            .unwrap()
            .expect("the triangle overlaps the window");
        assert_eq!(clipped.vertices.len(), 6);
        // A fan over an n-gon yields n-2 triangles.
        assert_eq!(clipped.triangles.len(), clipped.vertices.len() - 2);
        let area = mesh_area(&clipped);
        assert!((area - 3.5).abs() < 1e-9, "clipped area {area}");
        // Every clipped vertex stays inside the window.
        for v in &clipped.vertices {
            assert!(v.x >= -1.0 - 1e-9 && v.x <= 1.0 + 1e-9);
            assert!(v.y >= -1.0 - 1e-9 && v.y <= 1.0 + 1e-9);
        }
    }

    #[test]
    fn mesh_per_vertex_colors_interpolate_at_clip_vertices() {
        // Red ramp along x: (-2,0)=0, (0,0)=100, (0,2)=200.
        let mesh = Mesh {
            vertices: vec![p(-2.0, 0.0), p(0.0, 0.0), p(0.0, 2.0)],
            triangles: vec![[0, 1, 2]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: vec![[0, 0, 0], [100, 0, 0], [200, 0, 0]],
        };
        let clipped = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
            .unwrap()
            .expect("overlaps");
        assert_eq!(clipped.colors.len(), clipped.vertices.len());
        // (-1,0) is the x=-1 crossing of the (-2,0)->(0,0) edge: 50%.
        let left = clipped
            .vertices
            .iter()
            .position(|v| (v.x + 1.0).abs() < 1e-9 && v.y.abs() < 1e-9)
            .expect("left crossing");
        assert_eq!(clipped.colors[left][0], 50, "interpolated red channel");
        // (0,1) is the y=1 crossing of the (0,0)->(0,2) edge: 150.
        let top = clipped
            .vertices
            .iter()
            .position(|v| v.x.abs() < 1e-9 && (v.y - 1.0).abs() < 1e-9)
            .expect("top crossing");
        assert_eq!(clipped.colors[top][0], 150, "interpolated red channel");
    }

    #[test]
    fn mesh_with_a_missing_index_is_refused_not_dropped() {
        let mesh = Mesh {
            vertices: vec![p(0.0, 0.0)],
            triangles: vec![[0, 1, 2]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        };
        let refusal = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
            .expect_err("a missing index cannot be clipped exactly");
        assert_eq!(refusal.reason, clip_reason::MESH_UNCLIPPABLE);
    }

    #[test]
    fn mesh_with_a_non_finite_vertex_is_refused_not_dropped() {
        let mesh = triangle_mesh([p(f64::NAN, 0.0), p(0.0, 0.0), p(1.0, 1.0)]);
        let refusal = clip_mesh_to_rect(&mesh, [0.0, 0.0], [1.0, 1.0])
            .expect_err("a non-finite mesh cannot be clipped exactly");
        assert_eq!(refusal.reason, clip_reason::MESH_UNCLIPPABLE);
    }

    #[test]
    fn image_quad_clip_interpolates_uvs() {
        // Identity image: unit square with UV = position. The window looks at
        // x in [0,1], y in [-0.5,0.5], so the top edge is clipped at y=0.5 and
        // its template coordinate v must interpolate to 0.5 (not stretch).
        let vertices = clip_image_quad_to_rect(&Transform3::identity(), [0.5, 0.0], [0.5, 0.5])
            .expect("finite quad");
        assert_eq!(vertices.len(), 4);
        for v in &vertices {
            assert!((v.position.x - v.uv[0]).abs() < 1e-9);
            if (v.position.y - 0.5).abs() < 1e-9 {
                assert!((v.uv[1] - 0.5).abs() < 1e-9, "uv {:?}", v.uv);
            }
        }
    }

    #[test]
    fn image_quad_fully_outside_clips_to_nothing() {
        let vertices = clip_image_quad_to_rect(&Transform3::identity(), [3.0, 3.0], [1.0, 1.0])
            .expect("finite quad");
        assert!(vertices.is_empty());
    }

    #[test]
    fn transformed_image_clip_maps_positions_but_keeps_uvs() {
        let clip = std::sync::Arc::from(
            vec![
                ImageVertex {
                    position: p(0.0, 0.0),
                    uv: [0.0, 0.0],
                },
                ImageVertex {
                    position: p(1.0, 0.0),
                    uv: [1.0, 0.0],
                },
            ]
            .into_boxed_slice(),
        );
        let primitive = DisplayPrimitive::Image {
            resource: cad_resources::ResourceKey("img:0".into()),
            transform: Transform3::identity(),
            clip: Some(clip),
        };
        match primitive.transformed(&Transform3::translation(p(10.0, 5.0))) {
            DisplayPrimitive::Image { clip: Some(v), .. } => {
                assert_eq!(v[0].position, p(10.0, 5.0));
                assert_eq!(v[1].position, p(11.0, 5.0));
                assert_eq!(v[0].uv, [0.0, 0.0]);
                assert_eq!(v[1].uv, [1.0, 0.0]);
            }
            _ => panic!("expected a clipped image primitive"),
        }
    }

    // ---- paper build integration for the new clip paths --------------------

    #[test]
    fn paper_build_clips_a_model_mesh_per_triangle_and_stays_complete() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbFace".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Mesh(triangle_mesh([
                p(0.0, 0.0),
                p(20000.0, 0.0),
                p(0.0, 20000.0),
            ])),
            draw_order: 0,
        })
        .unwrap();
        b.insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "L1".into(),
            viewports: vec![simple_viewport()],
        })
        .unwrap();
        let db = b.finish().unwrap();
        let registry = ProviderRegistry::with_default_provider();
        let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
        // Exact per-triangle clipping is not a degradation.
        assert_eq!(rep.completeness, Completeness::Complete);
        let mesh = rep
            .fragments
            .iter()
            .find_map(|f| match &f.primitive {
                DisplayPrimitive::Mesh(mesh) => Some(mesh.clone()),
                _ => None,
            })
            .expect("clipped mesh fragment");
        assert!(!mesh.triangles.is_empty());
        for v in &mesh.vertices {
            // Paper window is x in [0,100], y in [0,50].
            assert!(v.x >= -1e-9 && v.x <= 100.0 + 1e-9);
            assert!(v.y >= -1e-9 && v.y <= 50.0 + 1e-9);
        }
        assert!(!rep
            .diagnostics
            .iter()
            .any(|d| d.code == "representation.viewport_clip_partial"));
    }

    #[test]
    fn paper_build_keeps_an_unclippable_mesh_partial_and_reported() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        // A triangle that references a missing vertex cannot be clipped exactly.
        let broken = Mesh {
            vertices: vec![p(0.0, 0.0), p(1.0, 0.0)],
            triangles: vec![[0, 1, 2]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        };
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbFace".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Mesh(broken),
            draw_order: 0,
        })
        .unwrap();
        b.insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "L1".into(),
            viewports: vec![simple_viewport()],
        })
        .unwrap();
        let db = b.finish().unwrap();
        let registry = ProviderRegistry::with_default_provider();
        let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
        assert!(matches!(rep.completeness, Completeness::Partial(_)));
        assert!(rep.diagnostics.iter().any(|d| {
            d.code == "representation.viewport_clip_partial"
                && d.message.contains(clip_reason::MESH_UNCLIPPABLE)
        }));
        // The unclipped geometry is kept, never dropped silently.
        assert!(rep
            .fragments
            .iter()
            .any(|f| matches!(f.primitive, DisplayPrimitive::Mesh(_))));
    }

    #[test]
    fn paper_build_keeps_unshaped_text_partial_with_a_precise_reason() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbText".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Text {
                text: "A".into(),
                position: p(10.0, 20.0),
                style: StyleId(0),
                height: 2.5,
                rotation: 0.0,
                font: None,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
            },
            draw_order: 0,
        })
        .unwrap();
        b.insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "L1".into(),
            viewports: vec![simple_viewport()],
        })
        .unwrap();
        let db = b.finish().unwrap();
        let registry = ProviderRegistry::with_default_provider();
        let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
        assert!(matches!(rep.completeness, Completeness::Partial(_)));
        assert!(rep.diagnostics.iter().any(|d| {
            d.code == "representation.viewport_clip_partial"
                && d.message.contains(clip_reason::TEXT_FONT_DEPENDENT)
        }));
        // The unshaped placeholder is kept, never dropped or faked as clipped.
        assert!(rep
            .fragments
            .iter()
            .any(|f| matches!(f.primitive, DisplayPrimitive::Text { .. })));
    }

    /// A provider that emits a single `Image` quad, so the paper-build image
    /// clip path can be exercised without an importer.
    struct ImageProvider;

    impl crate::RepresentationProvider for ImageProvider {
        fn registration(&self) -> crate::Registration {
            crate::Registration {
                type_key: "test.image".into(),
                version: 1,
                priority: 10,
                entity_types: vec!["TestImage".into()],
                capabilities: vec!["image".into()],
            }
        }

        fn build(
            &self,
            entity: &DbEntity,
            context: &RepresentationContext,
        ) -> CadResult<DisplayRepresentation> {
            let mut matrix = Transform3::identity().matrix;
            matrix[0][0] = 20000.0;
            matrix[1][1] = 20000.0;
            matrix[0][3] = 10.0;
            matrix[1][3] = 20.0;
            Ok(DisplayRepresentation {
                fragments: vec![DisplayFragment {
                    source: SelectionRef {
                        document: context.document,
                        entity: entity.id,
                        instance: InstancePath::default(),
                        sub_element: None,
                    },
                    geometry_source: GeometrySource::Analytic,
                    precision: Precision::Analytic,
                    alpha: 1.0,
                    color: crate::DEFAULT_RENDER_COLOR,
                    color_unresolved: true,
                    lineweight: crate::DEFAULT_LINEWEIGHT_MM,
                    lineweight_unresolved: true,
                    linetype: cad_db::LinetypePattern::continuous(),
                    linetype_unresolved: true,
                    linetype_scale: 1.0,
                    primitive: DisplayPrimitive::Image {
                        resource: cad_resources::ResourceKey("img:0".into()),
                        transform: Transform3 { matrix },
                        clip: None,
                    },
                }],
                completeness: Completeness::Complete,
                diagnostics: Vec::new(),
            })
        }
    }

    #[test]
    fn paper_build_clips_a_model_image_quad_with_interpolated_uvs() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "TestImage".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Opaque {
                type_key: "TestImage".into(),
                version: 1,
                payload: Vec::new(),
            },
            draw_order: 0,
        })
        .unwrap();
        b.insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "L1".into(),
            viewports: vec![simple_viewport()],
        })
        .unwrap();
        let db = b.finish().unwrap();
        let mut registry = ProviderRegistry::with_default_provider();
        registry.register(Box::new(ImageProvider)).unwrap();
        let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
        // The clipped textured quad is representable, so this stays exact.
        assert_eq!(rep.completeness, Completeness::Complete);
        let clip = rep
            .fragments
            .iter()
            .find_map(|f| match &f.primitive {
                DisplayPrimitive::Image { clip, .. } => clip.clone(),
                _ => None,
            })
            .expect("clipped image carries its polygon");
        assert_eq!(clip.len(), 4);
        // The image maps the unit square in model space to paper
        // [50,250]x[25,225] through the 1:100 window; the window cuts it to
        // [50,100]x[25,50]. The corner at (100,50) keeps texture UV (0.25,0.125).
        let corner = clip
            .iter()
            .find(|v| (v.position.x - 100.0).abs() < 1e-9 && (v.position.y - 50.0).abs() < 1e-9)
            .expect("clipped corner");
        assert!((corner.uv[0] - 0.25).abs() < 1e-9, "uv {:?}", corner.uv);
        assert!((corner.uv[1] - 0.125).abs() < 1e-9, "uv {:?}", corner.uv);
    }

    /// A provider that emits a single `Image` quad carrying the given entity
    /// clip (in the primitive's model-space coordinates), so the entity-clip +
    /// viewport-clip path can be exercised without an importer. The quad uses
    /// the same model→paper mapping as [`ImageProvider`]: the unit square maps
    /// to paper `[50,250]x[25,225]` inside the `1:100` window `[0,100]x[0,50]`.
    struct ClippedImageProvider {
        clip: std::sync::Arc<[ImageVertex]>,
    }

    impl crate::RepresentationProvider for ClippedImageProvider {
        fn registration(&self) -> crate::Registration {
            crate::Registration {
                type_key: "test.clipped_image".into(),
                version: 1,
                priority: 10,
                entity_types: vec!["TestClippedImage".into()],
                capabilities: vec!["image".into()],
            }
        }

        fn build(
            &self,
            entity: &DbEntity,
            context: &RepresentationContext,
        ) -> CadResult<DisplayRepresentation> {
            let mut matrix = Transform3::identity().matrix;
            matrix[0][0] = 20000.0;
            matrix[1][1] = 20000.0;
            matrix[0][3] = 10.0;
            matrix[1][3] = 20.0;
            Ok(DisplayRepresentation {
                fragments: vec![DisplayFragment {
                    source: SelectionRef {
                        document: context.document,
                        entity: entity.id,
                        instance: InstancePath::default(),
                        sub_element: None,
                    },
                    geometry_source: GeometrySource::Analytic,
                    precision: Precision::Analytic,
                    alpha: 1.0,
                    color: crate::DEFAULT_RENDER_COLOR,
                    color_unresolved: true,
                    lineweight: crate::DEFAULT_LINEWEIGHT_MM,
                    lineweight_unresolved: true,
                    linetype: cad_db::LinetypePattern::continuous(),
                    linetype_unresolved: true,
                    linetype_scale: 1.0,
                    primitive: DisplayPrimitive::Image {
                        resource: cad_resources::ResourceKey("img:0".into()),
                        transform: Transform3 { matrix },
                        clip: Some(self.clip.clone()),
                    },
                }],
                completeness: Completeness::Complete,
                diagnostics: Vec::new(),
            })
        }
    }

    /// A database with one model-space `TestClippedImage` and the standard
    /// `1:100` legacy viewport.
    fn clipped_image_layout() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "TestClippedImage".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Opaque {
                type_key: "TestClippedImage".into(),
                version: 1,
                payload: Vec::new(),
            },
            draw_order: 0,
        })
        .unwrap();
        b.insert_layout(cad_db::Layout {
            id: LayoutId(1),
            name: "L1".into(),
            viewports: vec![simple_viewport()],
        })
        .unwrap();
        b.finish().unwrap()
    }

    #[test]
    fn paper_build_intersects_an_entity_clip_with_a_viewport_cut() {
        // Entity clip: the bottom strip of the image, model y in [20, 2020]
        // (texture v in [0, 0.1]) across the full width. In paper space it is
        // [50,250]x[25,45]; the viewport cuts it to [50,100]x[25,45], so the
        // surviving v stays at most 0.1. Dropping the entity clip (the old
        // behaviour) would have stretched to the full quad's v = 0.125.
        let clip: std::sync::Arc<[ImageVertex]> = std::sync::Arc::from(
            vec![
                ImageVertex {
                    position: p(10.0, 20.0),
                    uv: [0.0, 0.0],
                },
                ImageVertex {
                    position: p(20010.0, 20.0),
                    uv: [1.0, 0.0],
                },
                ImageVertex {
                    position: p(20010.0, 2020.0),
                    uv: [1.0, 0.1],
                },
                ImageVertex {
                    position: p(10.0, 2020.0),
                    uv: [0.0, 0.1],
                },
            ]
            .into_boxed_slice(),
        );
        let db = clipped_image_layout();
        let mut registry = ProviderRegistry::with_default_provider();
        registry
            .register(Box::new(ClippedImageProvider { clip }))
            .unwrap();
        let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
        // The intersection of two convex polygons is exact, not a degradation.
        assert_eq!(rep.completeness, Completeness::Complete);
        let polygon = rep
            .fragments
            .iter()
            .find_map(|f| match &f.primitive {
                DisplayPrimitive::Image { clip, .. } => clip.clone(),
                _ => None,
            })
            .expect("the intersected image carries its polygon");
        assert_eq!(polygon.len(), 4);
        let max_v = polygon.iter().map(|v| v.uv[1]).fold(f64::MIN, f64::max);
        assert!(
            (max_v - 0.1).abs() < 1e-9,
            "the entity clip's v=0.1 edge must survive, not the full quad's 0.125: {max_v}"
        );
        // The viewport cut still applies: no vertex escapes the window.
        for v in polygon.iter() {
            assert!(v.position.x <= 100.0 + 1e-9, "{:?}", v.position);
            assert!(v.position.y <= 50.0 + 1e-9, "{:?}", v.position);
        }
    }

    #[test]
    fn paper_build_keeps_a_non_convex_entity_clip_and_reports_it() {
        // A concave "chevron" in model space; a provider contract violation, but
        // the paper build must not silently widen or drop it.
        let clip: std::sync::Arc<[ImageVertex]> = std::sync::Arc::from(
            vec![
                ImageVertex {
                    position: p(10.0, 20.0),
                    uv: [0.0, 0.0],
                },
                ImageVertex {
                    position: p(20010.0, 20.0),
                    uv: [1.0, 0.0],
                },
                ImageVertex {
                    position: p(10010.0, 10020.0),
                    uv: [0.5, 0.5],
                },
                ImageVertex {
                    position: p(20010.0, 20020.0),
                    uv: [1.0, 1.0],
                },
                ImageVertex {
                    position: p(10.0, 20020.0),
                    uv: [0.0, 1.0],
                },
            ]
            .into_boxed_slice(),
        );
        let db = clipped_image_layout();
        let mut registry = ProviderRegistry::with_default_provider();
        registry
            .register(Box::new(ClippedImageProvider { clip }))
            .unwrap();
        let rep = build_paper_space(&registry, &db, LayoutId(1), &context(), &|_| true).unwrap();
        assert!(matches!(rep.completeness, Completeness::Partial(_)));
        assert!(rep.diagnostics.iter().any(|d| {
            d.code == "representation.viewport_clip_partial"
                && d.message
                    .contains(clip_reason::IMAGE_INTERSECTION_UNSUPPORTED)
        }));
        // The entity clip is preserved unchanged (never widened to the window).
        let polygon = rep
            .fragments
            .iter()
            .find_map(|f| match &f.primitive {
                DisplayPrimitive::Image { clip, .. } => clip.clone(),
                _ => None,
            })
            .expect("the entity clip is kept");
        assert_eq!(polygon.len(), 5);
    }
}
