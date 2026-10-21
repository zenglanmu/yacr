use super::*;

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

/// Half-extent, in **logical pixels**, of an object-snap marker glyph.
///
/// A snap marker is a fixed **screen** affordance: its shape (square, triangle,
/// circle, diamond, perpendicular, diagonal X — see `push_snap_hint`) must read
/// the same at any zoom. The glyph spans twice this value (**18** logical px).
///
/// The size is deliberately the **dominant** shape at the snap point: the pickbox
/// is only `2 × TolerancePolicy::pickbox_logical_pixels` = **6** logical px across
/// at the default 3 (it marks what a click can hit, so it tracks the configured
/// `view.pickbox.sizePixels`), giving a **3 : 1** marker-to-pickbox ratio — the
/// same order as AutoCAD, where the osnap marker reads as the large glyph and the
/// pickbox is a small square inside it. A user who enlarges the pickbox shrinks
/// that ratio, exactly as AutoCAD's own two options do.
///
/// At the previous 10 px the marker and the pickbox were the same order of size
/// and a reader could not tell the snap type from the frame.
///
/// The world-space half-extent handed to the overlay is this pixel size times the
/// current view scale (see [`PreviewOptions::for_view_scale`]), so the marker does
/// not drift with zoom the way a fixed world size does.
pub const SNAP_MARKER_HALF_EXTENT_PIXELS: f64 = 9.0;

/// `draw_order` of the object-snap hint markers.
///
/// **Above every other overlay**, including the pointer cursor crosshair and its
/// pickbox ([`CURSOR_DRAW_ORDER`], `2_500_000`) and the tool preview
/// ([`PREVIEW_DRAW_ORDER`], `2_000_000`). The snap marker is what the user
/// actually reads at the snap point: it must never be painted over by the
/// crosshair's pickbox (which sits at the pointer, i.e. on the same point when
/// the pointer is on the snap target) nor by the rubber band the marker anchors.
/// This is the AutoCAD behaviour — the osnap glyph is the dominant shape at the
/// snap point — and it is what makes the snap *type* legible at a glance.
pub const SNAP_HINT_DRAW_ORDER: i64 = 3_000_000;

/// Colour of the object-snap hint markers: a warm amber, distinct from the
/// selection highlight tint and from both preview colours.
pub const SNAP_HINT_COLOR: [f32; 3] = [1.0, 0.72, 0.16];

/// Number of segments used to approximate the `Center` snap marker circle.
pub(super) const SNAP_HINT_CIRCLE_SEGMENTS: usize = 12;

/// `draw_order` of the live box-selection rubber band.
///
/// Below the tool preview (`PREVIEW_DRAW_ORDER`, 2_000_000) and above the
/// object-snap hints (`SNAP_HINT_DRAW_ORDER`, 1_500_000): the snap marker the
/// user is aiming at stays visible, the live draw rubber band stays on top.
pub const BOX_SELECT_DRAW_ORDER: i64 = 1_750_000;

/// AutoCAD paints a WINDOW selection rectangle blue (left-to-right drag).
pub const BOX_SELECT_WINDOW_COLOR: [f32; 3] = [0.29, 0.56, 0.95];
/// AutoCAD paints a CROSSING rectangle green (right-to-left drag).
pub const BOX_SELECT_CROSSING_COLOR: [f32; 3] = [0.35, 0.85, 0.45];

/// The live box-selection rubber band.
///
/// The host tessellates the rectangle (and any dashing) into world-space
/// segments on the work plane, so this crate never guesses a dash length in
/// screen pixels it cannot see. Purely transient: it never writes the
/// database, the revision or the history.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxSelectionPreview {
    pub segments: Vec<[Point3; 2]>,
    /// `true` = CROSSING (right-to-left, green); `false` = WINDOW (blue).
    pub crossing: bool,
}

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

impl PreviewOptions {
    /// Preview options whose marker half-extent is a fixed **screen** size.
    ///
    /// `view_scale` is the current view scale in world units per logical pixel
    /// (the same quantity `Viewport::world_per_px` reports). The world-space
    /// half-extent is [`SNAP_MARKER_HALF_EXTENT_PIXELS`] logical pixels times
    /// that scale, so every marker glyph keeps a constant on-screen size under
    /// any zoom (`marker_size` is documented as world units).
    ///
    /// A missing (`None`) or degenerate (non-finite, non-positive) scale cannot
    /// define a screen-relative size, so the documented world default
    /// ([`DEFAULT_PREVIEW_MARKER_SIZE`]) is used rather than producing a
    /// zero/NaN size; a positive scale whose product would underflow to zero is
    /// floored at the smallest positive `f64`. The result is therefore always
    /// finite and strictly positive.
    pub fn for_view_scale(view_scale: Option<f64>) -> Self {
        let marker_size = match view_scale {
            Some(scale) if scale.is_finite() && scale > 0.0 => {
                (SNAP_MARKER_HALF_EXTENT_PIXELS * scale).max(f64::MIN_POSITIVE)
            }
            _ => DEFAULT_PREVIEW_MARKER_SIZE,
        };
        PreviewOptions {
            draw_order: PREVIEW_DRAW_ORDER,
            marker_size,
        }
    }
}

/// The transient highlight / preview overlay inputs a host supplies.
///
/// All fields are pure session state; none of them writes the database. An
/// empty selection and a `None` preview produce no overlay batches, which is
/// an explicit empty overlay rather than an error.
#[derive(Debug, Clone, PartialEq)]
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
    /// Live box-selection rubber band (ADR 0014 decision B.9).
    ///
    /// Gesture feedback, not a selection tint: it is drawn whenever present and
    /// is **not** gated by `visibility.selection_highlight`. `None` by default,
    /// so a host that never drags a selection box gets no batch.
    pub selection_box: Option<BoxSelectionPreview>,
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
    /// Positions of the visible POINT entities, drawn as screen-relative cross
    /// markers (ADR 0015 decision E).
    ///
    /// Filled by the scene assembly from the drawing being displayed
    /// (`CadSceneController::prepare_*`), respecting the same layer-visibility
    /// overrides the rest of the overlay uses; a host normally leaves it empty.
    /// It participates in [`overlay_fingerprint`], so a changed point set
    /// rebuilds the transient overlay, and the marker size is derived from
    /// [`Self::viewport_bounds`] (see `point_marker_overlay`). Empty by default.
    pub point_markers: Vec<Point3>,
    /// Current view scale in **world units per logical pixel** of the active 2D
    /// view (the same quantity `Viewport::world_per_px` reports), used to size
    /// the screen-relative overlays — the object-snap markers
    /// ([`PreviewOptions::for_view_scale`]) and the pointer cursor crosshair and
    /// pickbox (`cursor_overlay`) — from a fixed logical-pixel size. `None` in a
    /// 3D view or with no valid view, so a screen-relative size has no domain.
    ///
    /// It is derived from the camera, so it changes exactly when
    /// [`Self::viewport_bounds`] does; it is therefore deliberately **not** part
    /// of [`overlay_fingerprint`] (the bounds already track a camera change, so
    /// the overlay still rebuilds on zoom without adding a second camera signal).
    pub view_scale: Option<f64>,
    /// The pointer cursor's world position on the work plane, or `None` when
    /// there is no cursor (the pointer is not on the surface, or the view is 3D
    /// and has no planar cursor). Drawn as a viewport-spanning crosshair with a
    /// square pickbox at this point (see `cursor_overlay`); `None` produces no
    /// cursor batch — an explicit empty overlay, never a fabricated one. It is
    /// part of [`overlay_fingerprint`], so a moved cursor rebuilds the transient
    /// overlay so the crosshair follows it.
    pub cursor: Option<Point3>,
    /// Crosshair arm length as an integer percentage of each arm's full
    /// cursor-to-edge span ([`DEFAULT_CROSSHAIR_PERCENT`] is the historical
    /// full-viewport crosshair). The pickbox is deliberately unaffected: it is
    /// the pick tolerance, not a display preference. Part of
    /// [`overlay_fingerprint`], so a resized crosshair rebuilds the overlay.
    pub crosshair_percent: u32,
    /// Pickbox half-side in logical pixels
    /// ([`cad_domain::TolerancePolicy::pickbox_logical_pixels`], the AutoCAD
    /// `PICKBOX` analogue). It is the configured pickbox size, mapped from the
    /// effective `view.pickbox.sizePixels` by the host, so the drawn box and the
    /// click predicate can never disagree. Part of [`overlay_fingerprint`], so a
    /// resized pickbox rebuilds the overlay.
    pub pickbox_logical_pixels: f64,
}

impl Default for OverlayInputs {
    fn default() -> Self {
        Self {
            selection: SelectionSet::default(),
            hovered: SelectionSet::default(),
            hidden: SelectionSet::default(),
            measurement: None,
            draw_preview: None,
            selection_box: None,
            snap_hints_input: Vec::new(),
            visibility: OverlayVisibility::default(),
            viewport_bounds: None,
            point_markers: Vec::new(),
            view_scale: None,
            cursor: None,
            crosshair_percent: DEFAULT_CROSSHAIR_PERCENT,
            pickbox_logical_pixels: cad_domain::TolerancePolicy::default().pickbox_logical_pixels,
        }
    }
}

impl OverlayInputs {
    /// Whether every overlay input is empty (no highlight, no hover/hidden
    /// flags, no preview, no box selection, no resolved snap hints, no visible
    /// POINT markers, no pointer cursor).
    pub fn is_empty(&self) -> bool {
        self.selection.is_empty()
            && self.hovered.is_empty()
            && self.hidden.is_empty()
            && self.measurement.is_none()
            && self.draw_preview.is_none()
            && self.selection_box.is_none()
            && self.snap_hints_input.is_empty()
            && self.point_markers.is_empty()
            && self.cursor.is_none()
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
