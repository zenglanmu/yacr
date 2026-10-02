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
//! A [`PaperViewport`] carries three clip points (two *opposite* paper corners
//! plus the model *view center*) and a scale that is stored as model-units per
//! paper-unit and must be **inverted** to go model → paper. Interpreting that
//! safely is the whole job here: the transform is derived from the viewport's
//! own rectangle and scale, and any viewport whose state we cannot reconstruct
//! exactly is reported as [`ViewportState::Unsupported`] rather than drawn at a
//! guessed ratio. Paper output that would be wrong is never silently
//! substituted.
//!
//! Note (audit B22, import-side gap): the current importer stores two *adjacent*
//! bottom corners and drops the paper height, so its output is refused here as
//! non-opposite. That is deliberate; the import workstream must store opposite
//! corners, after which this layer draws it. See `docs/layouts.md` §3.1.
//!
//! ## Coordinate convention
//!
//! A supported viewport defines an axis-aligned rectangle on the paper sheet
//! (centre `c`, half-extent `h`) and a uniform model→paper scale `s`:
//!
//! ```text
//! paper = (model - anchor) * s + c
//! ```
//!
//! where `anchor` is the model-space point that lands on the paper centre.
//! `s` is *paper units per model unit* and is positive; a non-uniform, rotated,
//! mirrored or singular view transform is unsupported here. Model fragments are
//! then clipped to the rectangle `[c - h, c + h]`.
//!
//! Paper-space measurement remains disabled in `cad-measure`; nothing in this
//! module re-enables it. See `docs/layouts.md`.

use crate::{
    DisplayFragment, DisplayPrimitive, DisplayRepresentation, ProviderRegistry,
    RepresentationContext,
};
use cad_db::{DbEntity, DrawingDatabase, PaperViewport};
use cad_domain::*;

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
/// can report it instead of showing a wrong or empty sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDescriptor {
    pub id: LayoutId,
    pub name: String,
    /// True when every viewport in the layout is drawable by this build.
    pub supported: bool,
    /// Empty when `supported`; otherwise the first unsupported reason.
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
                        reason = why;
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

/// The result of interpreting one stored [`PaperViewport`].
#[derive(Debug, Clone, PartialEq)]
pub enum ViewportState {
    /// A drawable rectangular viewport with an exact transform.
    Supported(ViewportTransform),
    /// The viewport cannot be drawn correctly by this build; the string names
    /// the concrete cause. Never drawn at a guessed ratio.
    Unsupported(String),
}

/// An exact model→paper mapping for one rectangular viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportTransform {
    /// Paper-space rectangle centre.
    pub paper_center: [f64; 2],
    /// Paper-space rectangle half-extent (both components strictly positive).
    pub paper_half: [f64; 2],
    /// Model-space point that maps to `paper_center`.
    pub model_anchor: Point3,
    /// Paper units per model unit (strictly positive, uniform).
    pub paper_per_model: f64,
}

impl ViewportTransform {
    /// Map a model-space point onto the paper sheet.
    ///
    /// The z component is carried through unchanged so 2D paper output keeps
    /// the viewport's plane; clipping uses only x/y.
    pub fn model_to_paper(&self, p: Point3) -> Point3 {
        let s = self.paper_per_model;
        Point3 {
            x: (p.x - self.model_anchor.x) * s + self.paper_center[0],
            y: (p.y - self.model_anchor.y) * s + self.paper_center[1],
            z: p.z,
        }
    }

    /// The inverse paper→model mapping, when it is needed (paper measurement is
    /// still refused elsewhere; this exists so a future verified inverse has a
    /// single source).
    pub fn paper_to_model(&self, p: Point3) -> Point3 {
        let s = self.paper_per_model;
        Point3 {
            x: (p.x - self.paper_center[0]) / s + self.model_anchor.x,
            y: (p.y - self.paper_center[1]) / s + self.model_anchor.y,
            z: p.z,
        }
    }

    /// Express the mapping as a [`Transform3`] so existing primitive transforms
    /// can be reused (uniform scale + translation, z unchanged).
    pub fn as_transform3(&self) -> Transform3 {
        let mut m = Transform3::identity().matrix;
        let s = self.paper_per_model;
        m[0][0] = s;
        m[1][1] = s;
        m[0][3] = self.paper_center[0] - self.model_anchor.x * s;
        m[1][3] = self.paper_center[1] - self.model_anchor.y * s;
        Transform3 { matrix: m }
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

/// Interpret a stored viewport exactly, or refuse it.
///
/// Accepted shape (the shape the importer writes): `clip` holds three finite
/// points `[corner_a, corner_b, model_view_center]`; `corner_a`/`corner_b` are
/// opposite corners of the axis-aligned paper rectangle and `model_view_center`
/// is the model point shown at the rectangle centre. `model_to_paper` must be a
/// pure uniform positive scale about the origin (no rotation, shear, mirror or
/// translation); its value is model-units-per-paper-unit and is inverted here.
///
/// Everything else — a non-rectangular or complex clip, a rotated/mirrored/
/// non-uniform/singular transform, a degenerate rectangle, non-finite data, a
/// clip that is not three points — is `Unsupported` with a specific reason.
pub fn viewport_transform(viewport: &PaperViewport) -> ViewportState {
    // A complex (non-rectangular) clip is already flagged by the importer; trust
    // that report but require the exact geometry shape below regardless.
    if let Completeness::Partial(reasons) = &viewport.completeness {
        let why = reasons
            .first()
            .cloned()
            .unwrap_or_else(|| "partial viewport state".to_string());
        return ViewportState::Unsupported(format!("viewport is not fully supported: {why}"));
    }

    if viewport.clip.len() != 3 {
        return ViewportState::Unsupported(format!(
            "viewport clip must be three points (two paper corners + model centre), found {}",
            viewport.clip.len()
        ));
    }
    let corner_a = viewport.clip[0];
    let corner_b = viewport.clip[1];
    let model_anchor = viewport.clip[2];
    if !finite_point(&corner_a) || !finite_point(&corner_b) || !finite_point(&model_anchor) {
        return ViewportState::Unsupported("viewport clip has non-finite coordinates".to_string());
    }

    // The two corners must span a non-degenerate axis-aligned rectangle. Two
    // adjacent corners (sharing one coordinate) are rejected explicitly: the
    // other dimension is not encoded anywhere in `PaperViewport`, so the
    // rectangle cannot be reconstructed and must not be guessed.
    let width = (corner_b.x - corner_a.x).abs();
    let height = (corner_b.y - corner_a.y).abs();
    let adjacent = width <= 1e-12 || height <= 1e-12;
    if adjacent {
        return ViewportState::Unsupported(format!(
            "viewport clip corners are adjacent, not opposite (width {width}, height {height}); \
             the missing paper dimension is not recoverable from the stored viewport"
        ));
    }
    let paper_center = [
        (corner_a.x + corner_b.x) * 0.5,
        (corner_a.y + corner_b.y) * 0.5,
    ];
    let paper_half = [width * 0.5, height * 0.5];

    // `model_to_paper` must be a pure uniform positive scale.
    let m = &viewport.model_to_paper.matrix;
    let model_units_per_paper = m[0][0];
    if !model_units_per_paper.is_finite() || model_units_per_paper <= 0.0 {
        return ViewportState::Unsupported(format!(
            "viewport scale must be a finite positive number, found {model_units_per_paper}"
        ));
    }
    if !viewport.model_to_paper.is_uniform_scale(1e-9) {
        return ViewportState::Unsupported(
            "viewport transform is not a uniform scale (rotation, shear or mirror is unsupported)"
                .to_string(),
        );
    }
    // `is_uniform_scale` accepts a pure rotation or a mirror, which this build
    // does not draw. Require a *pure* scale: off-diagonal terms zero and an
    // equal, positive diagonal.
    let pure_scale = m[0][1].abs() < 1e-12
        && m[0][2].abs() < 1e-12
        && m[1][0].abs() < 1e-12
        && m[1][2].abs() < 1e-12
        && m[2][0].abs() < 1e-12
        && m[2][1].abs() < 1e-12
        && (m[1][1] - model_units_per_paper).abs() < 1e-9 * model_units_per_paper;
    if !pure_scale {
        return ViewportState::Unsupported(
            "viewport transform is rotated, sheared or non-uniform; only a pure scale is supported"
                .to_string(),
        );
    }

    // Correct the stored direction: the field is model-units-per-paper-unit, but
    // the paper mapping needs paper-units-per-model-unit.
    let paper_per_model = 1.0 / model_units_per_paper;
    ViewportState::Supported(ViewportTransform {
        paper_center,
        paper_half,
        model_anchor,
        paper_per_model,
    })
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

/// Build the display representation for one paper-space layout (spec §3.3, F04).
///
/// The result contains, in this order:
///
/// 1. the layout's own paper-space entities, drawn as-is;
/// 2. for each **supported** rectangular viewport, the model-space entities
///    mapped onto the sheet and clipped to the viewport rectangle.
///
/// A viewport whose state cannot be drawn exactly contributes no geometry and a
/// `Missing`/`Partial` report plus a `representation.viewport_unsupported`
/// diagnostic naming the cause — never a guessed transform. A layout that does
/// not exist is `Missing`. `visible` is the caller's layer-visibility predicate
/// (kept as a closure so this crate stays independent of `cad-app`).
///
/// Rectangular clipping is exact for line/fill geometry; meshes and text are
/// transformed but only flagged `Partial` (their per-triangle clip is a
/// documented remaining gap — see `docs/layouts.md`), never claimed as clipped.
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
            ViewportState::Unsupported(reason) => {
                out.completeness = weaker(
                    &out.completeness,
                    &Completeness::Missing(vec![reason.clone()]),
                );
                out.diagnostics.push(Diagnostic {
                    object: None,
                    code: "representation.viewport_unsupported".into(),
                    message: format!("layout {} viewport {index}: {reason}", layout_id.0),
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
                    DisplayPrimitive::Lines(points) => {
                        for run in clip_polyline_to_rect(&points, center, half) {
                            out.fragments.push(DisplayFragment {
                                source: fragment.source.clone(),
                                geometry_source: fragment.geometry_source.clone(),
                                alpha: fragment.alpha,
                                primitive: DisplayPrimitive::Lines(std::sync::Arc::from(
                                    run.into_boxed_slice(),
                                )),
                            });
                        }
                    }
                    other => {
                        // Rectangular clipping is only exact for line geometry
                        // here; report the limitation instead of pretending.
                        out.completeness = weaker(
                            &out.completeness,
                            &Completeness::Partial(vec![
                                "viewport clip applied to line geometry only".into(),
                            ]),
                        );
                        out.fragments.push(DisplayFragment {
                            source: fragment.source,
                            geometry_source: fragment.geometry_source,
                            alpha: fragment.alpha,
                            primitive: other,
                        });
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

    /// The importer's shape: two opposite paper corners + the model view centre,
    /// with a model-units-per-paper-unit scale.
    fn simple_viewport() -> PaperViewport {
        // Paper rectangle (0,0)-(100,50), model centre at (10, 20), 1:100 => the
        // stored ratio is 100 model units per paper unit.
        viewport(vec![p(0.0, 0.0), p(100.0, 50.0), p(10.0, 20.0)], 100.0)
    }

    #[test]
    fn supported_viewport_derives_centre_half_and_inverted_scale() {
        let ViewportState::Supported(t) = viewport_transform(&simple_viewport()) else {
            panic!("expected a supported viewport");
        };
        assert_eq!(t.paper_center, [50.0, 25.0]);
        assert_eq!(t.paper_half, [50.0, 25.0]);
        assert_eq!(t.model_anchor, p(10.0, 20.0));
        // The stored 100 is model-units-per-paper-unit; the mapping uses the
        // inverse (audit B22 "方向需纠正").
        assert_eq!(t.paper_per_model, 0.01);
    }

    #[test]
    fn model_to_paper_places_the_anchor_at_the_centre() {
        let ViewportState::Supported(t) = viewport_transform(&simple_viewport()) else {
            panic!("expected a supported viewport");
        };
        // The model anchor lands on the paper centre...
        assert_eq!(t.model_to_paper(p(10.0, 20.0)), p(50.0, 25.0));
        // ...and one paper unit is 100 model units: 1 paper unit right of centre
        // is model x = 110.
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

    #[test]
    fn complex_clip_is_refused_with_its_reason() {
        let mut vp = simple_viewport();
        vp.completeness = Completeness::Partial(vec!["complex viewport clip".into()]);
        match viewport_transform(&vp) {
            ViewportState::Unsupported(reason) => assert!(reason.contains("complex viewport clip")),
            ViewportState::Supported(_) => panic!("complex clip must be refused"),
        }
    }

    #[test]
    fn missing_scale_is_refused() {
        let vp = viewport(vec![p(0.0, 0.0), p(10.0, 10.0), p(0.0, 0.0)], 0.0);
        match viewport_transform(&vp) {
            ViewportState::Unsupported(reason) => assert!(reason.contains("scale")),
            ViewportState::Supported(_) => panic!("zero scale must be refused"),
        }
    }

    #[test]
    fn rotated_view_transform_is_refused() {
        let mut vp = simple_viewport();
        // A 45° rotation about Z is a valid transform but not one we draw.
        let c = std::f64::consts::FRAC_1_SQRT_2;
        vp.model_to_paper = Transform3 {
            matrix: [
                [c, -c, 0.0, 0.0],
                [c, c, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        assert!(matches!(
            viewport_transform(&vp),
            ViewportState::Unsupported(_)
        ));
    }

    #[test]
    fn adjacent_clip_corners_are_refused_as_not_opposite() {
        // The importer currently stores the two bottom corners (same y); the
        // paper height is not stored, so this must be refused, not guessed.
        let vp = viewport(vec![p(0.0, 0.0), p(10.0, 0.0), p(0.0, 0.0)], 100.0);
        match viewport_transform(&vp) {
            ViewportState::Unsupported(reason) => assert!(reason.contains("adjacent")),
            ViewportState::Supported(_) => panic!("adjacent corners must be refused"),
        }
    }

    #[test]
    fn zero_size_rectangle_is_refused() {
        let vp = viewport(vec![p(0.0, 0.0), p(0.0, 0.0), p(0.0, 0.0)], 1.0);
        assert!(matches!(
            viewport_transform(&vp),
            ViewportState::Unsupported(_)
        ));
    }

    #[test]
    fn non_three_point_clip_is_refused() {
        let vp = viewport(vec![p(0.0, 0.0), p(10.0, 10.0)], 1.0);
        assert!(matches!(
            viewport_transform(&vp),
            ViewportState::Unsupported(_)
        ));
    }

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
        // In at x=-1, out at x=1 through (0,0): a single continuous run of 3.
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
        // A "U": two inside stubs joined by a segment fully outside (y=5). The
        // outside segment breaks the result into two separate runs.
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
            viewports: vec![simple_viewport()],
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
        assert!(layouts[1].reason.contains("scale"));
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

    fn context() -> RepresentationContext {
        RepresentationContext::new(
            DocumentId(1),
            TolerancePolicy::default(),
            TaskStamp::new(DocumentId(1), 0),
        )
    }

    #[test]
    fn paper_build_maps_and_clips_model_geometry_through_the_viewport() {
        // Model (0,0)-(1,0) is far outside a 1:100 window centred on (10,20), so
        // it is clipped away; a model line through the anchor survives and lands
        // on the paper.
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
    fn paper_build_reports_an_unsupported_viewport_and_draws_no_guess() {
        let db = db_with_layouts();
        let registry = ProviderRegistry::with_default_provider();
        // Layout2's only viewport has a zero scale: unsupported.
        let rep = build_paper_space(&registry, &db, LayoutId(2), &context(), &|_| true).unwrap();
        assert!(matches!(rep.completeness, Completeness::Missing(_)));
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
}
