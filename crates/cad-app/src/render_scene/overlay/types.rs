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
}

impl OverlayInputs {
    /// Whether every overlay input is empty (no highlight, no hover/hidden
    /// flags, no preview, no box selection, no resolved snap hints).
    pub fn is_empty(&self) -> bool {
        self.selection.is_empty()
            && self.hovered.is_empty()
            && self.hidden.is_empty()
            && self.measurement.is_none()
            && self.draw_preview.is_none()
            && self.selection_box.is_none()
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
