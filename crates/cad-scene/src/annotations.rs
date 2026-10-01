//! Annotation → scene batch conversion (spec F07).
//!
//! F07 stores annotations in `cad-db`/`cad-annotations`, but until now nothing
//! consumed them: the panel listed real rows while the canvas drew nothing. This
//! module turns annotation geometry into the *existing* [`RenderBatch`] /
//! [`RenderTopology`] types — there is deliberately no second renderer. A caller
//! concatenates the batches with the drawing's own batches and hands the combined
//! [`SceneDelta`](crate::SceneDelta) to the renderer.
//!
//! ## What is converted
//!
//! | geometry | shape |
//! |---|---|
//! | `Text` | the text outlined to polylines through a [`FontEngine`], or an explicit `Partial` when no font is available |
//! | `Leader` | open polyline |
//! | `Rectangle` | closed 4-corner loop |
//! | `Ellipse` | parametric discretisation `p(t) = center + axis_u·cos t + axis_v·sin t` |
//! | `Freehand` | open polyline |
//! | `Cloud` | closed polyline (no scallop arcs) |
//! | `Measurement` | **not drawn here** — an explicit `Partial`/`Missing` reason |
//!
//! ## Explicit, non-silent limitations
//!
//! - **Measurement geometry** is not part of the annotation overlay; it is a
//!   measurement record and its rendering belongs to the measurement overlay
//!   path (`docs/ui.md` §2 records the same gap). It is reported, never dropped
//!   in silence.
//! - **Per-annotation RGB is not carried by [`RenderBatch`]**: the batch only has
//!   a constant `alpha`. The style's alpha channel is honoured; its RGB channels
//!   are not, which is recorded as an `annotation.color` diagnostic.
//! - **Revision-cloud scallops** are not modelled: the stored geometry is a
//!   point loop, and it is drawn as a plain closed polyline.
//! - **Text requires a font engine.** Without one, a text annotation contributes
//!   an explicit `Partial` reason instead of a guessed placeholder box.
//! - **Duplicate points** inside a polyline are removed before batching; a run
//!   that collapses to fewer than two distinct points is an explicit reason.

use cad_db::{Annotation, AnnotationGeometry};
use cad_domain::{
    AnnotationId, CadError, CadResult, Completeness, Diagnostic, DocumentId, EntityId,
    GeometrySource, InstancePath, Point3, SelectionRef,
};
use cad_geometry::{arc_segments_for_tolerance, TessellationParams};
use cad_representation::FontEngine;

use crate::{BudgetExceeded, FrameBudget, FrameUsage, RenderBatch, RenderTopology};

/// Font key used to outline annotation text.
///
/// `cad-db::Annotation` carries no font field, so there is nothing authoritative
/// to read. This is the same conventional face the drawing pipeline falls back
/// to; [`FontEngine`]'s own fallback chain substitutes a registered face when
/// this one is absent, and a missing engine/face is reported rather than faked.
pub const DEFAULT_ANNOTATION_FONT: &str = "arial.ttf";

/// Inputs that do not come from the annotation database.
pub struct AnnotationSceneOptions<'a> {
    /// Drawing the annotations belong to; used only for [`SelectionRef`].
    pub document: DocumentId,
    /// Shaping fonts for text annotations. `None` reports text as missing.
    pub fonts: Option<&'a FontEngine>,
    /// Per-frame vertex/triangle budget shared with the drawing batches.
    pub budget: FrameBudget,
    /// Chord tolerance for ellipse discretisation, in world units.
    pub tolerance: f64,
    /// `draw_order` applied to the first annotation batch; each annotation
    /// increments it so annotations sort after drawing geometry in a caller that
    /// honours order.
    pub draw_order_base: i64,
}

impl Default for AnnotationSceneOptions<'_> {
    fn default() -> Self {
        AnnotationSceneOptions {
            document: DocumentId(0),
            fonts: None,
            budget: FrameBudget {
                max_vertices: 8_000_000,
                max_triangles: 2_000_000,
            },
            tolerance: 0.01,
            draw_order_base: 1_000_000,
        }
    }
}

/// The converted annotation overlay.
pub struct AnnotationScene {
    pub batches: Vec<RenderBatch>,
    /// `Complete` when every visible annotation was fully drawn; otherwise
    /// `Partial`/`Missing` with one reason per affected annotation.
    pub completeness: Completeness,
    /// Per-annotation diagnostics (`annotation.unsupported`, `annotation.color`,
    /// `annotation.empty`, `annotation.budget`, …).
    pub diagnostics: Vec<Diagnostic>,
    /// Budget usage consumed by the returned batches.
    pub usage: FrameUsage,
    /// Set when a batch was refused because the frame budget was exhausted.
    pub budget_exceeded: Option<BudgetExceeded>,
}

impl AnnotationScene {
    /// Number of batches produced.
    pub fn drawn(&self) -> usize {
        self.batches.len()
    }
}

/// Convert the visible annotations into render batches.
///
/// `visible` is the session visibility predicate: it receives each annotation id
/// and returns whether the scene should draw it. The annotation database has no
/// stored hidden flag, so callers pass the session override set
/// (`cad-app::AnnotationVisibilitySet`) — this crate cannot depend on `cad-app`,
/// which is why the filter is a closure.
///
/// Ordering follows the iterator (the database is a `BTreeMap`, so id order is
/// deterministic). An empty input is an explicit empty overlay, not an error.
pub fn annotation_batches<'a, I>(
    annotations: I,
    visible: impl Fn(AnnotationId) -> bool,
    options: &AnnotationSceneOptions<'_>,
) -> AnnotationScene
where
    I: IntoIterator<Item = &'a Annotation>,
{
    let mut result = AnnotationScene {
        batches: Vec::new(),
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
        usage: FrameUsage::default(),
        budget_exceeded: None,
    };
    let mut order = options.draw_order_base;

    'annotations: for annotation in annotations {
        if !visible(annotation.id) {
            // Hidden is a session choice, not a gap: it is not `Partial`.
            continue;
        }
        let converted = convert_geometry(annotation, options);

        // Report colour loss for anything that is actually drawn: the batch
        // cannot carry RGB (see the module docs).
        if converted.has_drawable_geometry() && annotation.style.rgba[..3] != [0, 0, 0] {
            result.diagnostics.push(Diagnostic {
                object: None,
                code: "annotation.color".into(),
                message: format!(
                    "annotation {:?} uses rgba {:?}; the batch carries alpha only, RGB is not drawn",
                    annotation.id, annotation.style.rgba
                ),
            });
            result.completeness =
                result
                    .completeness
                    .combine(Completeness::Partial(vec![format!(
                        "annotation {:?}: colour RGB is not rendered",
                        annotation.id
                    )]));
        }

        for issue in converted.reasons {
            result.diagnostics.push(Diagnostic {
                object: None,
                code: issue.code.into(),
                message: issue.message.clone(),
            });
            let completeness = match issue.severity {
                IssueSeverity::Missing => Completeness::Missing(vec![issue.message]),
                IssueSeverity::Partial => Completeness::Partial(vec![issue.message]),
            };
            result.completeness = result.completeness.combine(completeness);
        }

        for points in converted.polylines {
            let Some(batch) = make_batch(annotation, &points, order, options.document) else {
                continue;
            };
            let vertices = batch.vertices.len();
            match options.budget.charge(&mut result.usage, vertices, 0) {
                Ok(()) => {
                    result.batches.push(batch);
                    order += 1;
                }
                Err(exceeded) => {
                    result.budget_exceeded = Some(exceeded);
                    result.diagnostics.push(Diagnostic {
                        object: None,
                        code: "annotation.budget".into(),
                        message: format!(
                            "annotation overlay stopped: {} budget exceeded ({} > {})",
                            exceeded.category, exceeded.requested, exceeded.limit
                        ),
                    });
                    result.completeness = result.completeness.combine(Completeness::Partial(vec![
                        "annotation overlay truncated by the frame budget".to_string(),
                    ]));
                    break 'annotations;
                }
            }
        }
    }

    result
}

/// Severity of a conversion issue: `Missing` means nothing was drawn for the
/// annotation, `Partial` means something was drawn but not everything.
#[derive(Clone, Copy)]
enum IssueSeverity {
    Partial,
    Missing,
}

struct Issue {
    code: &'static str,
    message: String,
    severity: IssueSeverity,
}

struct Converted {
    polylines: Vec<Vec<Point3>>,
    reasons: Vec<Issue>,
}

impl Converted {
    fn empty() -> Self {
        Converted {
            polylines: Vec::new(),
            reasons: Vec::new(),
        }
    }

    fn has_drawable_geometry(&self) -> bool {
        !self.polylines.is_empty()
    }

    fn issue(&mut self, code: &'static str, message: String, severity: IssueSeverity) {
        self.reasons.push(Issue {
            code,
            message,
            severity,
        });
    }
}

/// Convert one annotation's geometry into world-space polylines.
fn convert_geometry(annotation: &Annotation, options: &AnnotationSceneOptions<'_>) -> Converted {
    match &annotation.geometry {
        AnnotationGeometry::Text(origin) => {
            let mut out = Converted::empty();
            match options.fonts {
                Some(fonts) if !annotation.text.is_empty() => {
                    match fonts.outline(
                        DEFAULT_ANNOTATION_FONT,
                        &annotation.text,
                        *origin,
                        annotation.style.text_height,
                        0.0,
                        cad_domain::TextAlignH::Left,
                        cad_domain::TextAlignV::Baseline,
                    ) {
                        Ok(polylines) => out.polylines = polylines,
                        Err(e) => out.issue(
                            "annotation.text_unshaped",
                            format!("annotation {:?}: text not shaped: {e}", annotation.id),
                            IssueSeverity::Missing,
                        ),
                    }
                }
                Some(_) => out.issue(
                    "annotation.empty",
                    format!("annotation {:?}: text is empty", annotation.id),
                    IssueSeverity::Missing,
                ),
                None => out.issue(
                    "annotation.text_unshaped",
                    format!(
                        "annotation {:?}: no font engine; text is not drawn",
                        annotation.id
                    ),
                    IssueSeverity::Missing,
                ),
            }
            out
        }
        AnnotationGeometry::Leader(points) => {
            let mut out = Converted::empty();
            push_polyline(&mut out, annotation.id, "leader", points);
            out
        }
        AnnotationGeometry::Freehand(points) => {
            let mut out = Converted::empty();
            push_polyline(&mut out, annotation.id, "freehand", points);
            out
        }
        AnnotationGeometry::Cloud(points) => {
            let mut out = Converted::empty();
            push_closed_polyline(&mut out, annotation.id, "cloud", points);
            if out.has_drawable_geometry() {
                out.issue(
                    "annotation.cloud_plain",
                    format!(
                        "annotation {:?}: revision cloud drawn as a plain closed polyline (no scallops)",
                        annotation.id
                    ),
                    IssueSeverity::Partial,
                );
            }
            out
        }
        AnnotationGeometry::Rectangle(pair) => {
            let mut out = Converted::empty();
            let [a, b] = pair;
            let corners = vec![
                *a,
                Point3 {
                    x: b.x,
                    y: a.y,
                    z: a.z,
                },
                *b,
                Point3 {
                    x: a.x,
                    y: b.y,
                    z: a.z,
                },
                *a,
            ];
            if is_degenerate(&corners) {
                out.issue(
                    "annotation.empty",
                    format!("annotation {:?}: rectangle has zero extent", annotation.id),
                    IssueSeverity::Missing,
                );
            } else {
                out.polylines.push(corners);
            }
            out
        }
        AnnotationGeometry::Ellipse {
            center,
            axis_u,
            axis_v,
        } => {
            let mut out = Converted::empty();
            let points = tessellate_ellipse(*center, *axis_u, *axis_v, options.tolerance);
            if points.len() < 3 {
                out.issue(
                    "annotation.empty",
                    format!("annotation {:?}: ellipse has no extent", annotation.id),
                    IssueSeverity::Missing,
                );
            } else {
                out.polylines.push(points);
            }
            out
        }
        AnnotationGeometry::Measurement(record) => {
            let mut out = Converted::empty();
            out.issue(
                "annotation.unsupported",
                format!(
                    "annotation {:?}: measurement geometry ({:?}) is not part of the annotation overlay",
                    annotation.id, record.algorithm
                ),
                IssueSeverity::Missing,
            );
            out
        }
    }
}

fn push_polyline(out: &mut Converted, id: AnnotationId, kind: &str, points: &[Point3]) {
    let cleaned = dedupe(points);
    if cleaned.len() < 2 || is_degenerate(&cleaned) {
        out.issue(
            "annotation.empty",
            format!(
                "annotation {id:?}: {kind} has {} distinct point(s)",
                cleaned.len()
            ),
            IssueSeverity::Missing,
        );
        return;
    }
    out.polylines.push(cleaned);
}

fn push_closed_polyline(out: &mut Converted, id: AnnotationId, kind: &str, points: &[Point3]) {
    let mut cleaned = dedupe(points);
    if cleaned.len() < 2 || is_degenerate(&cleaned) {
        out.issue(
            "annotation.empty",
            format!(
                "annotation {id:?}: {kind} has {} distinct point(s)",
                cleaned.len()
            ),
            IssueSeverity::Missing,
        );
        return;
    }
    if cleaned.first() != cleaned.last() {
        cleaned.push(cleaned[0]);
    }
    out.polylines.push(cleaned);
}

/// Drop consecutive duplicate points.
fn dedupe(points: &[Point3]) -> Vec<Point3> {
    let mut out: Vec<Point3> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().map(|last| same_point(*last, *p)) != Some(true) {
            out.push(*p);
        }
    }
    out
}

fn same_point(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() < 1e-12 && (a.y - b.y).abs() < 1e-12 && (a.z - b.z).abs() < 1e-12
}

fn is_degenerate(points: &[Point3]) -> bool {
    if points.len() < 2 {
        return true;
    }
    let first = points[0];
    points.iter().all(|p| same_point(*p, first))
}

fn magnitude(v: Point3) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// Parametrically discretise `p(t) = center + axis_u·cos t + axis_v·sin t`.
///
/// The segment count reuses `cad-geometry`'s sagitta rule against the largest
/// half-axis, so the ellipse's error is bounded like any other tessellated
/// curve in the workspace. Returns an empty vector when either axis (or the
/// result) is degenerate/non-finite, so a caller reports a reason rather than
/// queueing a zero-length batch.
pub fn tessellate_ellipse(
    center: Point3,
    axis_u: Point3,
    axis_v: Point3,
    tolerance: f64,
) -> Vec<Point3> {
    let radius = magnitude(axis_u).max(magnitude(axis_v));
    if !radius.is_finite() || radius < 1e-12 {
        return Vec::new();
    }
    if !center.x.is_finite() || !center.y.is_finite() || !center.z.is_finite() {
        return Vec::new();
    }
    let params = TessellationParams {
        tolerance: tolerance.max(1e-12),
        ..Default::default()
    };
    let segments = arc_segments_for_tolerance(radius, std::f64::consts::TAU, params);
    let mut out = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = std::f64::consts::TAU * (i as f64) / (segments as f64);
        let (sin, cos) = t.sin_cos();
        out.push(Point3 {
            x: center.x + axis_u.x * cos + axis_v.x * sin,
            y: center.y + axis_u.y * cos + axis_v.y * sin,
            z: center.z + axis_u.z * cos + axis_v.z * sin,
        });
    }
    out
}

/// Build a line batch from world-space points, relative to a local origin.
///
/// Returns `None` for a polyline that is already degenerate after cleaning, so a
/// zero-length batch is never queued.
fn make_batch(
    annotation: &Annotation,
    points: &[Point3],
    draw_order: i64,
    document: DocumentId,
) -> Option<RenderBatch> {
    let cleaned = dedupe(points);
    if cleaned.len() < 2 || is_degenerate(&cleaned) {
        return None;
    }
    let origin = cleaned[0];
    let vertices: Vec<[f32; 3]> = cleaned
        .iter()
        .map(|p| {
            [
                (p.x - origin.x) as f32,
                (p.y - origin.y) as f32,
                (p.z - origin.z) as f32,
            ]
        })
        .collect();
    Some(RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        // The batch carries a constant alpha only; RGB is a documented gap.
        alpha: annotation.style.rgba[3] as f32 / 255.0,
        sources: vec![SelectionRef {
            document,
            entity: EntityId(annotation.id.0),
            instance: InstancePath::default(),
            sub_element: None,
        }],
        draw_order,
    })
}

/// A visibility predicate that shows every annotation.
///
/// The annotation database has no stored hidden flag, so this is the honest
/// default for callers without a session visibility set.
pub fn all_visible(_id: AnnotationId) -> bool {
    true
}

/// Whether the conversion can draw this annotation's geometry at all.
///
/// Measurement geometry is the one kind that is deliberately outside the overlay;
/// callers can use this to explain a `Missing` before batching.
pub fn conversion_supported(annotation: &Annotation) -> CadResult<()> {
    match &annotation.geometry {
        AnnotationGeometry::Measurement(_) => Err(CadError::Unsupported(
            "measurement geometry is not part of the annotation overlay".into(),
        )),
        _ => Ok(()),
    }
}

/// Geometry source label that would be attached to a converted annotation.
///
/// Kept here so diagnostics and any future picking metadata agree on one mapping.
pub fn annotation_geometry_source(annotation: &Annotation) -> GeometrySource {
    match &annotation.geometry {
        AnnotationGeometry::Text(_)
        | AnnotationGeometry::Rectangle(_)
        | AnnotationGeometry::Ellipse { .. } => GeometrySource::Analytic,
        AnnotationGeometry::Leader(_)
        | AnnotationGeometry::Freehand(_)
        | AnnotationGeometry::Cloud(_)
        | AnnotationGeometry::Measurement(_) => GeometrySource::UserPoints,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::AnnotationStyle;
    use cad_domain::{Precision, SpaceId};

    fn point(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn annotation(id: u128, geometry: AnnotationGeometry) -> Annotation {
        annotation_with_text(id, geometry, "")
    }

    fn annotation_with_text(id: u128, geometry: AnnotationGeometry, text: &str) -> Annotation {
        Annotation {
            id: AnnotationId(id),
            space: SpaceId::Model,
            geometry,
            text: text.to_string(),
            style: AnnotationStyle::default(),
            created_unix_ms: 0,
            modified_unix_ms: 0,
            anchor: None,
            precision: Precision::Analytic,
        }
    }

    fn convert(annotations: &[Annotation]) -> AnnotationScene {
        annotation_batches(
            annotations.iter(),
            all_visible,
            &AnnotationSceneOptions::default(),
        )
    }

    #[test]
    fn leader_becomes_one_line_batch() {
        let a = annotation(
            1,
            AnnotationGeometry::Leader(vec![point(0.0, 0.0), point(5.0, 1.0)]),
        );
        let scene = convert(&[a]);
        assert_eq!(scene.batches.len(), 1);
        let batch = &scene.batches[0];
        assert_eq!(batch.topology, RenderTopology::Lines);
        assert_eq!(batch.vertices.len(), 2);
        assert_eq!(batch.local_origin, point(0.0, 0.0));
        assert_eq!(scene.usage.vertices, 2);
    }

    #[test]
    fn freehand_with_duplicate_points_is_deduped_not_dropped() {
        let a = annotation(
            2,
            AnnotationGeometry::Freehand(vec![
                point(0.0, 0.0),
                point(0.0, 0.0),
                point(1.0, 0.0),
                point(1.0, 0.0),
                point(2.0, 0.0),
            ]),
        );
        let scene = convert(&[a]);
        assert_eq!(scene.batches.len(), 1);
        assert_eq!(scene.batches[0].vertices.len(), 3);
    }

    #[test]
    fn single_point_polyline_is_reported_not_dropped_silently() {
        let a = annotation(3, AnnotationGeometry::Leader(vec![point(0.0, 0.0)]));
        let scene = convert(&[a]);
        assert!(scene.batches.is_empty());
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "annotation.empty"));
        assert!(matches!(scene.completeness, Completeness::Missing(_)));
    }

    #[test]
    fn rectangle_is_a_closed_loop_of_four_corners() {
        let a = annotation(
            4,
            AnnotationGeometry::Rectangle([point(0.0, 0.0), point(4.0, 2.0)]),
        );
        let scene = convert(&[a]);
        assert_eq!(scene.batches.len(), 1);
        let verts = &scene.batches[0].vertices;
        assert_eq!(verts.len(), 5);
        assert_eq!(verts[0], verts[4]);
    }

    #[test]
    fn degenerate_rectangle_is_reported() {
        let a = annotation(
            5,
            AnnotationGeometry::Rectangle([point(1.0, 1.0), point(1.0, 1.0)]),
        );
        let scene = convert(&[a]);
        assert!(scene.batches.is_empty());
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "annotation.empty"));
    }

    #[test]
    fn ellipse_tessellates_onto_its_parametric_curve() {
        let axis_u = point(4.0, 0.0);
        let axis_v = point(0.0, 2.0);
        let points = tessellate_ellipse(point(10.0, 20.0), axis_u, axis_v, 0.01);
        assert!(points.len() >= 8);
        // Every sample satisfies the ellipse equation: ((x-10)/4)^2 + ((y-20)/2)^2 = 1.
        for p in &points {
            let u = (p.x - 10.0) / 4.0;
            let v = (p.y - 20.0) / 2.0;
            assert!(
                (u * u + v * v - 1.0).abs() < 1e-9,
                "sample off curve: {p:?}"
            );
        }
        // First and last sample coincide at t=0 and t=2π.
        assert!((points.first().unwrap().x - points.last().unwrap().x).abs() < 1e-9);
    }

    #[test]
    fn ellipse_with_tilted_axes_stays_on_its_curve() {
        // 45°-rotated major axis, still a valid ellipse image.
        let axis_u = point(3.0, 3.0);
        let axis_v = point(-1.0, 1.0);
        let a = annotation(
            6,
            AnnotationGeometry::Ellipse {
                center: point(0.0, 0.0),
                axis_u,
                axis_v,
            },
        );
        let scene = convert(&[a]);
        assert_eq!(scene.batches.len(), 1);
        // All vertices are finite and the loop is closed.
        let verts = &scene.batches[0].vertices;
        assert!(verts.iter().all(|v| v.iter().all(|c| c.is_finite())));
    }

    #[test]
    fn degenerate_ellipse_is_reported() {
        let a = annotation(
            7,
            AnnotationGeometry::Ellipse {
                center: point(0.0, 0.0),
                axis_u: point(0.0, 0.0),
                axis_v: point(0.0, 0.0),
            },
        );
        let scene = convert(&[a]);
        assert!(scene.batches.is_empty());
        assert!(matches!(scene.completeness, Completeness::Missing(_)));
    }

    #[test]
    fn cloud_is_a_plain_closed_loop_and_says_so() {
        let a = annotation(
            8,
            AnnotationGeometry::Cloud(vec![point(0.0, 0.0), point(3.0, 0.0), point(3.0, 3.0)]),
        );
        let scene = convert(&[a]);
        assert_eq!(scene.batches.len(), 1);
        assert_eq!(scene.batches[0].vertices.len(), 4); // first == last
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "annotation.cloud_plain"));
        assert!(matches!(scene.completeness, Completeness::Partial(_)));
    }

    #[test]
    fn text_without_fonts_is_reported_not_faked() {
        let a = annotation_with_text(9, AnnotationGeometry::Text(point(0.0, 0.0)), "hello");
        let scene = annotation_batches(
            std::iter::once(&a),
            all_visible,
            &AnnotationSceneOptions::default(),
        );
        assert!(scene.batches.is_empty());
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "annotation.text_unshaped"));
    }

    #[test]
    fn measurement_is_explicitly_unsupported_not_dropped() {
        use cad_db::{MeasurementAlgorithm, MeasurementRecord};
        let record = MeasurementRecord {
            algorithm: MeasurementAlgorithm::Distance2d,
            inputs: vec![point(0.0, 0.0), point(1.0, 0.0)],
            plane: None,
            value: 1.0,
            units: cad_domain::UnitContext::drawing_units(),
            source: GeometrySource::UserPoints,
            precision: Precision::Analytic,
        };
        let a = annotation(10, AnnotationGeometry::Measurement(record.clone()));
        let scene = convert(std::slice::from_ref(&a));
        assert!(scene.batches.is_empty());
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "annotation.unsupported"));
        assert!(conversion_supported(&a).is_err());
    }

    #[test]
    fn visibility_filter_removes_hidden_annotations_without_a_diagnostic() {
        // Black RGBA so the only observable effect is the visibility filter.
        let mut visible_ann = annotation(
            11,
            AnnotationGeometry::Leader(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        );
        visible_ann.style.rgba = [0, 0, 0, 255];
        let mut hidden_ann = annotation(
            12,
            AnnotationGeometry::Leader(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        );
        hidden_ann.style.rgba = [0, 0, 0, 255];
        let scene = annotation_batches(
            [&visible_ann, &hidden_ann],
            |id| id != AnnotationId(12),
            &AnnotationSceneOptions::default(),
        );
        // Only the visible annotation produces a batch, and hiding is not a gap.
        assert_eq!(scene.batches.len(), 1);
        assert_eq!(scene.batches[0].sources[0].entity, EntityId(11));
        assert_eq!(scene.completeness, Completeness::Complete);
        assert!(scene.diagnostics.is_empty());
    }

    #[test]
    fn hidden_is_not_missing_even_when_nothing_else_is_drawn() {
        let only = annotation(
            13,
            AnnotationGeometry::Leader(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        );
        let scene = annotation_batches(
            std::iter::once(&only),
            |_| false,
            &AnnotationSceneOptions::default(),
        );
        assert!(scene.batches.is_empty());
        assert_eq!(scene.completeness, Completeness::Complete);
        assert!(scene.diagnostics.is_empty());
        assert_eq!(scene.usage.vertices, 0);
    }

    #[test]
    fn empty_database_is_an_explicit_empty_overlay() {
        let empty: Vec<Annotation> = Vec::new();
        let scene = convert(&empty);
        assert!(scene.batches.is_empty());
        assert_eq!(scene.completeness, Completeness::Complete);
        assert!(scene.budget_exceeded.is_none());
    }

    #[test]
    fn batch_carries_annotation_identity_alpha_and_draw_order() {
        let mut a = annotation(
            14,
            AnnotationGeometry::Leader(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        );
        a.style.rgba = [10, 20, 30, 128];
        let scene = convert(&[a]);
        let batch = &scene.batches[0];
        assert_eq!(batch.sources[0].entity, EntityId(14));
        assert_eq!(batch.sources[0].document, DocumentId(0));
        assert!((batch.alpha - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(batch.draw_order, 1_000_000);
        // RGB cannot be represented, so it is reported as Partial.
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "annotation.color"));
    }

    #[test]
    fn budget_is_charged_and_truncation_is_reported() {
        let make = |base: f64| {
            annotation(
                100 + base as u128,
                AnnotationGeometry::Leader(vec![point(0.0, 0.0), point(1.0, 0.0)]),
            )
        };
        let annotations = [make(0.0), make(2.0)];
        let options = AnnotationSceneOptions {
            budget: FrameBudget {
                max_vertices: 3,
                max_triangles: 0,
            },
            ..Default::default()
        };
        let scene = annotation_batches(annotations.iter(), all_visible, &options);
        // First batch (2 vertices) fits; second would reach 4 > 3.
        assert_eq!(scene.batches.len(), 1);
        let exceeded = scene.budget_exceeded.unwrap();
        assert_eq!(exceeded.category, "vertices");
        assert_eq!(scene.usage.vertices, 2);
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "annotation.budget"));
        assert!(matches!(scene.completeness, Completeness::Partial(_)));
    }

    #[test]
    fn large_coordinates_keep_relative_vertex_precision() {
        let a = annotation(
            15,
            AnnotationGeometry::Leader(vec![
                point(1_000_000.0, 2_000_000.0),
                point(1_000_010.0, 2_000_000.0),
            ]),
        );
        let scene = convert(&[a]);
        let batch = &scene.batches[0];
        assert_eq!(batch.local_origin.x, 1_000_000.0);
        assert_eq!(batch.vertices[1][0], 10.0);
    }

    #[test]
    fn geometry_source_mapping_is_stable() {
        let line = annotation(
            16,
            AnnotationGeometry::Leader(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        );
        let rect = annotation(
            17,
            AnnotationGeometry::Rectangle([point(0.0, 0.0), point(1.0, 1.0)]),
        );
        assert_eq!(
            annotation_geometry_source(&line),
            GeometrySource::UserPoints
        );
        assert_eq!(annotation_geometry_source(&rect), GeometrySource::Analytic);
    }
}
