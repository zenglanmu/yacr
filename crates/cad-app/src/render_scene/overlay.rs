//! Transient visual overlays: selection highlight and tool previews.
//!
//! This module produces the **derived** overlay batches a host draws on top of
//! the authoritative scene:
//!
//! * a **selection highlight** that resolves the exact `(entity, instance,
//!   sub-element)` refs in a [`SelectionSet`] against the database, expanding
//!   `INSERT`s exactly like [`crate::picking::drawing_pick_items`], so two
//!   placements of the same block highlight independently;
//! * a **tool preview** that turns an in-progress [`MeasurementPreview`] /
//!   [`AnnotationPreview`] into line geometry (a rubber-band chain, point
//!   crosses and, for rectangle/ellipse annotations, the shape being dragged).
//!
//! Nothing here mutates the database, the scene cache or history. Previews are
//! pure snapshots: a cancelled preview simply stops being passed in, and no
//! transaction is ever opened (see `cad-app::measure_tool` /
//! `cad-app::annotation_tool`).
//!
//! ## Reuse, not a second renderer
//!
//! Highlight batches are built by [`cad_scene::highlight_batches`], the one
//! existing highlight→[`RenderBatch`] converter, from `DisplayRepresentation`s
//! the default provider tessellates (the same discretisation the base scene
//! uses). The preview path reuses `cad_scene::tessellate_ellipse` for ellipse
//! previews. There is no second tessellator and no new renderer.
//!
//! ## Paint order
//!
//! | overlay | `draw_order` |
//! |---|---|
//! | base drawing (`SceneCache`) | `0` |
//! | selection highlight (`HIGHLIGHT_DRAW_ORDER`) | `900_000` |
//! | committed annotations (`AnnotationSceneOptions::draw_order_base`) | `1_000_000` |
//! | tool preview ([`PREVIEW_DRAW_ORDER`]) | `2_000_000` |
//!
//! A selection is a transient decoration below committed markup; a tool preview
//! is the live feedback the user is acting on, so it draws above everything.

use std::sync::Arc;

use cad_db::DrawingDatabase;
use cad_domain::{Completeness, Diagnostic, DocumentId, Point3, SelectionRef, TaskStamp};
use cad_representation::{
    DisplayRepresentation, FontEngine, ProviderRegistry, RepresentationContext,
};
use cad_scene::{HighlightOptions, HighlightScene, RenderBatch, RenderTopology};

use crate::annotation_tool::{AnnotationPreview, AnnotationToolKind};
use crate::measure_tool::MeasurementPreview;
use crate::selection::SelectionSet;
use crate::viewer_config::OverlayVisibility;

/// `draw_order` of the first tool-preview batch.
///
/// Above the committed annotation overlay (`1_000_000`) and above the selection
/// highlight (`HIGHLIGHT_DRAW_ORDER`, `900_000`), so the live tool feedback is
/// never hidden by markup while the user is drawing.
pub const PREVIEW_DRAW_ORDER: i64 = 2_000_000;

/// `draw_order` of the world-space axes and grid reference overlays.
///
/// Below the selection highlight (`HIGHLIGHT_DRAW_ORDER`, `900_000`) so a
/// selection and committed markup always read above the reference grid, and
/// above the base drawing (`0`).
pub const AXES_GRID_DRAW_ORDER: i64 = 800_000;

/// Safety cap on the total number of grid line segments in one batch.
///
/// The nice-step chooser targets roughly ten divisions per axis, but a
/// pathological extent could still ask for far more. The generator samples the
/// grid down to at most this many segments so the overlay batch stays bounded;
/// this is a guard, not the normal spacing.
pub const GRID_MAX_LINES: usize = 400;

/// Colour of the world X/Y axes.
pub const AXES_COLOR: [f32; 3] = [0.65, 0.67, 0.72];
/// Colour of the reference grid, deliberately dimmer than the axes.
pub const GRID_COLOR: [f32; 3] = [0.42, 0.45, 0.50];

/// Default half-extent of a point marker cross, in world units.
///
/// Markers are a screen affordance; without a camera this is a fixed world size
/// the host can override through [`PreviewOptions::marker_size`]. It is drawn
/// as two axis-aligned segments of length `2 * marker_size` per point.
pub const DEFAULT_PREVIEW_MARKER_SIZE: f64 = 0.5;

/// Preview colour: a cool cyan, distinct from the warm highlight tint.
pub const MEASUREMENT_PREVIEW_COLOR: [f32; 3] = [0.20, 0.85, 0.95];
/// Preview colour for annotation tools: magenta, distinct from highlight and
/// measurement.
pub const ANNOTATION_PREVIEW_COLOR: [f32; 3] = [0.95, 0.35, 0.85];

/// Paint parameters for the tool preview overlay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewOptions {
    /// `draw_order` of the first preview batch.
    pub draw_order: i64,
    /// Half-extent of each point marker cross, in world units.
    pub marker_size: f64,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        PreviewOptions {
            draw_order: PREVIEW_DRAW_ORDER,
            marker_size: DEFAULT_PREVIEW_MARKER_SIZE,
        }
    }
}

/// The transient highlight / preview overlay inputs a host supplies.
///
/// All fields are pure session state; none of them writes the database. An
/// empty selection and two `None` previews produce no overlay batches, which is
/// an explicit empty overlay rather than an error.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OverlayInputs {
    pub selection: SelectionSet,
    pub measurement: Option<MeasurementPreview>,
    pub annotation: Option<AnnotationPreview>,
    /// Which derived overlays the host wants drawn. Defaults to all-on
    /// ([`OverlayVisibility::default`]) so a caller that never manages overlay
    /// visibility still gets the historical behavior.
    pub visibility: OverlayVisibility,
}

impl OverlayInputs {
    /// Whether every overlay input is empty (no highlight, no preview).
    pub fn is_empty(&self) -> bool {
        self.selection.is_empty() && self.measurement.is_none() && self.annotation.is_none()
    }
}

/// The built transient overlay.
#[derive(Debug, Clone)]
pub struct VisualOverlay {
    pub batches: Vec<RenderBatch>,
    /// Per-selection/preview diagnostics (`highlight.*`, `preview.*`); never
    /// translated prose.
    pub diagnostics: Vec<Diagnostic>,
    pub completeness: Completeness,
}

impl Default for VisualOverlay {
    fn default() -> Self {
        VisualOverlay {
            batches: Vec::new(),
            diagnostics: Vec::new(),
            completeness: Completeness::Complete,
        }
    }
}

impl VisualOverlay {
    /// Number of overlay batches produced.
    pub fn drawn(&self) -> usize {
        self.batches.len()
    }

    /// Whether the overlay is empty.
    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    pub(crate) fn merge(&mut self, other: VisualOverlay) -> &mut Self {
        self.batches.extend(other.batches);
        self.diagnostics.extend(other.diagnostics);
        self.completeness = self.completeness.clone().combine(other.completeness);
        self
    }

    pub(crate) fn partial(&mut self, code: &'static str, message: String) {
        self.diagnostics.push(Diagnostic {
            object: None,
            code: code.into(),
            message: message.clone(),
        });
        self.completeness = self
            .completeness
            .clone()
            .combine(Completeness::Partial(vec![message]));
    }
}

/// A stable change fingerprint of the transient overlay inputs.
///
/// The controller uses it to rebuild **only** the highlight/preview overlay when
/// the selection or a preview point/cursor changes, leaving the base drawing and
/// the annotation batches' `Arc`s untouched. Selecting a different object or
/// moving a preview cursor changes this value; rebuilding the scene with the
/// same inputs does not.
pub fn overlay_fingerprint(inputs: &OverlayInputs) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for reference in inputs.selection.refs() {
        reference.document.0.hash(&mut hasher);
        reference.entity.0.hash(&mut hasher);
        for instance in &reference.instance.0 {
            instance.0.hash(&mut hasher);
        }
        // A separator keeps `[a]` distinct from `[]` + `a`.
        u8::MAX.hash(&mut hasher);
        match &reference.sub_element {
            Some(sub) => {
                sub.source_key.hash(&mut hasher);
                sub.topology_revision.0.hash(&mut hasher);
            }
            None => 0u8.hash(&mut hasher),
        }
    }
    hash_preview(
        &mut hasher,
        0u8,
        inputs
            .measurement
            .as_ref()
            .map(|p| (p.points.as_slice(), p.cursor)),
    );
    hash_preview(
        &mut hasher,
        1u8,
        inputs
            .annotation
            .as_ref()
            .map(|p| (p.points.as_slice(), p.cursor)),
    );
    // Visibility is part of the inputs: turning an overlay off must rebuild the
    // transient overlay even when the selection and previews are unchanged.
    inputs.visibility.axes.hash(&mut hasher);
    inputs.visibility.grid.hash(&mut hasher);
    inputs.visibility.selection_highlight.hash(&mut hasher);
    inputs.visibility.snap_hints.hash(&mut hasher);
    inputs.visibility.annotations.hash(&mut hasher);
    hasher.finish()
}

fn hash_preview<H: std::hash::Hasher>(
    hasher: &mut H,
    tag: u8,
    preview: Option<(&[Point3], Option<Point3>)>,
) {
    use std::hash::Hash;
    match preview {
        Some((points, cursor)) => {
            tag.hash(hasher);
            points.len().hash(hasher);
            for point in points {
                hash_point(hasher, *point);
            }
            match cursor {
                Some(point) => {
                    1u8.hash(hasher);
                    hash_point(hasher, point);
                }
                None => 0u8.hash(hasher),
            }
        }
        None => 255u8.hash(hasher),
    }
}

fn hash_point<H: std::hash::Hasher>(hasher: &mut H, point: Point3) {
    use std::hash::Hash;
    point.x.to_bits().hash(hasher);
    point.y.to_bits().hash(hasher);
    point.z.to_bits().hash(hasher);
}

/// Build the world-space X/Y axes overlay for `bounds`.
///
/// `bounds` is the drawing's model-space `(min, max)` from
/// [`cad_db::DrawingDatabase::bounds`]. Both axes are one `Lines` batch of two
/// segments on the `z = 0` work plane: the X axis at `y = 0` spanning the
/// bounds' x range and the Y axis at `x = 0` spanning the bounds' y range. The
/// axes are reference geometry and draw below the selection highlight.
pub fn axes_overlay(bounds: (Point3, Point3), document: DocumentId) -> VisualOverlay {
    let (min, max) = bounds;
    let points = vec![
        Point3 {
            x: min.x,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: max.x,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: min.y,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: max.y,
            z: 0.0,
        },
    ];
    let mut out = VisualOverlay::default();
    if let Some(batch) = line_batch(
        &points,
        AXES_COLOR,
        document,
        AXES_GRID_DRAW_ORDER + 1,
        "overlay.axes",
    ) {
        out.batches.push(batch);
    }
    out
}

/// Build the world-space reference grid overlay for `bounds`.
///
/// Grid lines are evenly spaced at a power-of-ten step chosen from each axis'
/// extent ([`grid_step`]); the whole grid is one `Lines` batch whose consecutive
/// vertex pairs are the individual segments. The per-axis line count is capped
/// by [`GRID_MAX_LINES`] so a pathological extent cannot grow the batch without
/// bound. Like the axes, the grid draws below the selection highlight.
pub fn grid_overlay(bounds: (Point3, Point3), document: DocumentId) -> VisualOverlay {
    let (min, max) = bounds;
    let per_axis = GRID_MAX_LINES / 2;
    let xs = grid_positions(min.x, max.x, grid_step(max.x - min.x), per_axis);
    let ys = grid_positions(min.y, max.y, grid_step(max.y - min.y), per_axis);
    let mut points: Vec<Point3> = Vec::with_capacity((xs.len() + ys.len()) * 2);
    for x in xs {
        points.push(Point3 {
            x,
            y: min.y,
            z: 0.0,
        });
        points.push(Point3 {
            x,
            y: max.y,
            z: 0.0,
        });
    }
    for y in ys {
        points.push(Point3 {
            x: min.x,
            y,
            z: 0.0,
        });
        points.push(Point3 {
            x: max.x,
            y,
            z: 0.0,
        });
    }
    let mut out = VisualOverlay::default();
    if let Some(batch) = line_batch(
        &points,
        GRID_COLOR,
        document,
        AXES_GRID_DRAW_ORDER,
        "overlay.grid",
    ) {
        out.batches.push(batch);
    }
    out
}

/// Pick a "nice" 1/2/5 × 10ⁿ grid step targeting roughly ten divisions.
///
/// Rounding straight to a power of ten can jump from ~15 to ~40 divisions for a
/// small change in extent; snapping to the 1/2/5 sequence keeps the grid between
/// about 7 and 15 divisions so an explicitly enabled grid stays readable. A
/// non-finite or degenerate extent falls back to `1.0`; the caller's count cap
/// bounds the result regardless.
fn grid_step(extent: f64) -> f64 {
    if !extent.is_finite() || extent <= 0.0 {
        return 1.0;
    }
    let target = extent / 12.0;
    let magnitude = 10f64.powf(target.log10().floor());
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return 1.0;
    }
    let normalized = target / magnitude;
    let nice = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = nice * magnitude;
    if step.is_finite() && step > 0.0 {
        step
    } else {
        1.0
    }
}

/// The multiples of `step` inside `[min, max]`, capped at `cap` entries.
///
/// When the raw count exceeds `cap`, every `stride`-th multiple is kept so the
/// grid still spans the whole range instead of shrinking into one corner.
fn grid_positions(min: f64, max: f64, step: f64, cap: usize) -> Vec<f64> {
    let mut out = Vec::new();
    if cap == 0
        || !(min.is_finite() && max.is_finite() && step.is_finite())
        || step <= 0.0
        || max <= min
    {
        return out;
    }
    let start = (min / step).ceil();
    let end = (max / step).floor();
    if !(start.is_finite() && end.is_finite()) || end < start {
        return out;
    }
    let count = (end - start + 1.0) as usize;
    let stride = count.div_ceil(cap).max(1);
    let mut index = start;
    while index <= end && out.len() < cap {
        out.push(index * step);
        index += stride as f64;
    }
    out
}

/// Build the selection highlight overlay for `selection`.
///
/// Each ref is resolved against the database by expanding `INSERT`s exactly like
/// [`crate::picking::drawing_pick_items`], so a block child reached through two
/// placements becomes two distinct refs and highlights independently. The
/// resolved geometry is tessellated by the default representation provider (the
/// same discretisation the base scene uses) and converted to overlay batches by
/// [`cad_scene::highlight_batches`].
///
/// A ref that no longer resolves (deleted entity, foreign document, or an
/// `INSERT` root that only exists as an expansion) produces an explicit
/// `highlight.unresolved` diagnostic and **no** batch — never a fabricated one.
/// Model space is the highlighted space; paper-space refs resolve only when they
/// are reached through the model-space expansion.
pub fn selection_highlight(
    database: &DrawingDatabase,
    selection: &SelectionSet,
    document: DocumentId,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    if selection.is_empty() {
        return out;
    }

    let registry = ProviderRegistry::with_default_provider();
    let mut context =
        RepresentationContext::new(document, cad_domain::TolerancePolicy::default(), stamp);
    if let Some(fonts) = fonts {
        context = context.with_fonts(fonts);
    }

    // `drawing_pick_items` already flattens INSERTs and carries the exact
    // `(entity, instance)` source, so two placements never collapse to one.
    let items = crate::picking::drawing_pick_items(database, document);
    let mut representations: Vec<DisplayRepresentation> = Vec::new();
    let mut build_reasons: Vec<Diagnostic> = Vec::new();
    for item in &items {
        let selected = selection
            .refs()
            .iter()
            .any(|reference| same_entity_instance(&item.source, reference));
        if !selected {
            continue;
        }
        match representation_for_item(&registry, database, item, &context) {
            Some(representation) => representations.push(representation),
            None => {
                // The pick item was produced from this database, so a missing
                // entity here is an internal inconsistency, not a user error;
                // report it rather than drawing something wrong.
                build_reasons.push(Diagnostic {
                    object: None,
                    code: "highlight.entity-missing".into(),
                    message: format!(
                        "selection entity {} is present in the pick index but not in the database",
                        item.source.entity.0
                    ),
                });
            }
        }
    }

    let HighlightScene {
        batches,
        completeness,
        diagnostics,
    } = cad_scene::highlight_batches(
        selection.refs(),
        representations.iter(),
        |_| true,
        &HighlightOptions::default(),
    );
    out.batches = batches;
    out.diagnostics = diagnostics;
    out.completeness = completeness;
    for reason in build_reasons {
        out.diagnostics.push(reason);
    }
    out
}

/// Whether two refs name the same entity reached through the same instance
/// chain, ignoring the sub-element (which the highlight face filter applies).
fn same_entity_instance(a: &SelectionRef, b: &SelectionRef) -> bool {
    a.document == b.document && a.entity == b.entity && a.instance == b.instance
}

/// Build a `DisplayRepresentation` for one pick item.
///
/// The provider tessellates the entity's `SemanticGeometry`; the item's
/// accumulated INSERT transform is then applied to each fragment and the
/// fragment's `source` is replaced by the exact `(entity, instance)` the item
/// carries. Returns `None` when the database entity no longer exists.
fn representation_for_item(
    registry: &ProviderRegistry,
    database: &DrawingDatabase,
    item: &cad_spatial::PickItem,
    context: &RepresentationContext,
) -> Option<DisplayRepresentation> {
    let entity = database.entity(item.source.entity)?;
    let mut representation = match registry.build(entity, context) {
        Ok(representation) => representation,
        Err(error) => DisplayRepresentation {
            fragments: Vec::new(),
            completeness: Completeness::Missing(vec![error.to_string()]),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "highlight.representation".into(),
                message: error.to_string(),
            }],
        },
    };
    for fragment in &mut representation.fragments {
        fragment.primitive = fragment.primitive.transformed(&item.transform);
        fragment.source = item.source.clone();
    }
    Some(representation)
}

/// Build the tool-preview overlay for an in-progress measurement and/or
/// annotation.
///
/// The preview is pure: it only reads the captured points, the live cursor and
/// (for rectangle/ellipse annotations) the kind. It never commits anything to
/// the database.
///
/// `snap_hints` gates the axis-aligned point crosses only. They are the current
/// cursor/snap markers, so a host that turned `view.overlays.snapHints` off
/// still gets the rubber-band chains and dragged shapes, just without the
/// crosses.
pub fn preview_overlay(
    measurement: Option<&MeasurementPreview>,
    annotation: Option<&AnnotationPreview>,
    document: DocumentId,
    snap_hints: bool,
    options: &PreviewOptions,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    if let Some(preview) = measurement {
        out.merge(measurement_preview_overlay(
            preview, document, snap_hints, options,
        ));
    }
    if let Some(preview) = annotation {
        out.merge(annotation_preview_overlay(
            preview, document, snap_hints, options,
        ));
    }
    out
}

/// The rubber-band chain for a preview: captured points followed by the live
/// cursor when one is present and there is at least one captured point.
fn preview_chain(points: &[Point3], cursor: Option<Point3>) -> Vec<Point3> {
    let mut chain: Vec<Point3> = points.to_vec();
    if let Some(cursor) = cursor {
        if !chain.is_empty() && chain.last() != Some(&cursor) {
            chain.push(cursor);
        }
    }
    chain
}

fn measurement_preview_overlay(
    preview: &MeasurementPreview,
    document: DocumentId,
    snap_hints: bool,
    options: &PreviewOptions,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    let mut order = options.draw_order;

    let chain = preview_chain(&preview.points, preview.cursor);
    if chain.len() >= 2 {
        let batch = line_batch(
            &chain,
            MEASUREMENT_PREVIEW_COLOR,
            document,
            order,
            "measurement.preview",
        );
        if let Some(batch) = batch {
            out.batches.push(batch);
            order += 1;
        }
    }
    if snap_hints {
        let markers = marker_batch(
            &preview.points,
            options.marker_size,
            MEASUREMENT_PREVIEW_COLOR,
            document,
            order,
            "measurement.preview",
        );
        if let Some(batch) = markers {
            out.batches.push(batch);
        }
    }
    out
}

fn annotation_preview_overlay(
    preview: &AnnotationPreview,
    document: DocumentId,
    snap_hints: bool,
    options: &PreviewOptions,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    let mut order = options.draw_order;
    let mut geometry: Vec<Vec<Point3>> = Vec::new();

    match preview.kind {
        AnnotationToolKind::Rectangle => {
            if let Some(corners) = rectangle_corners(&preview.points, preview.cursor) {
                geometry.push(corners);
            }
        }
        AnnotationToolKind::Ellipse => {
            if let Some(points) = ellipse_points(&preview.points, preview.cursor) {
                geometry.push(points);
            }
        }
        // Text is placed at a point (a marker, no chain); leader/freehand/cloud
        // are open strokes. A cursor extends an open stroke as a rubber band.
        AnnotationToolKind::Text => {}
        AnnotationToolKind::Leader | AnnotationToolKind::Freehand | AnnotationToolKind::Cloud => {
            let chain = preview_chain(&preview.points, preview.cursor);
            if chain.len() >= 2 {
                geometry.push(chain);
            }
        }
    }

    for points in geometry {
        if let Some(batch) = line_batch(
            &points,
            ANNOTATION_PREVIEW_COLOR,
            document,
            order,
            "annotation.preview",
        ) {
            out.batches.push(batch);
            order += 1;
        }
    }

    if snap_hints {
        if let Some(batch) = marker_batch(
            &preview.points,
            options.marker_size,
            ANNOTATION_PREVIEW_COLOR,
            document,
            order,
            "annotation.preview",
        ) {
            out.batches.push(batch);
        }
    }

    // Text placement is the one kind with no line geometry: explain the marker
    // so a host is not surprised by a points-only preview. When snap hints are
    // off the marker is suppressed, so say that instead of claiming a draw.
    if preview.kind == AnnotationToolKind::Text && !preview.points.is_empty() {
        if snap_hints {
            out.partial(
                "preview.text-marker-only",
                "text preview is a point marker; glyphs are shaped on commit".into(),
            );
        } else {
            out.partial(
                "preview.text-marker-suppressed",
                "text preview point marker hidden by view.overlays.snapHints".into(),
            );
        }
    }
    out
}

/// The closed rectangle loop for a rectangle preview, using the live cursor as
/// the second corner while only one point is captured.
fn rectangle_corners(points: &[Point3], cursor: Option<Point3>) -> Option<Vec<Point3>> {
    let corners = preview_pair(points, cursor)?;
    let [a, b] = corners;
    Some(vec![
        a,
        Point3 {
            x: b.x,
            y: a.y,
            z: a.z,
        },
        b,
        Point3 {
            x: a.x,
            y: b.y,
            z: a.z,
        },
        a,
    ])
}

/// The parametric ellipse for an ellipse preview.
///
/// Uses the same centre + orthogonal axes convention as
/// `AnnotationTool::build` and reuses [`cad_scene::tessellate_ellipse`] for the
/// discretisation, so the preview and the committed annotation agree.
fn ellipse_points(points: &[Point3], cursor: Option<Point3>) -> Option<Vec<Point3>> {
    let [center, corner] = preview_pair(points, cursor)?;
    let axis_u = Point3 {
        x: corner.x - center.x,
        y: 0.0,
        z: 0.0,
    };
    let axis_v = Point3 {
        x: 0.0,
        y: corner.y - center.y,
        z: 0.0,
    };
    if (axis_u.x * axis_u.x + axis_v.y * axis_v.y).sqrt() < 1e-12 {
        return None;
    }
    let points = cad_scene::tessellate_ellipse(center, axis_u, axis_v, 0.01);
    (points.len() >= 3).then_some(points)
}

/// The two defining points of a rectangle/ellipse preview: captured points
/// first, the live cursor filling the missing second point.
fn preview_pair(points: &[Point3], cursor: Option<Point3>) -> Option<[Point3; 2]> {
    match (points.first(), points.get(1)) {
        (Some(a), Some(b)) => Some([*a, *b]),
        (Some(a), None) => cursor.map(|c| [*a, c]),
        _ => None,
    }
}

/// Build one `Lines` batch through `points` (a polyline; consecutive duplicates
/// are preserved as zero-length segments, exactly as captured).
fn line_batch(
    points: &[Point3],
    color: [f32; 3],
    document: DocumentId,
    draw_order: i64,
    code: &str,
) -> Option<RenderBatch> {
    if points.len() < 2 {
        return None;
    }
    let origin = points[0];
    let vertices: Vec<[f32; 3]> = points
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
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::sanitize_color(color),
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: cad_db::LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![SelectionRef {
            document,
            entity: cad_domain::EntityId(preview_source_id(code)),
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        }],
        draw_order,
    })
}

/// Build one `Lines` batch of axis-aligned crosses (two segments per point).
fn marker_batch(
    points: &[Point3],
    marker_size: f64,
    color: [f32; 3],
    document: DocumentId,
    draw_order: i64,
    code: &str,
) -> Option<RenderBatch> {
    if points.is_empty() || !marker_size.is_finite() || marker_size <= 0.0 {
        return None;
    }
    let half = marker_size;
    let origin = points[0];
    let mut vertices: Vec<[f32; 3]> = Vec::with_capacity(points.len() * 4);
    let push = |vertices: &mut Vec<[f32; 3]>, p: Point3| {
        vertices.push([
            (p.x - origin.x) as f32,
            (p.y - origin.y) as f32,
            (p.z - origin.z) as f32,
        ]);
    };
    for point in points {
        push(
            &mut vertices,
            Point3 {
                x: point.x - half,
                y: point.y,
                z: point.z,
            },
        );
        push(
            &mut vertices,
            Point3 {
                x: point.x + half,
                y: point.y,
                z: point.z,
            },
        );
        push(
            &mut vertices,
            Point3 {
                x: point.x,
                y: point.y - half,
                z: point.z,
            },
        );
        push(
            &mut vertices,
            Point3 {
                x: point.x,
                y: point.y + half,
                z: point.z,
            },
        );
    }
    Some(RenderBatch {
        local_origin: origin,
        topology: RenderTopology::Lines,
        vertices,
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: cad_scene::sanitize_color(color),
        color_unresolved: false,
        lineweight: 0.0,
        lineweight_unresolved: false,
        linetype: cad_db::LinetypePattern::continuous(),
        linetype_unresolved: false,
        sources: vec![SelectionRef {
            document,
            entity: cad_domain::EntityId(preview_source_id(code)),
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        }],
        draw_order,
    })
}

/// A stable synthetic source id for a preview batch.
///
/// Previews are not database objects, so the id is a fixed namespace derived
/// from the overlay kind. It is never a real `EntityId` and never reaches the
/// database; it only lets a host attribute the batch.
fn preview_source_id(code: &str) -> u128 {
    let mut hash: u128 = 0xcbf2_9ce4_8422_2325;
    for byte in code.bytes() {
        hash ^= byte as u128;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    use cad_domain::{
        BlockId, DatabaseId, EntityId, InstancePath, LayerId, ObjectId, Revision, SemanticGeometry,
        Transform3,
    };

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn entity(id: u128, geometry: SemanticGeometry, draw_order: i64) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbEntity".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space: cad_domain::SpaceId::Model,
            geometry,
            draw_order,
        }
    }

    /// A drawing with two INSERT placements of one single-line block at x = 10
    /// and x = 20, plus a top-level line at x = 30.
    fn drawing() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(entity(
            100,
            SemanticGeometry::Line {
                start: p(0.0, -1.0),
                end: p(0.0, 1.0),
            },
            0,
        ))
        .unwrap();
        b.insert_entity(entity(
            10,
            SemanticGeometry::Insert {
                block: BlockId(0),
                transform: Transform3::translation(p(10.0, 0.0)),
            },
            1,
        ))
        .unwrap();
        b.insert_entity(entity(
            20,
            SemanticGeometry::Insert {
                block: BlockId(0),
                transform: Transform3::translation(p(20.0, 0.0)),
            },
            2,
        ))
        .unwrap();
        b.insert_entity(entity(
            30,
            SemanticGeometry::Line {
                start: p(30.0, -1.0),
                end: p(30.0, 1.0),
            },
            3,
        ))
        .unwrap();
        b.finish().unwrap()
    }

    fn block_child_ref(instance: u128) -> SelectionRef {
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(100),
            instance: InstancePath(vec![EntityId(instance)]),
            sub_element: None,
        }
    }

    fn model_ref(entity: u128) -> SelectionRef {
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(entity),
            instance: InstancePath::default(),
            sub_element: None,
        }
    }

    fn stamp() -> TaskStamp {
        TaskStamp::new(DocumentId(1), 1)
    }

    #[test]
    fn empty_selection_yields_no_batches() {
        let db = drawing();
        let overlay = selection_highlight(&db, &SelectionSet::new(), DocumentId(1), stamp(), None);
        assert!(overlay.is_empty());
        assert_eq!(overlay.completeness, Completeness::Complete);
        assert!(overlay.diagnostics.is_empty());
    }

    #[test]
    fn highlight_of_a_line_has_expected_vertices() {
        let db = drawing();
        let selection = SelectionSet::from_refs([model_ref(30)]);
        let overlay = selection_highlight(&db, &selection, DocumentId(1), stamp(), None);
        assert_eq!(overlay.drawn(), 1);
        let batch = &overlay.batches[0];
        assert_eq!(batch.topology, RenderTopology::Lines);
        assert_eq!(batch.vertices.len(), 2);
        // Local origin is the line start; the second vertex is the delta.
        assert_eq!(batch.local_origin, p(30.0, -1.0));
        assert_eq!(batch.vertices[1], [0.0, 2.0, 0.0]);
        // Highlight tint is concrete and the draw order sits between the base
        // drawing (0) and the annotation overlay (1_000_000).
        assert!(!batch.color_unresolved);
        assert_eq!(batch.color, cad_scene::DEFAULT_HIGHLIGHT_COLOR);
        assert!(batch.draw_order > 0 && batch.draw_order < 1_000_000);
        assert_eq!(batch.sources, vec![model_ref(30)]);
    }

    #[test]
    fn two_insert_instances_highlight_independently() {
        let db = drawing();
        // Only the first placement (instance [10]) is selected.
        let selection = SelectionSet::from_refs([block_child_ref(10)]);
        let overlay = selection_highlight(&db, &selection, DocumentId(1), stamp(), None);
        assert_eq!(overlay.drawn(), 1);
        let batch = &overlay.batches[0];
        assert_eq!(batch.sources, vec![block_child_ref(10)]);
        // The block child is a vertical line at local x = 0; placing it at
        // x = 10 gives start (10, -1).
        assert_eq!(batch.local_origin, p(10.0, -1.0));
        assert_eq!(batch.vertices[1], [0.0, 2.0, 0.0]);

        // Selecting both placements yields two batches with distinct origins.
        let both = SelectionSet::from_refs([block_child_ref(10), block_child_ref(20)]);
        let overlay = selection_highlight(&db, &both, DocumentId(1), stamp(), None);
        assert_eq!(overlay.drawn(), 2);
        assert_ne!(
            overlay.batches[0].local_origin,
            overlay.batches[1].local_origin
        );
        assert_eq!(overlay.batches[0].sources, vec![block_child_ref(10)]);
        assert_eq!(overlay.batches[1].sources, vec![block_child_ref(20)]);
    }

    #[test]
    fn unresolved_ref_yields_a_diagnostic_and_no_batch() {
        let db = drawing();
        let selection = SelectionSet::from_refs([model_ref(999)]);
        let overlay = selection_highlight(&db, &selection, DocumentId(1), stamp(), None);
        assert!(
            overlay.is_empty(),
            "a deleted entity must not produce a fabricated batch"
        );
        assert!(overlay
            .diagnostics
            .iter()
            .any(|d| d.code == "highlight.unresolved"));
        assert!(matches!(overlay.completeness, Completeness::Missing(_)));
    }

    #[test]
    fn empty_preview_yields_no_batches() {
        let overlay = preview_overlay(None, None, DocumentId(1), true, &PreviewOptions::default());
        assert!(overlay.is_empty());
    }

    #[test]
    fn measurement_preview_produces_n_minus_one_segments_and_markers() {
        let preview = MeasurementPreview {
            kind: crate::measure_tool::MeasurementToolKind::PolylineLength,
            points: vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 1.0)],
            cursor: None,
            remaining: 0,
            ready: true,
        };
        let overlay = preview_overlay(
            Some(&preview),
            None,
            DocumentId(1),
            true,
            &PreviewOptions::default(),
        );
        // One polyline batch (3 vertices -> 2 segments) + one marker batch.
        assert_eq!(overlay.drawn(), 2);
        assert_eq!(overlay.batches[0].vertices.len(), 3);
        // 3 points -> 3 crosses -> 6 segments -> 12 vertices.
        assert_eq!(overlay.batches[1].vertices.len(), 12);
        assert_eq!(overlay.batches[0].color, MEASUREMENT_PREVIEW_COLOR);
        assert!(overlay.batches[0].draw_order >= PREVIEW_DRAW_ORDER);
        assert!(overlay.batches.iter().all(|b| !b.color_unresolved));
    }

    #[test]
    fn measurement_cursor_extends_the_chain_to_the_last_point() {
        let preview = MeasurementPreview {
            kind: crate::measure_tool::MeasurementToolKind::Distance,
            points: vec![p(0.0, 0.0)],
            cursor: Some(p(5.0, 0.0)),
            remaining: 1,
            ready: false,
        };
        let overlay = preview_overlay(
            Some(&preview),
            None,
            DocumentId(1),
            true,
            &PreviewOptions::default(),
        );
        // Chain is [captured, cursor] -> 2 vertices; plus the captured marker.
        assert_eq!(overlay.drawn(), 2);
        assert_eq!(overlay.batches[0].vertices.len(), 2);
        assert_eq!(overlay.batches[0].vertices[1], [5.0, 0.0, 0.0]);
    }

    #[test]
    fn cancelling_a_preview_makes_it_disappear() {
        let preview = MeasurementPreview {
            kind: crate::measure_tool::MeasurementToolKind::Distance,
            points: vec![p(0.0, 0.0), p(1.0, 0.0)],
            cursor: None,
            remaining: 0,
            ready: true,
        };
        let active = preview_overlay(
            Some(&preview),
            None,
            DocumentId(1),
            true,
            &PreviewOptions::default(),
        );
        assert!(!active.is_empty());
        let cancelled =
            preview_overlay(None, None, DocumentId(1), true, &PreviewOptions::default());
        assert!(cancelled.is_empty());
    }

    #[test]
    fn rectangle_annotation_preview_is_a_closed_loop() {
        let preview = AnnotationPreview {
            kind: AnnotationToolKind::Rectangle,
            points: vec![p(0.0, 0.0), p(4.0, 2.0)],
            cursor: None,
            remaining: 0,
            requires_text: false,
            text_supplied: false,
        };
        let overlay = preview_overlay(
            None,
            Some(&preview),
            DocumentId(1),
            true,
            &PreviewOptions::default(),
        );
        let loop_batch = &overlay.batches[0];
        assert_eq!(loop_batch.vertices.len(), 5);
        assert_eq!(loop_batch.vertices[0], loop_batch.vertices[4]);
        assert_eq!(loop_batch.color, ANNOTATION_PREVIEW_COLOR);
    }

    #[test]
    fn ellipse_annotation_preview_tessellates_onto_the_curve() {
        let preview = AnnotationPreview {
            kind: AnnotationToolKind::Ellipse,
            points: vec![p(0.0, 0.0), p(4.0, 2.0)],
            cursor: None,
            remaining: 0,
            requires_text: false,
            text_supplied: false,
        };
        let overlay = preview_overlay(
            None,
            Some(&preview),
            DocumentId(1),
            true,
            &PreviewOptions::default(),
        );
        let batch = &overlay.batches[0];
        assert!(batch.vertices.len() >= 8);
        assert_eq!(batch.color, ANNOTATION_PREVIEW_COLOR);
        // Vertices are local to `local_origin`; reconstruct world points and
        // check the ellipse equation ((x)/4)^2 + ((y)/2)^2 = 1.
        for v in &batch.vertices {
            let x = batch.local_origin.x + v[0] as f64;
            let y = batch.local_origin.y + v[1] as f64;
            let u = x / 4.0;
            let w = y / 2.0;
            assert!((u * u + w * w - 1.0).abs() < 1e-6, "off curve: {x},{y}");
        }
    }

    #[test]
    fn annotation_cursor_previews_the_dragged_rectangle() {
        let preview = AnnotationPreview {
            kind: AnnotationToolKind::Rectangle,
            points: vec![p(0.0, 0.0)],
            cursor: Some(p(3.0, 3.0)),
            remaining: 1,
            requires_text: false,
            text_supplied: false,
        };
        let overlay = preview_overlay(
            None,
            Some(&preview),
            DocumentId(1),
            true,
            &PreviewOptions::default(),
        );
        let loop_batch = &overlay.batches[0];
        assert_eq!(loop_batch.vertices.len(), 5);
        assert_eq!(loop_batch.vertices[2], [3.0, 3.0, 0.0]);
    }

    #[test]
    fn overlay_fingerprint_tracks_selection_and_cursor() {
        let mut a = OverlayInputs::default();
        let base = overlay_fingerprint(&a);
        a.selection = SelectionSet::from_refs([block_child_ref(10)]);
        let selected = overlay_fingerprint(&a);
        assert_ne!(base, selected);
        a.selection.replace([block_child_ref(20)]);
        assert_ne!(selected, overlay_fingerprint(&a));

        let mut b = OverlayInputs {
            measurement: Some(MeasurementPreview {
                kind: crate::measure_tool::MeasurementToolKind::Distance,
                points: vec![p(0.0, 0.0)],
                cursor: None,
                remaining: 1,
                ready: false,
            }),
            ..OverlayInputs::default()
        };
        let no_cursor = overlay_fingerprint(&b);
        b.measurement.as_mut().unwrap().cursor = Some(p(1.0, 0.0));
        assert_ne!(no_cursor, overlay_fingerprint(&b));
        // Rebuilding with identical inputs is stable.
        assert_eq!(overlay_fingerprint(&b), overlay_fingerprint(&b));
    }

    #[test]
    fn overlay_fingerprint_tracks_each_visibility_flag() {
        let baseline = overlay_fingerprint(&OverlayInputs::default());
        let flips: [fn(&mut OverlayVisibility); 5] = [
            |v| v.axes = false,
            |v| v.grid = false,
            |v| v.selection_highlight = false,
            |v| v.snap_hints = false,
            |v| v.annotations = false,
        ];
        for flip in flips {
            let mut inputs = OverlayInputs::default();
            flip(&mut inputs.visibility);
            assert_ne!(
                baseline,
                overlay_fingerprint(&inputs),
                "flipping a visibility flag must invalidate the overlay"
            );
        }
    }

    #[test]
    fn axes_overlay_draws_two_origin_lines_below_the_highlight() {
        let overlay = axes_overlay((p(-2.0, -3.0), p(4.0, 5.0)), DocumentId(1));
        assert_eq!(overlay.drawn(), 1);
        let batch = &overlay.batches[0];
        assert_eq!(batch.topology, RenderTopology::Lines);
        // Two segments: X axis (-2,0)->(4,0), then Y axis (0,-3)->(0,5).
        assert_eq!(batch.vertices.len(), 4);
        assert_eq!(batch.local_origin, p(-2.0, 0.0));
        assert_eq!(batch.vertices[0], [0.0, 0.0, 0.0]);
        assert_eq!(batch.vertices[1], [6.0, 0.0, 0.0]);
        assert_eq!(batch.vertices[2], [2.0, -3.0, 0.0]);
        assert_eq!(batch.vertices[3], [2.0, 5.0, 0.0]);
        assert_eq!(batch.color, AXES_COLOR);
        assert!(batch.draw_order < 900_000);
        assert!(!batch.color_unresolved);
    }

    #[test]
    fn grid_overlay_is_one_bounded_batch_spanning_the_bounds() {
        let overlay = grid_overlay((p(-50.0, -50.0), p(50.0, 50.0)), DocumentId(1));
        assert_eq!(overlay.drawn(), 1);
        let batch = &overlay.batches[0];
        assert!(!batch.vertices.is_empty());
        assert_eq!(batch.vertices.len() % 2, 0);
        assert!(batch.vertices.len() / 2 <= GRID_MAX_LINES);
        assert_eq!(batch.color, GRID_COLOR);
        assert!(batch.draw_order < 900_000);
    }

    #[test]
    fn grid_positions_caps_a_dense_axis_without_shrinking_it() {
        let positions = grid_positions(0.0, 1000.0, 0.001, 10);
        assert_eq!(positions.len(), 10);
        assert_eq!(positions[0], 0.0);
        assert!(
            positions.last().copied().unwrap() > 900.0,
            "the cap samples across the whole range, not just the corner"
        );
    }

    #[test]
    fn snap_hints_false_suppresses_measurement_markers_but_keeps_the_chain() {
        let preview = MeasurementPreview {
            kind: crate::measure_tool::MeasurementToolKind::Distance,
            points: vec![p(0.0, 0.0), p(1.0, 0.0)],
            cursor: None,
            remaining: 0,
            ready: true,
        };
        let with = preview_overlay(
            Some(&preview),
            None,
            DocumentId(1),
            true,
            &PreviewOptions::default(),
        );
        // Rubber-band chain + point-marker crosses.
        assert_eq!(with.drawn(), 2);
        let without = preview_overlay(
            Some(&preview),
            None,
            DocumentId(1),
            false,
            &PreviewOptions::default(),
        );
        // The chain stays; only the cursor/snap crosses disappear.
        assert_eq!(without.drawn(), 1);
        assert_eq!(without.batches[0].topology, RenderTopology::Lines);
    }

    #[test]
    fn snap_hints_false_suppresses_annotation_markers_but_keeps_the_stroke() {
        let preview = AnnotationPreview {
            kind: AnnotationToolKind::Leader,
            points: vec![p(0.0, 0.0), p(1.0, 0.0)],
            cursor: None,
            remaining: 0,
            requires_text: false,
            text_supplied: false,
        };
        let with = preview_overlay(
            None,
            Some(&preview),
            DocumentId(1),
            true,
            &PreviewOptions::default(),
        );
        assert_eq!(with.drawn(), 2);
        let without = preview_overlay(
            None,
            Some(&preview),
            DocumentId(1),
            false,
            &PreviewOptions::default(),
        );
        assert_eq!(without.drawn(), 1);
    }
}
