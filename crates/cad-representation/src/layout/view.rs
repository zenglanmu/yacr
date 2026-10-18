use cad_db::PaperViewport;
use cad_domain::{Completeness, Point3, Transform3};

use super::selection::{ViewportState, ViewportUnsupported};
use super::viewport_reason;

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

pub(super) fn finite_point(p: &Point3) -> bool {
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
