//! Transient visual overlays: selection highlight and tool previews.
//!
//! This module produces the **derived** overlay batches a host draws on top of
//! the authoritative scene:
//!
//! * a **selection highlight** that resolves the exact `(entity, instance,
//!   sub-element)` refs in a [`SelectionSet`] against the database, expanding
//!   `INSERT`s exactly like [`crate::picking::drawing_pick_items`], so two
//!   placements of the same block highlight independently;
//! * a **tool preview** that turns an in-progress [`MeasurementPreview`] into
//!   line geometry (a rubber-band chain and point crosses).
//!
//! Nothing here mutates the database, the scene cache or history. Previews are
//! pure snapshots: a cancelled preview simply stops being passed in, and no
//! transaction is ever opened (see `cad-app::measure_tool`).
//!
//! ## Reuse, not a second renderer
//!
//! Highlight batches are built by [`cad_scene::highlight_batches`], the one
//! existing highlight→[`RenderBatch`] converter, from `DisplayRepresentation`s
//! the default provider tessellates (the same discretisation the base scene
//! uses). There is no second tessellator and no new renderer.
//!
//! ## Paint order
//!
//! | overlay | `draw_order` |
//! |---|---|
//! | base drawing (`SceneCache`) | `0` |
//! | selection highlight (`HIGHLIGHT_DRAW_ORDER`) | `900_000` |
//! | object-snap hints ([`SNAP_HINT_DRAW_ORDER`]) | `1_500_000` |
//! | tool preview ([`PREVIEW_DRAW_ORDER`]) | `2_000_000` |
//!
//! A selection is a transient decoration; a tool preview is the live feedback
//! the user is acting on, so it draws above everything. Object-snap hints sit
//! below the live preview rubber band.

use std::collections::HashSet;
use std::sync::Arc;

use cad_db::DrawingDatabase;
use cad_domain::{
    Completeness, Diagnostic, DocumentId, EntityId, Point3, SelectionRef, TaskStamp, Transform3,
};
use cad_measure::{SnapCandidate, SnapKind};
use cad_representation::{
    DisplayRepresentation, FontEngine, ProviderRegistry, RepresentationContext,
};
use cad_scene::{HighlightOptions, HighlightScene, RenderBatch, RenderTopology};

use crate::draw_tool::{DrawPreview, DrawToolKind};
use crate::measure_tool::MeasurementPreview;
use crate::selection::SelectionSet;
use crate::viewer_config::OverlayVisibility;

/// `draw_order` of the first tool-preview batch.
///
/// Above the selection highlight (`HIGHLIGHT_DRAW_ORDER`, `900_000`), so the
/// live tool feedback is never hidden by markup while the user is drawing.
pub const PREVIEW_DRAW_ORDER: i64 = 2_000_000;

/// `draw_order` of the world-space axes and grid reference overlays.
///
/// Below the selection highlight (`HIGHLIGHT_DRAW_ORDER`, `900_000`) so a
/// selection always reads above the reference grid, and above the base drawing
/// (`0`).
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

/// `draw_order` of the object-snap hint markers.
///
/// Above the selection highlight (`900_000`), so a snapped point is never
/// hidden, but below the tool preview ([`PREVIEW_DRAW_ORDER`], `2_000_000`)
/// because the live rubber-band feedback the user is acting on must stay on top.
pub const SNAP_HINT_DRAW_ORDER: i64 = 1_500_000;

/// Colour of the object-snap hint markers: a warm amber, distinct from the
/// selection highlight tint and from both preview colours.
pub const SNAP_HINT_COLOR: [f32; 3] = [1.0, 0.72, 0.16];

/// Number of segments used to approximate the `Center` snap marker circle.
const SNAP_HINT_CIRCLE_SEGMENTS: usize = 12;

/// A resolved object-snap hint a host feeds into the transient overlay.
///
/// This is the real [`cad_measure::SnapCandidate`] the measurement snap engine
/// already returns: `cad-app` depends on `cad-measure`, and no architecture rule
/// forbids that edge (`scripts/check-architecture.py` only constrains the
/// acadrust/wgpu/UI/platform boundaries and the `cad-domain`/`cad-db` bases).
/// Reusing the engine type avoids a lossy mirror and means a host can hand the
/// exact `snap_best`/`snap_targets` result straight to the renderer; a host that
/// only knows a point and a kind builds the struct literal directly.
pub type SnapHint = SnapCandidate;

/// The kind of an object-snap hint, mirroring [`cad_measure::SnapKind`].
///
/// Re-exported under an overlay-facing name so a host that does not itself
/// depend on `cad-measure` can still name the marker shapes.
pub type SnapHintKind = SnapKind;

/// Preview colour: a cool cyan, distinct from the warm highlight tint.
pub const MEASUREMENT_PREVIEW_COLOR: [f32; 3] = [0.20, 0.85, 0.95];
/// Preview colour for drawing/editing tools: magenta, distinct from highlight
/// and measurement.
pub const DRAW_PREVIEW_COLOR: [f32; 3] = [0.95, 0.35, 0.85];

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
/// empty selection and a `None` preview produce no overlay batches, which is
/// an explicit empty overlay rather than an error.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OverlayInputs {
    pub selection: SelectionSet,
    /// Entity-level hovered refs.
    ///
    /// Hover is the *same* flags mechanism as selection, just a lighter tint
    /// (`FLAG_HOVERED`); it is a whole-entity state, so it is carried as flags
    /// rather than an overlay batch. Empty by default, so a host that never feeds
    /// a hover gets no tint.
    pub hovered: SelectionSet,
    /// Entity-level refs the session wants hidden (`FLAG_HIDDEN` discards the
    /// fragment in-shader, so a flagged entity is not drawn).
    ///
    /// This is a transient session state distinct from the database/layer
    /// visibility that decides what is in the base scene at all. Empty by default.
    pub hidden: SelectionSet,
    pub measurement: Option<MeasurementPreview>,
    /// In-progress draw/edit preview (drawing-edit §4), drawn through the same
    /// transient overlay so a draw gesture gets live rubber-band feedback.
    pub draw_preview: Option<DrawPreview>,
    /// Resolved object-snap hints (the markers the measurement snap engine
    /// reports), drawn as a distinct overlay. Empty by default, so a host that
    /// never feeds snaps gets no snap-hint batch. Gated by
    /// `visibility.snap_hints` separately from the pointer-cursor crosses in
    /// [`preview_overlay`]; the two are different concepts.
    pub snap_hints_input: Vec<SnapHint>,
    /// Which derived overlays the host wants drawn. Defaults to all-on
    /// ([`OverlayVisibility::default`]) so a caller that never manages overlay
    /// visibility still gets the historical behavior.
    pub visibility: OverlayVisibility,
    /// Exact world-space view rectangle for viewport-sized reference geometry.
    /// Updated on camera, canvas size, and zoom changes; the base scene remains
    /// reusable while only this visual overlay is regenerated.
    pub viewport_bounds: Option<(Point3, Point3)>,
}

impl OverlayInputs {
    /// Whether every overlay input is empty (no highlight, no hover/hidden
    /// flags, no preview, no resolved snap hints).
    pub fn is_empty(&self) -> bool {
        self.selection.is_empty()
            && self.hovered.is_empty()
            && self.hidden.is_empty()
            && self.measurement.is_none()
            && self.draw_preview.is_none()
            && self.snap_hints_input.is_empty()
    }
}

/// The built transient overlay.
#[derive(Debug, Clone, PartialEq)]
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

/// A stable change fingerprint of the transient **overlay batch** inputs.
///
/// The controller uses it to rebuild **only** the sub-element/preview overlay
/// when a face selection or a preview point/cursor changes, leaving the base
/// drawing `Arc` untouched. Entity-level selection/hover/hidden are the renderer's
/// flags path, not overlay batches, so they are deliberately **excluded** here
/// (see [`entity_flags_fingerprint`]): an entity-only selection change must not
/// rebuild or re-upload any overlay geometry.
pub fn overlay_fingerprint(inputs: &OverlayInputs) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Only sub-element (mesh face) refs still produce an overlay batch.
    for reference in inputs
        .selection
        .refs()
        .iter()
        .filter(|reference| reference.sub_element.is_some())
    {
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
            .draw_preview
            .as_ref()
            .map(|p| (p.points.as_slice(), p.cursor)),
    );
    // Snap hints are pure session state; their point/kind changes the marker
    // set and must invalidate the transient overlay.
    2u8.hash(&mut hasher);
    inputs.snap_hints_input.len().hash(&mut hasher);
    for hint in &inputs.snap_hints_input {
        // `SnapKind` has no `Hash`; its stable string is the fingerprint key.
        hint.kind.as_str().hash(&mut hasher);
        hash_point(&mut hasher, hint.point);
    }
    // Visibility is part of the inputs: turning an overlay off must rebuild the
    // transient overlay even when the selection and previews are unchanged.
    inputs.visibility.axes.hash(&mut hasher);
    inputs.visibility.grid.hash(&mut hasher);
    inputs.visibility.selection_highlight.hash(&mut hasher);
    inputs.visibility.snap_hints.hash(&mut hasher);
    match inputs.viewport_bounds {
        Some((min, max)) => {
            1u8.hash(&mut hasher);
            hash_point(&mut hasher, min);
            hash_point(&mut hasher, max);
        }
        None => 0u8.hash(&mut hasher),
    }
    hasher.finish()
}

/// A stable change fingerprint of the **entity flags** inputs.
///
/// Entity-level selection/hover/hidden are resolved into renderer flags, not
/// overlay batches, so they are hashed here (not in [`overlay_fingerprint`]).
/// The controller uses it to republish the flags without rebuilding or
/// re-uploading any overlay geometry: an entity-only selection change leaves the
/// base `Arc` and the overlay delta byte-identical.
///
/// `selection_highlight` gates the selected tint only; hover/hidden are
/// independent transient states and always participate.
///
/// Note: `selection_highlight` is hashed **here and** in [`overlay_fingerprint`]
/// on purpose — it gates both the sub-element overlay (built only when on) and
/// the entity-level selected tint, so a toggle must rebuild/republish both.
pub fn entity_flags_fingerprint(inputs: &OverlayInputs) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let hash_refs =
        |hasher: &mut std::collections::hash_map::DefaultHasher, tag: u8, refs: &[SelectionRef]| {
            tag.hash(hasher);
            for reference in refs
                .iter()
                .filter(|reference| reference.sub_element.is_none())
            {
                reference.document.0.hash(hasher);
                reference.entity.0.hash(hasher);
                for instance in &reference.instance.0 {
                    instance.0.hash(hasher);
                }
                u8::MAX.hash(hasher);
            }
        };
    hash_refs(&mut hasher, 0, inputs.selection.refs());
    hash_refs(&mut hasher, 1, inputs.hovered.refs());
    hash_refs(&mut hasher, 2, inputs.hidden.refs());
    inputs.visibility.selection_highlight.hash(&mut hasher);
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
/// segments on the `z = 0` work plane: the X axis at `y = 0` and the Y axis at
/// `x = 0`. Each axis is drawn **symmetrically about the origin**, spanning
/// `[-extent, extent]` on that axis where `extent` is the larger absolute bound.
/// Clipping each axis to the one-sided model range made the origin cross read as
/// a lopsided "L"-ish shape whenever the drawing sat mostly on one side of the
/// origin (the reported "crosshair left arm too short"); mirroring keeps both
/// arms equal so it reads as a proper crosshair. The axes are reference geometry
/// and draw below the selection highlight.
pub fn axes_overlay(bounds: (Point3, Point3), document: DocumentId) -> VisualOverlay {
    let (min, max) = bounds;
    let extent_x = min.x.abs().max(max.x.abs());
    let extent_y = min.y.abs().max(max.y.abs());
    let points = vec![
        Point3 {
            x: -extent_x,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: extent_x,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: -extent_y,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: extent_y,
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
    let span = (max.x - min.x).abs().max((max.y - min.y).abs());
    // At large pan offsets, a nominal zoom-in step can be smaller than one
    // representable f64 increment at the coordinates. Keep adjacent ticks
    // distinguishable instead of emitting duplicate lines.
    let coordinate_scale = [min.x, min.y, max.x, max.y]
        .into_iter()
        .map(f64::abs)
        .fold(1.0_f64, f64::max);
    let step = grid_step(span).max(coordinate_scale * f64::EPSILON * 8.0);
    let xs = grid_positions(min.x, max.x, step, per_axis);
    let ys = grid_positions(min.y, max.y, step, per_axis);
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

/// Build the **sub-element** selection highlight overlay for `selection`.
///
/// Whole-entity refs are the renderer's in-stream flags path (see
/// [`selection_entity_flags`]): they produce no batch here, so a whole-entity
/// selection never double-highlights. Only mesh-face sub-elements still need a
/// small separate draw, because the static merged streams do not re-upload per
/// selection change (the user explicitly authorized a separate draw for the few
/// selected sub-elements). Tool previews are built by [`preview_overlay`].
///
/// Only the selected **sub-element** refs are resolved: each ref's own `INSERT`
/// chain is walked with [`crate::picking::resolve_pick_transform`] instead of
/// scanning the whole drawing, so a block child reached through two placements
/// becomes two distinct refs and highlights independently without cloning every
/// entity's geometry. The resolved geometry is tessellated by the default
/// representation provider (the same discretisation the base scene uses) and
/// converted to overlay batches by [`cad_scene::highlight_batches`].
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
    // Entity-level refs are the flags path; only sub-elements are overlaid.
    let sub_element_refs: Vec<SelectionRef> = selection
        .refs()
        .iter()
        .filter(|reference| reference.sub_element.is_some())
        .cloned()
        .collect();
    if sub_element_refs.is_empty() {
        return out;
    }

    let registry = ProviderRegistry::with_default_provider();
    let mut context =
        RepresentationContext::new(document, cad_domain::TolerancePolicy::default(), stamp);
    if let Some(fonts) = fonts {
        context = context.with_fonts(fonts);
    }

    let mut representations: Vec<DisplayRepresentation> = Vec::new();
    let mut build_reasons: Vec<Diagnostic> = Vec::new();
    // Two refs that share an `(entity, instance)` but differ in sub-element are
    // the *same* object to the old full scan, which matched both against one
    // item; resolve each such object exactly once so the batches stay identical.
    let mut resolved: HashSet<(DocumentId, EntityId, Vec<EntityId>)> = HashSet::new();
    for reference in &sub_element_refs {
        let key = (
            reference.document,
            reference.entity,
            reference.instance.0.clone(),
        );
        if !resolved.insert(key) {
            continue;
        }
        let Some(transform) = crate::picking::resolve_pick_transform(database, document, reference)
        else {
            // The full scan would not have produced this ref either; the missing
            // representation becomes an explicit `highlight.unresolved`.
            continue;
        };
        match representation_for_ref(&registry, database, reference, &transform, &context) {
            Some(representation) => representations.push(representation),
            None => {
                // The ref resolved against this database, so a missing entity
                // here is an internal inconsistency, not a user error; report it
                // rather than drawing something wrong.
                build_reasons.push(Diagnostic {
                    object: None,
                    code: "highlight.entity-missing".into(),
                    message: format!(
                        "selection entity {} is present in the selection but not in the database",
                        reference.entity.0
                    ),
                });
            }
        }
    }

    let HighlightScene {
        batches,
        completeness,
        diagnostics,
        ..
    } = cad_scene::highlight_batches(
        &sub_element_refs,
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

/// The entity-level flags resolved from an [`OverlayInputs`].
#[derive(Debug, Clone, PartialEq)]
pub struct EntityFlagScene {
    /// One `(SelectionRef, bits)` per entity-level ref, in selection/hover/hidden
    /// order, with the bits of the same placement OR-ed. Bits are
    /// [`cad_scene::FLAG_SELECTED`] / [`FLAG_HOVERED`] / [`FLAG_HIDDEN`].
    pub flags: Vec<(SelectionRef, u32)>,
    /// `highlight.unresolved` per entity-level ref the database does not resolve.
    pub diagnostics: Vec<Diagnostic>,
    pub completeness: Completeness,
}

/// Resolve the entity-level selection/hover/hidden inputs into renderer flags.
///
/// This is the flags twin of [`selection_highlight`]: it reuses the same
/// resolve-only-refs logic (`resolve_pick_transform` per ref, never a full pick
/// scan) but produces `(SelectionRef, bits)` pairs for the renderer's
/// `set_entity_flags` instead of overlay geometry. A ref that does not resolve
/// (deleted entity, foreign document, or an `INSERT` root that only exists as an
/// expansion) produces an explicit `highlight.unresolved` diagnostic and **no**
/// flag — never a silent miss. Sub-element refs are ignored here: they are the
/// overlay path.
///
/// `inputs.visibility.selection_highlight` gates the selected tint (turning the
/// selection highlight off clears it); hover and hidden are independent transient
/// states and always participate.
pub fn selection_entity_flags(
    database: &DrawingDatabase,
    inputs: &OverlayInputs,
    document: DocumentId,
) -> EntityFlagScene {
    let mut out = EntityFlagScene {
        flags: Vec::new(),
        diagnostics: Vec::new(),
        completeness: Completeness::Complete,
    };
    let resolve = |reference: &SelectionRef, bits: u32, out: &mut EntityFlagScene| {
        if crate::picking::resolve_pick_transform(database, document, reference).is_none() {
            let message = format!(
                "selected entity {} is not present in the document",
                reference.entity.0
            );
            out.diagnostics.push(Diagnostic {
                object: None,
                code: "highlight.unresolved".into(),
                message: message.clone(),
            });
            out.completeness = out
                .completeness
                .clone()
                .combine(Completeness::Missing(vec![message]));
            return;
        }
        match out
            .flags
            .iter_mut()
            .find(|(existing, _)| same_entity_instance(existing, reference))
        {
            Some(entry) => entry.1 |= bits,
            None => out.flags.push((reference.clone(), bits)),
        }
    };

    if inputs.visibility.selection_highlight {
        for reference in inputs
            .selection
            .refs()
            .iter()
            .filter(|reference| reference.sub_element.is_none())
        {
            resolve(reference, cad_scene::FLAG_SELECTED, &mut out);
        }
    }
    for reference in inputs
        .hovered
        .refs()
        .iter()
        .filter(|reference| reference.sub_element.is_none())
    {
        resolve(reference, cad_scene::FLAG_HOVERED, &mut out);
    }
    for reference in inputs
        .hidden
        .refs()
        .iter()
        .filter(|reference| reference.sub_element.is_none())
    {
        resolve(reference, cad_scene::FLAG_HIDDEN, &mut out);
    }
    out
}

/// Whether two refs name the same entity reached through the same instance
/// chain, ignoring the sub-element (which the highlight face filter applies).
///
/// The flags resolver merges bits by this identity, and the test-only full-scan
/// reference uses the same predicate.
fn same_entity_instance(a: &SelectionRef, b: &SelectionRef) -> bool {
    a.document == b.document && a.entity == b.entity && a.instance == b.instance
}

/// Build a `DisplayRepresentation` for one resolved selection ref.
///
/// The provider tessellates the entity's `SemanticGeometry`; the ref's
/// accumulated `INSERT` transform is then applied to each fragment and the
/// fragment's `source` is set to the ref's `(entity, instance)` so two
/// placements never collapse to one. Returns `None` when the database entity no
/// longer exists.
fn representation_for_ref(
    registry: &ProviderRegistry,
    database: &DrawingDatabase,
    reference: &SelectionRef,
    transform: &Transform3,
    context: &RepresentationContext,
) -> Option<DisplayRepresentation> {
    let entity = database.entity(reference.entity)?;
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
        fragment.primitive = fragment.primitive.transformed(transform);
        // The sub-element is applied by `highlight_batches`'s face filter, so the
        // fragment carries only the `(entity, instance)` identity (as the old
        // full-scan item did).
        fragment.source = SelectionRef {
            sub_element: None,
            ..reference.clone()
        };
    }
    Some(representation)
}

/// Build the tool-preview overlay for an in-progress measurement.
///
/// The preview is pure: it only reads the captured points and the live cursor.
/// It never commits anything to the database.
///
/// `snap_hints` gates the axis-aligned point crosses only. They are the current
/// cursor/snap markers, so a host that turned `view.overlays.snapHints` off
/// still gets the rubber-band chain, just without the crosses.
pub fn preview_overlay(
    measurement: Option<&MeasurementPreview>,
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
    out
}

/// Build the live draw/edit preview: a rubber-band chain for line/move/trim and
/// a circle for the circle tool.
///
/// Presentation only; the committed operation is the exact intent from the draw
/// tool's own state machine and never this overlay.
pub fn draw_preview_overlay(
    preview: &DrawPreview,
    document: DocumentId,
    snap_hints: bool,
    options: &PreviewOptions,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    let mut order = options.draw_order;
    let geometry: Option<Vec<Point3>> = match preview.kind {
        DrawToolKind::Circle => circle_points(&preview.points, preview.cursor),
        DrawToolKind::Arc => arc_points(&preview.points, preview.cursor).or_else(|| {
            let chain = preview_chain(&preview.points, preview.cursor);
            (chain.len() >= 2).then_some(chain)
        }),
        DrawToolKind::Line
        | DrawToolKind::Move
        | DrawToolKind::Trim
        | DrawToolKind::Copy
        | DrawToolKind::Extend
        | DrawToolKind::Rotate
        | DrawToolKind::Scale
        | DrawToolKind::Mirror
        | DrawToolKind::Offset => {
            let chain = preview_chain(&preview.points, preview.cursor);
            (chain.len() >= 2).then_some(chain)
        }
    };
    if let Some(points) = geometry {
        if let Some(batch) =
            line_batch(&points, DRAW_PREVIEW_COLOR, document, order, "draw.preview")
        {
            out.batches.push(batch);
            order += 1;
        }
    }
    if snap_hints {
        if let Some(batch) = marker_batch(
            &preview.points,
            options.marker_size,
            DRAW_PREVIEW_COLOR,
            document,
            order,
            "draw.preview",
        ) {
            out.batches.push(batch);
        }
    }
    out
}

/// The arc for an arc-tool preview: once two points are captured the live cursor
/// supplies the third (on-arc) point; a third captured point replaces it.
///
/// Returns `None` (so the caller falls back to the rubber-band chain) while fewer
/// than three points are known or when the three are collinear.
fn arc_points(points: &[Point3], cursor: Option<Point3>) -> Option<Vec<Point3>> {
    let (start, through, end) = match points {
        [a, b, c, ..] => (*a, *b, *c),
        [a, b] => (*a, *b, cursor?),
        _ => return None,
    };
    let parameters = crate::draw_tool::arc_parameters(start, through, end).ok()?;
    let sweep = parameters.sweep.abs();
    let segments = ((64.0 * sweep / std::f64::consts::TAU).ceil() as usize).clamp(8, 128);
    let mut out = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let angle = parameters.start + parameters.sweep * (i as f64) / (segments as f64);
        let (sin, cos) = angle.sin_cos();
        out.push(Point3 {
            x: parameters.center.x + parameters.radius * cos,
            y: parameters.center.y + parameters.radius * sin,
            z: parameters.center.z,
        });
    }
    Some(out)
}

/// The circle for a circle-tool preview: centre + edge point (the live cursor
/// fills the edge while only the centre is captured).
fn circle_points(points: &[Point3], cursor: Option<Point3>) -> Option<Vec<Point3>> {
    let center = *points.first()?;
    let edge = points.get(1).copied().or(cursor)?;
    let radius = (edge.x - center.x).hypot(edge.y - center.y);
    if !radius.is_finite() || radius < 1e-12 {
        return None;
    }
    let segments = 64usize;
    let mut out = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = std::f64::consts::TAU * (i as f64) / (segments as f64);
        let (sin, cos) = t.sin_cos();
        out.push(Point3 {
            x: center.x + radius * cos,
            y: center.y + radius * sin,
            z: center.z,
        });
    }
    Some(out)
}

/// Build the object-snap hint overlay for `hints`.
///
/// Each hint is drawn as **one distinct marker whose shape encodes its
/// [`SnapHintKind`]**, so a user can tell an endpoint snap from a midpoint or
/// centre snap at a glance (the run-time distinction [`SnapKind::as_str`]
/// exposes in diagnostics). The whole set is a single bounded `Lines` batch at
/// [`SNAP_HINT_DRAW_ORDER`], below the live tool preview so the rubber band the
/// user is acting on stays on top.
///
/// This is separate from the axis-aligned pointer-cursor crosses that
/// [`preview_overlay`] gates on the same `view.overlays.snapHints` flag: those
/// mark the *cursor* position while drawing, these mark *resolved snap targets*.
/// An empty `hints` slice produces no batch (an explicit empty overlay, never a
/// fabricated one).
pub fn snap_hint_overlay(
    hints: &[SnapHint],
    document: DocumentId,
    options: &PreviewOptions,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    if hints.is_empty() {
        return out;
    }
    let size = options.marker_size;
    if !size.is_finite() || size <= 0.0 {
        return out;
    }
    let mut points: Vec<Point3> = Vec::with_capacity(hints.len() * snap_hint_segment_count() * 2);
    for hint in hints {
        push_snap_hint(&mut points, hint.kind, hint.point, size);
    }
    // One bounded batch; `line_batch` rejects fewer than two vertices.
    if let Some(batch) = line_batch(
        &points,
        SNAP_HINT_COLOR,
        document,
        SNAP_HINT_DRAW_ORDER,
        "overlay.snap-hint",
    ) {
        out.batches.push(batch);
    }
    out
}

/// The maximum number of line segments any single snap marker emits, used to
/// pre-size the vertex buffer without a second pass.
fn snap_hint_segment_count() -> usize {
    // `Center` is the densest marker (a tessellated circle).
    SNAP_HINT_CIRCLE_SEGMENTS.max(4)
}

/// Append one snap marker's segment endpoints to `points`.
///
/// `LineList` topology pairs consecutive vertices, so every shape is emitted as
/// explicit, closed segment pairs and shapes are independent of each other.
/// Shape by kind: endpoint → square, midpoint → diamond, center → circle,
/// quadrant → triangle, perpendicular → plus, local intersection → diagonal X.
fn push_snap_hint(points: &mut Vec<Point3>, kind: SnapHintKind, center: Point3, size: f64) {
    let offset = |dx: f64, dy: f64| Point3 {
        x: center.x + dx,
        y: center.y + dy,
        z: center.z,
    };
    match kind {
        SnapKind::Endpoint => {
            // Axis-aligned square.
            let a = offset(-size, -size);
            let b = offset(size, -size);
            let c = offset(size, size);
            let d = offset(-size, size);
            push_segment(points, a, b);
            push_segment(points, b, c);
            push_segment(points, c, d);
            push_segment(points, d, a);
        }
        SnapKind::Midpoint => {
            // Diamond (square rotated 45°).
            let top = offset(0.0, size);
            let right = offset(size, 0.0);
            let bottom = offset(0.0, -size);
            let left = offset(-size, 0.0);
            push_segment(points, top, right);
            push_segment(points, right, bottom);
            push_segment(points, bottom, left);
            push_segment(points, left, top);
        }
        SnapKind::Center => {
            // Approximated circle.
            let segments = SNAP_HINT_CIRCLE_SEGMENTS.max(3);
            let mut previous: Option<Point3> = None;
            let mut first: Option<Point3> = None;
            for i in 0..segments {
                let theta = std::f64::consts::TAU * (i as f64) / (segments as f64);
                let point = offset(theta.cos() * size, theta.sin() * size);
                if first.is_none() {
                    first = Some(point);
                }
                if let Some(previous) = previous {
                    push_segment(points, previous, point);
                }
                previous = Some(point);
            }
            if let (Some(first), Some(last)) = (first, previous) {
                push_segment(points, last, first);
            }
        }
        SnapKind::Quadrant => {
            // Upward triangle.
            let top = offset(0.0, size);
            let right = offset(size, -size * 0.8);
            let left = offset(-size, -size * 0.8);
            push_segment(points, top, right);
            push_segment(points, right, left);
            push_segment(points, left, top);
        }
        SnapKind::Perpendicular => {
            // Axis-aligned plus.
            push_segment(points, offset(-size, 0.0), offset(size, 0.0));
            push_segment(points, offset(0.0, -size), offset(0.0, size));
        }
        SnapKind::LocalIntersection => {
            // Diagonal cross (X), visually distinct from the plus.
            push_segment(points, offset(-size, -size), offset(size, size));
            push_segment(points, offset(-size, size), offset(size, -size));
        }
    }
}

/// Push one independent line segment as two consecutive vertices.
fn push_segment(points: &mut Vec<Point3>, start: Point3, end: Point3) {
    points.push(start);
    points.push(end);
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
        conics: Vec::new(),
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
        conics: Vec::new(),
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
        BlockId, DatabaseId, EntityId, InstancePath, LayerId, Mesh, ObjectId, Precision, Revision,
        SemanticGeometry, SpaceId, SubElementId, Transform3,
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

    fn flags_inputs(selection: SelectionSet) -> OverlayInputs {
        OverlayInputs {
            selection,
            ..OverlayInputs::default()
        }
    }

    #[test]
    fn entity_level_selection_is_flags_not_an_overlay_batch() {
        let db = drawing();
        let selection = SelectionSet::from_refs([model_ref(30)]);
        // No overlay batch: a whole entity is the renderer flags path.
        let overlay = selection_highlight(&db, &selection, DocumentId(1), stamp(), None);
        assert!(overlay.is_empty());
        assert!(overlay.diagnostics.is_empty());

        // The flags resolver produces one SELECTED flag for the entity.
        let scene = selection_entity_flags(&db, &flags_inputs(selection), DocumentId(1));
        assert_eq!(scene.flags, vec![(model_ref(30), cad_scene::FLAG_SELECTED)]);
        assert!(scene.diagnostics.is_empty());
        assert_eq!(scene.completeness, Completeness::Complete);
    }

    #[test]
    fn two_insert_instances_flag_independently() {
        let db = drawing();
        // Only the first placement (instance [10]) is selected.
        let scene = selection_entity_flags(
            &db,
            &flags_inputs(SelectionSet::from_refs([block_child_ref(10)])),
            DocumentId(1),
        );
        assert_eq!(
            scene.flags,
            vec![(block_child_ref(10), cad_scene::FLAG_SELECTED)]
        );

        // Both placements are distinct entity-level refs, flagged independently
        // (the renderer resolves each to its own placement's pick key).
        let scene = selection_entity_flags(
            &db,
            &flags_inputs(SelectionSet::from_refs([
                block_child_ref(10),
                block_child_ref(20),
            ])),
            DocumentId(1),
        );
        assert_eq!(scene.flags.len(), 2);
        assert_eq!(scene.flags[0].0, block_child_ref(10));
        assert_eq!(scene.flags[1].0, block_child_ref(20));
    }

    #[test]
    fn a_sub_element_selection_still_gets_an_overlay_batch() {
        let db = mesh_insert_drawing();
        let selection = SelectionSet::from_refs([mesh_child_ref(Some(face("face-a")))]);
        let overlay = selection_highlight(&db, &selection, DocumentId(2), stamp(), None);
        assert_eq!(overlay.drawn(), 1);
        assert_eq!(overlay.batches[0].topology, RenderTopology::Mesh);
        assert_eq!(
            overlay.batches[0].sources[0]
                .sub_element
                .as_ref()
                .map(|s| s.source_key.as_str()),
            Some("face-a")
        );
    }

    #[test]
    fn unresolved_entity_level_ref_is_reported_by_the_flags_resolver() {
        let db = drawing();
        let scene = selection_entity_flags(
            &db,
            &flags_inputs(SelectionSet::from_refs([model_ref(999)])),
            DocumentId(1),
        );
        assert!(
            scene.flags.is_empty(),
            "a deleted entity must not produce a fabricated flag"
        );
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "highlight.unresolved"));
        assert!(matches!(scene.completeness, Completeness::Missing(_)));
    }

    // ---- resolve-only-selected-refs contract (perf fix) -------------------

    /// The pre-fix behaviour, kept test-only as the correctness oracle: scan
    /// **every** pick item, keep the selected ones, build their representations.
    fn legacy_selection_highlight(
        database: &DrawingDatabase,
        selection: &SelectionSet,
        document: DocumentId,
        stamp: TaskStamp,
    ) -> VisualOverlay {
        let mut out = VisualOverlay::default();
        if selection.is_empty() {
            return out;
        }
        let registry = ProviderRegistry::with_default_provider();
        let context =
            RepresentationContext::new(document, cad_domain::TolerancePolicy::default(), stamp);
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
            match legacy_representation_for_item(&registry, database, item, &context) {
                Some(representation) => representations.push(representation),
                None => build_reasons.push(Diagnostic {
                    object: None,
                    code: "highlight.entity-missing".into(),
                    message: format!(
                        "selection entity {} is present in the pick index but not in the database",
                        item.source.entity.0
                    ),
                }),
            }
        }
        let HighlightScene {
            batches,
            completeness,
            diagnostics,
            ..
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

    fn legacy_representation_for_item(
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

    fn face(key: &str) -> SubElementId {
        SubElementId {
            source_key: key.into(),
            topology_revision: Revision(0),
        }
    }

    /// A two-triangle mesh with two distinct stable face sources.
    fn two_face_mesh() -> Mesh {
        let p3 = |x: f64, y: f64| Point3 { x, y, z: 0.0 };
        Mesh {
            vertices: vec![
                p3(-2.0, -1.0),
                p3(-1.0, 1.0),
                p3(0.0, -1.0),
                p3(0.0, -1.0),
                p3(1.0, 1.0),
                p3(2.0, -1.0),
            ],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
            normals: Vec::new(),
            face_sources: vec![Some(face("face-a")), Some(face("face-b"))],
            colors: Vec::new(),
        }
    }

    /// A block whose only member is a two-face mesh, placed once at x = 10.
    fn mesh_insert_drawing() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(2));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(300)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(entity(300, SemanticGeometry::Mesh(two_face_mesh()), 0))
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
        b.finish().unwrap()
    }

    fn mesh_child_ref(sub_element: Option<SubElementId>) -> SelectionRef {
        SelectionRef {
            document: DocumentId(2),
            entity: EntityId(300),
            instance: InstancePath(vec![EntityId(10)]),
            sub_element,
        }
    }

    /// The resolve-only sub-element path must be byte-for-byte identical to the
    /// old full-scan path for every sub-element selection (mesh face, duplicate
    /// `(entity, instance)`, missing entity, broken instance path). Entity-level
    /// refs are the flags path and never reach `highlight_batches`, so they are
    /// exercised by `selection_entity_flags` tests instead.
    #[test]
    fn resolve_only_selected_refs_matches_the_full_scan_for_sub_elements() {
        let mesh_db = mesh_insert_drawing();
        let mesh_selections = [
            SelectionSet::new(),
            SelectionSet::from_refs([mesh_child_ref(None)]),
            SelectionSet::from_refs([mesh_child_ref(Some(face("face-a")))]),
            SelectionSet::from_refs([mesh_child_ref(Some(face("face-b")))]),
            // The same object selected both whole and by face: the scan matched
            // one item for both refs; the resolve path must resolve it once.
            SelectionSet::from_refs([mesh_child_ref(None), mesh_child_ref(Some(face("face-a")))]),
        ];
        for selection in mesh_selections {
            let fast = selection_highlight(&mesh_db, &selection, DocumentId(2), stamp(), None);
            let legacy = legacy_selection_highlight(&mesh_db, &selection, DocumentId(2), stamp());
            assert_eq!(fast, legacy, "mesh selection {selection:?} diverged");
        }
    }

    /// A camera settle re-prepares the transient overlay with a new `TaskStamp`
    /// (the next scene revision) over the same selection and database. The
    /// sub-element highlight is derived **only** from the selection and the
    /// database, so it must be byte-identical and add no diagnostic: a camera-only
    /// operation cannot degrade it. A `highlight.unresolved` requires a ref the
    /// database no longer resolves, i.e. a scene mutation.
    #[test]
    fn sub_element_highlight_is_identical_across_a_camera_settle() {
        let db = mesh_insert_drawing();
        let selection = SelectionSet::from_refs([mesh_child_ref(Some(face("face-a")))]);
        let before = selection_highlight(&db, &selection, DocumentId(2), stamp(), None);
        assert_eq!(
            before.drawn(),
            1,
            "the face selection must produce one batch"
        );
        assert!(before.diagnostics.is_empty());

        // The settle commit re-prepares the same inputs with the next revision.
        let settled = TaskStamp::new(DocumentId(2), 2);
        let after = selection_highlight(&db, &selection, DocumentId(2), settled, None);
        assert_eq!(
            after, before,
            "a camera settle must not change the sub-element highlight"
        );
        assert!(
            after.diagnostics.is_empty(),
            "a camera settle must not add a highlight diagnostic"
        );
    }

    /// An `INSERT` root only ever exists as an expansion, so selecting it is
    /// unresolved — never a fabricated flag of the block's contents.
    #[test]
    fn insert_root_is_unresolved_for_the_flags_resolver() {
        let db = drawing();
        let scene = selection_entity_flags(
            &db,
            &flags_inputs(SelectionSet::from_refs([model_ref(10)])),
            DocumentId(1),
        );
        assert!(scene.flags.is_empty());
        assert!(scene
            .diagnostics
            .iter()
            .any(|d| d.code == "highlight.unresolved"));
    }

    /// A whole-entity and a face selection of the same object: the entity is a
    /// flag and the face is the single overlay batch, so neither duplicates.
    #[test]
    fn a_face_and_its_entity_produce_one_overlay_batch_and_one_flag() {
        let db = mesh_insert_drawing();
        let selection =
            SelectionSet::from_refs([mesh_child_ref(None), mesh_child_ref(Some(face("face-a")))]);
        let overlay = selection_highlight(&db, &selection, DocumentId(2), stamp(), None);
        // Only the face-a batch; the whole entity is a flag, not a batch.
        assert_eq!(overlay.drawn(), 1);
        let scene = selection_entity_flags(&db, &flags_inputs(selection), DocumentId(2));
        assert_eq!(
            scene.flags,
            vec![(mesh_child_ref(None), cad_scene::FLAG_SELECTED)]
        );
    }

    #[test]
    fn empty_preview_yields_no_batches() {
        let overlay = preview_overlay(None, DocumentId(1), true, &PreviewOptions::default());
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
            DocumentId(1),
            true,
            &PreviewOptions::default(),
        );
        assert!(!active.is_empty());
        let cancelled = preview_overlay(None, DocumentId(1), true, &PreviewOptions::default());
        assert!(cancelled.is_empty());
    }

    #[test]
    fn overlay_fingerprint_tracks_sub_element_selection_and_cursor() {
        let mut a = OverlayInputs::default();
        let base = overlay_fingerprint(&a);
        a.selection = SelectionSet::from_refs([mesh_child_ref(Some(face("face-a")))]);
        let selected = overlay_fingerprint(&a);
        assert_ne!(base, selected);
        a.selection.replace([mesh_child_ref(Some(face("face-b")))]);
        assert_ne!(selected, overlay_fingerprint(&a));

        // An entity-level selection is the flags path: it must NOT change the
        // overlay fingerprint (so no overlay geometry is rebuilt/re-uploaded),
        // only the flags fingerprint.
        let mut entity_only = OverlayInputs::default();
        let overlay_before = overlay_fingerprint(&entity_only);
        let flags_before = entity_flags_fingerprint(&entity_only);
        entity_only.selection = SelectionSet::from_refs([block_child_ref(10)]);
        assert_eq!(overlay_fingerprint(&entity_only), overlay_before);
        assert_ne!(entity_flags_fingerprint(&entity_only), flags_before);

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
    fn entity_flags_fingerprint_tracks_hover_and_hidden() {
        let base = entity_flags_fingerprint(&OverlayInputs::default());
        let hovered = OverlayInputs {
            hovered: SelectionSet::from_refs([block_child_ref(10)]),
            ..OverlayInputs::default()
        };
        let hidden = OverlayInputs {
            hidden: SelectionSet::from_refs([block_child_ref(10)]),
            ..OverlayInputs::default()
        };
        assert_ne!(entity_flags_fingerprint(&hovered), base);
        assert_ne!(entity_flags_fingerprint(&hidden), base);
        assert_ne!(
            entity_flags_fingerprint(&hovered),
            entity_flags_fingerprint(&hidden),
            "hover and hidden are distinct flag kinds"
        );
    }

    #[test]
    fn overlay_fingerprint_tracks_each_visibility_flag() {
        let baseline = overlay_fingerprint(&OverlayInputs::default());
        let flips: [fn(&mut OverlayVisibility); 4] = [
            |v| v.axes = false,
            |v| v.grid = false,
            |v| v.selection_highlight = false,
            |v| v.snap_hints = false,
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

    fn hint(kind: SnapHintKind, point: Point3) -> SnapHint {
        SnapCandidate {
            kind,
            point,
            space: SpaceId::Model,
            source: model_ref(1),
            secondary: None,
            precision: Precision::Analytic,
            logical_pixel_distance: 0.0,
        }
    }

    fn snap_hint_kinds() -> [SnapHintKind; 6] {
        [
            SnapKind::Endpoint,
            SnapKind::Midpoint,
            SnapKind::Center,
            SnapKind::Quadrant,
            SnapKind::Perpendicular,
            SnapKind::LocalIntersection,
        ]
    }

    /// Local (origin-relative) segment pairs of a `LineList` batch.
    fn segment_pairs(batch: &RenderBatch) -> Vec<([f32; 3], [f32; 3])> {
        batch
            .vertices
            .chunks_exact(2)
            .map(|pair| (pair[0], pair[1]))
            .collect()
    }

    #[test]
    fn empty_snap_hints_yield_no_batch() {
        let overlay = snap_hint_overlay(&[], DocumentId(1), &PreviewOptions::default());
        assert!(overlay.is_empty());
        assert_eq!(overlay.completeness, Completeness::Complete);
        assert!(overlay.diagnostics.is_empty());
    }

    #[test]
    fn snap_hint_overlay_is_one_bounded_batch_below_the_preview() {
        let hints = [
            hint(SnapKind::Endpoint, p(0.0, 0.0)),
            hint(SnapKind::Center, p(5.0, 0.0)),
        ];
        let overlay = snap_hint_overlay(&hints, DocumentId(1), &PreviewOptions::default());
        assert_eq!(overlay.drawn(), 1, "all hints share one bounded batch");
        let batch = &overlay.batches[0];
        assert_eq!(batch.topology, RenderTopology::Lines);
        assert_eq!(batch.color, SNAP_HINT_COLOR);
        assert_eq!(batch.draw_order, SNAP_HINT_DRAW_ORDER);
        assert!(batch.draw_order > 900_000 && batch.draw_order < PREVIEW_DRAW_ORDER);
        // LineList: segment pairs are explicit; the count is even.
        assert_eq!(batch.vertices.len() % 2, 0);
        // Endpoint square (4) + center circle (12) segments.
        assert_eq!(segment_pairs(batch).len(), 4 + SNAP_HINT_CIRCLE_SEGMENTS);
        assert!(!batch.color_unresolved);
    }

    #[test]
    fn each_snap_kind_produces_a_distinct_marker_shape() {
        let kinds = snap_hint_kinds();
        let mut shapes: Vec<Vec<[f32; 3]>> = Vec::new();
        for kind in kinds {
            let overlay = snap_hint_overlay(
                &[hint(kind, p(1.0, 2.0))],
                DocumentId(1),
                &PreviewOptions::default(),
            );
            assert_eq!(overlay.drawn(), 1, "{kind:?} must draw exactly one batch");
            let batch = &overlay.batches[0];
            let segments = segment_pairs(batch);
            assert!(!segments.is_empty(), "{kind:?} must emit segments");
            // Every marker is drawn around the hint point, not at the origin.
            for (a, b) in &segments {
                for vertex in [a, b] {
                    let x = batch.local_origin.x + vertex[0] as f64;
                    let y = batch.local_origin.y + vertex[1] as f64;
                    assert!(
                        (x - 1.0).abs() <= 0.5 + 1e-9 && (y - 2.0).abs() <= 0.5 + 1e-9,
                        "{kind:?} vertex {vertex:?} off the marker centre"
                    );
                }
            }
            // Local (origin-relative) vertices, directly comparable per kind.
            shapes.push(batch.vertices.clone());
        }
        for i in 0..shapes.len() {
            for j in (i + 1)..shapes.len() {
                assert_ne!(
                    shapes[i],
                    shapes[j],
                    "snap kinds {} and {} must not share a marker shape",
                    kinds[i].as_str(),
                    kinds[j].as_str()
                );
            }
        }
    }

    #[test]
    fn snap_hint_markers_scale_with_the_marker_size() {
        let small = snap_hint_overlay(
            &[hint(SnapKind::Endpoint, p(0.0, 0.0))],
            DocumentId(1),
            &PreviewOptions {
                marker_size: 0.25,
                ..PreviewOptions::default()
            },
        );
        let large = snap_hint_overlay(
            &[hint(SnapKind::Endpoint, p(0.0, 0.0))],
            DocumentId(1),
            &PreviewOptions {
                marker_size: 1.0,
                ..PreviewOptions::default()
            },
        );
        let extent = |overlay: &VisualOverlay| {
            overlay.batches[0]
                .vertices
                .iter()
                .map(|v| v[0].abs().max(v[1].abs()))
                .fold(0.0f32, f32::max)
        };
        assert!(extent(&large) > extent(&small));
    }

    #[test]
    fn overlay_fingerprint_tracks_snap_hints() {
        let baseline = overlay_fingerprint(&OverlayInputs::default());
        let with_hint = OverlayInputs {
            snap_hints_input: vec![hint(SnapKind::Endpoint, p(0.0, 0.0))],
            ..OverlayInputs::default()
        };
        let hashed = overlay_fingerprint(&with_hint);
        assert_ne!(baseline, hashed);
        // The kind is part of the fingerprint even at the same point.
        let other_kind = OverlayInputs {
            snap_hints_input: vec![hint(SnapKind::Midpoint, p(0.0, 0.0))],
            ..OverlayInputs::default()
        };
        assert_ne!(hashed, overlay_fingerprint(&other_kind));
        // The point is part of the fingerprint even at the same kind.
        let other_point = OverlayInputs {
            snap_hints_input: vec![hint(SnapKind::Endpoint, p(1.0, 0.0))],
            ..OverlayInputs::default()
        };
        assert_ne!(hashed, overlay_fingerprint(&other_point));
        // Rebuilding with identical hints is stable.
        assert_eq!(hashed, overlay_fingerprint(&with_hint));
    }

    #[test]
    fn axes_overlay_draws_a_symmetric_origin_cross_below_the_highlight() {
        let overlay = axes_overlay((p(-2.0, -3.0), p(4.0, 5.0)), DocumentId(1));
        assert_eq!(overlay.drawn(), 1);
        let batch = &overlay.batches[0];
        assert_eq!(batch.topology, RenderTopology::Lines);
        // Each axis is symmetric about the origin: X spans |-4|..|4| (the larger
        // absolute x bound), Y spans |-5|..|5|, so both arms of each axis are
        // equal regardless of which side the drawing sits on.
        assert_eq!(batch.vertices.len(), 4);
        assert_eq!(batch.local_origin, p(-4.0, 0.0));
        assert_eq!(batch.vertices[0], [0.0, 0.0, 0.0]);
        assert_eq!(batch.vertices[1], [8.0, 0.0, 0.0]);
        assert_eq!(batch.vertices[2], [4.0, -5.0, 0.0]);
        assert_eq!(batch.vertices[3], [4.0, 5.0, 0.0]);
        assert_eq!(batch.color, AXES_COLOR);
        assert!(batch.draw_order < 900_000);
        assert!(!batch.color_unresolved);
    }

    #[test]
    fn grid_overlay_is_one_bounded_batch_spanning_the_supplied_view_bounds() {
        let overlay = grid_overlay((p(-50.0, -50.0), p(50.0, 50.0)), DocumentId(1));
        assert_eq!(overlay.drawn(), 1);
        let batch = &overlay.batches[0];
        assert!(!batch.vertices.is_empty());
        assert_eq!(batch.vertices.len() % 2, 0);
        assert!(batch.vertices.len() / 2 <= GRID_MAX_LINES);
        assert_eq!(batch.color, GRID_COLOR);
        assert!(batch.draw_order < 900_000);
        // Geometry spans the supplied view rectangle; line batches are clipped
        // to this exact extent by the renderer for every live camera/resize.
        let world_points = batch.vertices.iter().map(|v| {
            (
                v[0] as f64 + batch.local_origin.x,
                v[1] as f64 + batch.local_origin.y,
            )
        });
        let (min_x, max_x, min_y, max_y) = world_points.fold(
            (
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ),
            |(min_x, max_x, min_y, max_y), (x, y)| {
                (min_x.min(x), max_x.max(x), min_y.min(y), max_y.max(y))
            },
        );
        assert!((min_x + 50.0).abs() < 1e-4 && (max_x - 50.0).abs() < 1e-4);
        assert!((min_y + 50.0).abs() < 1e-4 && (max_y - 50.0).abs() < 1e-4);
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
    fn grid_overlay_remains_bounded_at_extreme_pan_and_zoom() {
        let overlay = grid_overlay(
            (p(1.0e12, -1.0e12), p(1.0e12 + 1.0e-2, -1.0e12 + 1.0e-2)),
            DocumentId(1),
        );
        let batch = &overlay.batches[0];
        assert!(!batch.vertices.is_empty());
        assert!(batch.vertices.len() / 2 <= GRID_MAX_LINES);
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
            DocumentId(1),
            true,
            &PreviewOptions::default(),
        );
        // Rubber-band chain + point-marker crosses.
        assert_eq!(with.drawn(), 2);
        let without = preview_overlay(
            Some(&preview),
            DocumentId(1),
            false,
            &PreviewOptions::default(),
        );
        // The chain stays; only the cursor/snap crosses disappear.
        assert_eq!(without.drawn(), 1);
        assert_eq!(without.batches[0].topology, RenderTopology::Lines);
    }
}
