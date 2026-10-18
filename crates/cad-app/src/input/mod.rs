//! Shared input, DPI, IME and status policy (audit U05–U08).
//!
//! Platform hosts differ wildly in how they deliver pointers, keys and surface
//! metrics, but the *decisions* they must make are
//! identical: does this pointer start a draw or a pan, is a multi-touch gesture
//! allowed to commit, does Esc cancel a tool or leave the app, and where does a
//! physical pixel land in world space. Keeping that policy here — pure, host
//! agnostic and unit-tested — is what stops each host from inventing its own
//! slightly-wrong answer (audit U05/U06/U07).
//!
//! Three rules are hard and are enforced by construction, not by convention:
//!
//! 1. A multi-touch gesture **never** commits a measurement. The
//!    moment a second contact appears the in-progress tool is cancelled
//!    ([`InputPolicy::handle`]).
//! 2. While a text field has focus or an IME is composing, canvas shortcuts are
//!    suppressed and composing text is never read as a command.
//! 3. Drawing and picking share one metrics mapping ([`ViewMetrics::surface_to_world`]),
//!    so a click and a render of the same pixel agree.

use cad_diagnostics::{DiagnosticsModel, DiagnosticsSummary};

use crate::{SessionState, ToolState, Viewport};
use cad_domain::{CadResult, Point3, WorkPlane};

/// A pointer update delivered by a host in logical pixels (y grows downward).
///
/// `contacts` is the total number of simultaneous contacts after this update,
/// `1` for a mouse drag. This is the only identity the policy needs: the drag
/// threshold and the draw/pan split key off contact count plus distance, not on
/// which physical finger moved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerUpdate {
    pub logical_position: [f64; 2],
    pub contacts: u8,
}

/// What the policy decided an input means. Hosts translate this into commands;
/// the policy never dispatches one itself (except the explicit cancel path,
/// which is documented below).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputOutcome {
    /// Nothing to do (for example a two-finger pan update).
    Ignored,
    /// Begin a navigate gesture (two-finger pan) at this logical position.
    BeginPan { logical: [f64; 2] },
    /// Continue a pan by a logical-pixel delta.
    Pan { delta: [f64; 2] },
    /// Commit the active draw/capture tool at this logical position.
    Commit { logical: [f64; 2] },
    /// Move a preview cursor without committing.
    Preview { logical: [f64; 2] },
    /// Begin a box zoom (whole multi-touch gesture or pinch) about this centre.
    BeginZoom { logical: [f64; 2] },
    /// Cancel the active tool: multi-touch took over, or a pointer was
    /// cancelled by the OS (gesture stolen by the system / browser).
    ToolCancelled { reason: CancelReason },
}

/// Why a tool was cancelled. Kept explicit so a host can show the right hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// A second contact arrived: multi-touch navigation wins, never a commit.
    MultiTouch,
    /// The OS/browser cancelled the pointer (for example an Android gesture was
    /// claimed, or the Web `pointercancel` fired).
    PointerCancelled,
}

/// Drag threshold in logical pixels before a single contact counts as a draw.
///
/// Chosen (audit U05): below this the pointer is treated as a tap/noise and only
/// moves a preview; at or above it the point is captured. 96px targets are the
/// minimum touch target (audit U01), so a threshold of a few pixels avoids
/// committing on the first jitter of a finger. A mouse click is a zero-length
/// drag and therefore commits on release without moving.
pub const DRAG_THRESHOLD_LOGICAL_PX: f64 = 4.0;

/// Pointer/gesture state machine shared by every host (audit U05).
///
/// ```text
/// Idle ──1 contact down──▶ Pending ──moved ≥ threshold──▶ Dragging
///   ▲                        │                              │
///   │      pointer cancel     │ 2nd contact                 │ 2nd contact
///   └────────────────────────┴──────────────▶ Navigate ◀────┘
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub enum PointerPhase {
    /// No contact.
    #[default]
    Idle,
    /// One contact down, not yet past the drag threshold. `anchor` is the down
    /// position, `last` the latest position.
    Pending { anchor: [f64; 2], last: [f64; 2] },
    /// One contact drawing: the anchor has been committed as a point and moves
    /// update the preview cursor.
    Dragging { anchor: [f64; 2], last: [f64; 2] },
    /// Two or more contacts navigating (pan/zoom). Never commits a tool.
    Navigate { last: [f64; 2] },
}

/// The pointer/gesture policy plus the keyboard/IME focus state.
///
/// Holds the state that is genuinely shared: the pointer phase and whether a
/// text field currently owns focus (including an active IME composition).
#[derive(Debug, Clone, PartialEq)]
pub struct InputPolicy {
    phase: PointerPhase,
    /// True when a text field (search box, annotation text, dialog) has focus.
    text_focus: bool,
    /// True while the IME is composing characters (pinyin/kanji preedit).
    composing: bool,
}

impl Default for InputPolicy {
    fn default() -> Self {
        InputPolicy::new()
    }
}

impl InputPolicy {
    pub fn new() -> Self {
        InputPolicy {
            phase: PointerPhase::Idle,
            text_focus: false,
            composing: false,
        }
    }

    pub fn phase(&self) -> &PointerPhase {
        &self.phase
    }

    /// Record that a text field gained/lost focus. Losing focus also clears any
    /// composition, so a cancelled IME can never leak into a later shortcut.
    pub fn set_text_focus(&mut self, focused: bool) {
        self.text_focus = focused;
        if !focused {
            self.composing = false;
        }
    }

    pub fn text_focus(&self) -> bool {
        self.text_focus
    }

    /// Record IME composition start/update/end.
    pub fn set_composing(&mut self, composing: bool) {
        self.composing = composing;
    }

    pub fn composing(&self) -> bool {
        self.composing
    }

    /// Whether canvas keyboard shortcuts are currently suppressed (audit U06).
    ///
    /// True while a text field has focus **or** an IME composition is active, so
    /// a pinyin preedit is never interpreted as a command.
    pub fn shortcuts_suppressed(&self) -> bool {
        self.text_focus || self.composing
    }

    /// Feed one pointer update and get the decided outcome.
    ///
    /// This is the single transition function for the state machine. Multi-touch
    /// cancels an in-progress tool and enters navigation; a pointer cancel exits
    /// to idle and cancels any open tool.
    pub fn handle(&mut self, update: PointerUpdate) -> InputOutcome {
        if !finite2(update.logical_position) {
            // A non-finite position is never a valid gesture; drop it rather
            // than poison the phase with NaN.
            return InputOutcome::Ignored;
        }
        match update.contacts {
            0 => self.handle_up(update.logical_position),
            1 => self.handle_single(update.logical_position),
            _ => self.handle_multi(update.logical_position),
        }
    }

    /// An explicit pointer-cancel from the host (OS stole the gesture).
    pub fn pointer_cancelled(&mut self) -> InputOutcome {
        let was_active = !matches!(self.phase, PointerPhase::Idle);
        self.phase = PointerPhase::Idle;
        if was_active {
            InputOutcome::ToolCancelled {
                reason: CancelReason::PointerCancelled,
            }
        } else {
            InputOutcome::Ignored
        }
    }

    /// Force the policy back to idle (used on tool confirm/cancel, mode switch).
    pub fn reset(&mut self) {
        self.phase = PointerPhase::Idle;
    }

    fn handle_up(&mut self, logical: [f64; 2]) -> InputOutcome {
        let previous = std::mem::replace(&mut self.phase, PointerPhase::Idle);
        match previous {
            // A tap with no movement: commit at the down position. A mouse click
            // is exactly this (down then up with no drag).
            PointerPhase::Pending { anchor, .. } => InputOutcome::Commit { logical: anchor },
            // A drag release commits the point that was already captured at the
            // start of the drag, not a second point.
            PointerPhase::Dragging { anchor, .. } => InputOutcome::Commit { logical: anchor },
            // Releasing during navigation changes nothing: no commit ever.
            PointerPhase::Navigate { .. } => InputOutcome::Ignored,
            // A stray up with no down: drop it.
            PointerPhase::Idle => {
                let _ = logical;
                InputOutcome::Ignored
            }
        }
    }

    fn handle_single(&mut self, logical: [f64; 2]) -> InputOutcome {
        match self.phase.clone() {
            PointerPhase::Idle => {
                self.phase = PointerPhase::Pending {
                    anchor: logical,
                    last: logical,
                };
                InputOutcome::Preview { logical }
            }
            PointerPhase::Pending { anchor, .. } => {
                if distance(anchor, logical) >= DRAG_THRESHOLD_LOGICAL_PX {
                    self.phase = PointerPhase::Dragging {
                        anchor,
                        last: logical,
                    };
                    // Crossing the threshold is the commit: the anchor point is
                    // captured and subsequent moves only move the preview.
                    InputOutcome::Commit { logical: anchor }
                } else {
                    self.phase = PointerPhase::Pending {
                        anchor,
                        last: logical,
                    };
                    InputOutcome::Preview { logical }
                }
            }
            PointerPhase::Dragging { anchor, .. } => {
                self.phase = PointerPhase::Dragging {
                    anchor,
                    last: logical,
                };
                InputOutcome::Preview { logical }
            }
            // A single contact during a navigate gesture is just the remaining
            // finger lifting; do not snap back into drawing.
            PointerPhase::Navigate { last } => {
                self.phase = PointerPhase::Navigate { last };
                InputOutcome::Ignored
            }
        }
    }

    fn handle_multi(&mut self, logical: [f64; 2]) -> InputOutcome {
        let previous = std::mem::replace(&mut self.phase, PointerPhase::Navigate { last: logical });
        let cancels = matches!(
            previous,
            PointerPhase::Pending { .. } | PointerPhase::Dragging { .. }
        );
        if cancels {
            // Hard rule: multi-touch wins over draw. The tool is cancelled, not
            // committed, and the gesture becomes navigation.
            InputOutcome::ToolCancelled {
                reason: CancelReason::MultiTouch,
            }
        } else {
            let delta = match previous {
                PointerPhase::Navigate { last } => [logical[0] - last[0], logical[1] - last[1]],
                _ => [0.0, 0.0],
            };
            InputOutcome::Pan { delta }
        }
    }
}

fn finite2(p: [f64; 2]) -> bool {
    p[0].is_finite() && p[1].is_finite()
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

/// What Esc should do (audit U05 "先取消工具，再处理退出").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeAction {
    /// A tool/gesture was active: cancel it and stay.
    CancelTool,
    /// A text field / IME has focus: Esc dismisses that first, not the canvas.
    DismissTextFocus,
    /// Nothing to cancel: the host may now handle back/exit.
    Exit,
}

/// Decide what Esc (or Android back) means right now.
///
/// Priority: a focused text field / active IME consumes Esc first; otherwise an
/// active canvas tool is cancelled; only when nothing is active may the host
/// treat Esc as back/exit.
pub fn escape_action(policy: &InputPolicy, session: &SessionState) -> EscapeAction {
    if policy.shortcuts_suppressed() {
        return EscapeAction::DismissTextFocus;
    }
    if matches!(session.tool, ToolState::Idle) {
        EscapeAction::Exit
    } else {
        EscapeAction::CancelTool
    }
}

/// A keyboard event as the policy needs to see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Suppressed: a text field / IME owns the key.
    Suppressed,
    /// Esc with the canvas owning focus.
    Escape,
    /// An ordinary canvas shortcut (letter/accelerator).
    Shortcut,
}

/// Classify a key press. `is_escape` distinguishes Esc from ordinary shortcuts.
///
/// Composing input is never a shortcut: a pinyin preedit that happens to contain
/// "p" must not pan (audit U06).
pub fn classify_key(policy: &InputPolicy, is_escape: bool) -> KeyAction {
    if policy.shortcuts_suppressed() {
        KeyAction::Suppressed
    } else if is_escape {
        KeyAction::Escape
    } else {
        KeyAction::Shortcut
    }
}

// --- U07 view metrics -------------------------------------------------------

/// A canvas measured in *physical* pixels, its DPI scale and its origin.
///
/// The canvas rectangle is the region the CAD content actually occupies, in
/// logical pixels from the top-left of the host surface; it is usually not the
/// whole window (toolbars/insets take space, audit U07). `dpi_scale` is the
/// physical pixels per logical pixel (1.0 desktop, 2–3 mobile).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasMetrics {
    /// Canvas origin in logical pixels from the surface top-left.
    pub origin_logical: [f64; 2],
    /// Canvas size in logical pixels (positive).
    pub size_logical: [f64; 2],
    /// Physical pixels per logical pixel (positive; 1.0 = standard DPI).
    pub dpi_scale: f64,
}

impl CanvasMetrics {
    pub fn new(origin_logical: [f64; 2], size_logical: [f64; 2], dpi_scale: f64) -> Self {
        CanvasMetrics {
            origin_logical,
            size_logical,
            dpi_scale,
        }
    }

    /// Whether the metrics describe a usable canvas.
    ///
    /// Degenerate metrics (zero/negative/non-finite size or a non-positive or
    /// non-finite DPI scale) are rejected explicitly rather than producing NaN
    /// world points (audit B17).
    pub fn is_valid(&self) -> bool {
        finite2(self.origin_logical)
            && finite2(self.size_logical)
            && self.size_logical[0] > 0.0
            && self.size_logical[1] > 0.0
            && self.dpi_scale.is_finite()
            && self.dpi_scale > 0.0
    }

    /// The canvas size in physical pixels.
    pub fn size_physical(&self) -> Option<[f64; 2]> {
        if !self.is_valid() {
            return None;
        }
        Some([
            self.size_logical[0] * self.dpi_scale,
            self.size_logical[1] * self.dpi_scale,
        ])
    }
}

/// The mapping canvas rectangle → logical → physical → world (audit U07).
///
/// `surface_logical` is a point in *surface* logical pixels (what a host
/// pointer gives). The mapping subtracts the canvas origin, so drawing and
/// picking agree no matter where the canvas sits (audit U07 "frame 用全窗口尺寸
/// 而 Image 只占工具栏之间区域").
#[derive(Debug)]
pub struct ViewMetrics<'a> {
    pub canvas: CanvasMetrics,
    /// The authoritative viewport: owns the camera, work plane and 2D mapping.
    pub viewport: &'a Viewport,
}

impl<'a> ViewMetrics<'a> {
    pub fn new(canvas: CanvasMetrics, viewport: &'a Viewport) -> Self {
        ViewMetrics { canvas, viewport }
    }

    /// Surface logical point → canvas-local logical point.
    ///
    /// Returns `None` for an invalid canvas or non-finite input.
    pub fn surface_to_canvas_logical(&self, surface: [f64; 2]) -> Option<[f64; 2]> {
        if !self.canvas.is_valid() || !finite2(surface) {
            return None;
        }
        Some([
            surface[0] - self.canvas.origin_logical[0],
            surface[1] - self.canvas.origin_logical[1],
        ])
    }

    /// Canvas-local logical point → physical pixels (top-left of the canvas).
    pub fn canvas_to_physical(&self, canvas_logical: [f64; 2]) -> Option<[f64; 2]> {
        if !self.canvas.is_valid() || !finite2(canvas_logical) {
            return None;
        }
        Some([
            canvas_logical[0] * self.canvas.dpi_scale,
            canvas_logical[1] * self.canvas.dpi_scale,
        ])
    }

    /// Surface logical point → canvas-local logical point in one step, used by
    /// picking and by preview rendering so they cannot diverge.
    pub fn surface_to_world(&self, surface: [f64; 2]) -> Option<Point3> {
        let canvas_logical = self.surface_to_canvas_logical(surface)?;
        // The viewport owns the 2D mapping and the work plane; reuse it rather
        // than re-deriving a second formula (audit U07 "绘制与拾取一致").
        self.viewport
            .screen_to_world(canvas_logical, self.canvas.size_logical)
    }

    /// The `world_per_px` a measurement snap should use for this canvas.
    ///
    /// Derived from the same camera the mapping uses, so a tolerance in logical
    /// pixels converts to the same world scale that a pick would.
    pub fn world_per_px(&self) -> f64 {
        self.viewport.world_per_px()
    }

    /// The work plane the pick maps onto (surface `z` when the view is plan).
    pub fn work_plane(&self) -> WorkPlane {
        self.viewport.work_plane
    }

    /// A world-space pan delta equivalent to a physical-pixel drag.
    ///
    /// The renderer works in logical pixels, so a pointer delta is already in
    /// logical units; converting through `dpi_scale` would double-count it. This
    /// helper exists to keep that decision in one place: physical pixels are
    /// converted *down* to logical before applying `world_per_px`.
    pub fn pan_delta_world(&self, physical_delta: [f64; 2]) -> Option<[f64; 2]> {
        if !self.canvas.is_valid() || !finite2(physical_delta) {
            return None;
        }
        let scale = self.world_per_px();
        let logical_dx = physical_delta[0] / self.canvas.dpi_scale;
        let logical_dy = physical_delta[1] / self.canvas.dpi_scale;
        Some([logical_dx * scale, logical_dy * scale])
    }
}

/// Recompute a viewport's logical size from new canvas metrics without touching
/// the camera, so a resize/rotation keeps the *centre* of the view fixed.
pub fn apply_canvas_metrics(viewport: &mut Viewport, canvas: &CanvasMetrics) -> CadResult<()> {
    if !canvas.is_valid() {
        return Err(cad_domain::CadError::InvalidInput(
            "canvas metrics are degenerate (size/DPI must be finite and positive)".into(),
        ));
    }
    viewport.logical_size = canvas.size_logical;
    viewport.dpi_scale = canvas.dpi_scale;
    Ok(())
}

/// Fold safe-area insets into a surface rectangle, producing the canvas metrics.
///
/// `safe_insets` is ordered `[top, right, bottom, left]` in logical pixels
/// (matching CSS `env(safe-area-inset-*)`). The canvas origin shifts in by the
/// left/top inset
/// and the canvas size shrinks by the sum of the opposing insets, so the host
/// supplies the canvas rect **minus** the safe area (audit U07,
/// `docs/input.md` §3).
///
/// The insets are logical-pixel geometry: the DPI scale is carried through
/// unchanged and `CanvasMetrics::size_physical` owns the logical→physical
/// conversion, so DPR=1/2/3 map the same logical point to the same world point.
///
/// Degenerate input is rejected explicitly with `None` — a non-finite or
/// negative inset, a non-finite/non-positive surface size or origin, or insets
/// that leave no canvas. Insets are never clamped or invented, so a host that
/// cannot measure them must pass explicit zeros (see the Linux host) rather than
/// guess.
pub fn inset_canvas_metrics(
    surface_origin_logical: [f64; 2],
    surface_size_logical: [f64; 2],
    safe_insets: [f64; 4],
    dpi_scale: f64,
) -> Option<CanvasMetrics> {
    if !finite2(surface_origin_logical) || !finite2(surface_size_logical) {
        return None;
    }
    if surface_size_logical[0] <= 0.0 || surface_size_logical[1] <= 0.0 {
        return None;
    }
    // A negative or non-finite inset is not a smaller margin to clamp away: it
    // means the host could not measure the safe area, so it is refused.
    if safe_insets
        .iter()
        .any(|inset| !inset.is_finite() || *inset < 0.0)
    {
        return None;
    }
    let [top, right, bottom, left] = safe_insets;
    let canvas = CanvasMetrics::new(
        [
            surface_origin_logical[0] + left,
            surface_origin_logical[1] + top,
        ],
        [
            surface_size_logical[0] - left - right,
            surface_size_logical[1] - top - bottom,
        ],
        dpi_scale,
    );
    // `is_valid` re-checks the reduced size and the DPI, so an over-large inset
    // (no canvas left) or a non-positive DPI is an explicit `None`.
    canvas.is_valid().then_some(canvas)
}

/// Map a layout-panel `SpaceSelection` to the validated `SwitchSpace` payload.
///
/// The UI host installs a layout-switch sink that dispatches
/// the same command the layout rows use (`CommandId::SwitchSpace`); naming the
/// payload here keeps the host and the UI from drifting on the mapping. It does
/// **not** validate
/// the layout: the command layer refuses an unknown `LayoutId` against the real
/// layout table, so an unknown space is an explicit failure, never a silent
/// success or a database mutation (`docs/layouts.md` §4).
pub fn space_switch_payload(space: cad_representation::SpaceSelection) -> crate::CommandPayload {
    crate::CommandPayload::Space(match space {
        cad_representation::SpaceSelection::Model => cad_domain::SpaceId::Model,
        cad_representation::SpaceSelection::Paper(id) => cad_domain::SpaceId::Paper(id),
    })
}

// --- U08 status -------------------------------------------------------------

/// A brief status-bar model. Deliberately small: the full diagnostic detail
/// lives in [`DiagnosticsModel`], not in a single status line (audit U08 "单行
/// 状态栏只显示首条诊断").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusModel {
    pub loading: LoadingState,
    /// The active tool, for the "current tool" hint (audit U04).
    pub tool: ToolStatus,
    /// Unit label of the open document, or `None` when unknown.
    pub units: Option<String>,
}

/// Coarse load state shown in the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadingState {
    Idle,
    Opening { label: String },
    Error { message: String },
}

/// A short, non-localized tool hint for the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    None,
    Select,
    Pan,
    Measure,
    /// A multi-touch/pointer gesture is navigating the view.
    Navigating,
}

impl StatusModel {
    /// Derive a status model from the live session.
    pub fn from_session(
        session: &SessionState,
        units: Option<String>,
        loading: LoadingState,
    ) -> Self {
        let tool = match &session.tool {
            ToolState::Idle => ToolStatus::None,
            ToolState::Selecting => ToolStatus::Select,
            // The legacy status projection has no draw-specific catalog value;
            // preview/status panels consume the typed draw snapshot directly.
            ToolState::Drawing(_) => ToolStatus::None,
            ToolState::Measuring(_) => ToolStatus::Measure,
            ToolState::Panning => ToolStatus::Pan,
        };
        StatusModel {
            loading,
            tool,
            units,
        }
    }

    /// A one-line summary for the status bar. Never contains the first
    /// diagnostic message: detail belongs in the diagnostics drawer.
    pub fn summary_line(&self) -> String {
        let loading = match &self.loading {
            LoadingState::Idle => "就绪",
            LoadingState::Opening { .. } => "加载中",
            LoadingState::Error { .. } => "打开失败",
        };
        let tool = match self.tool {
            ToolStatus::None => "无工具",
            ToolStatus::Select => "选择",
            ToolStatus::Pan => "平移",
            ToolStatus::Measure => "测量",
            ToolStatus::Navigating => "浏览",
        };
        let units = self.units.as_deref().unwrap_or("单位未知");
        format!("{loading} · {tool} · {units}")
    }
}

/// Object-level diagnostics summary for the drawer (audit U08).
///
/// Wraps [`DiagnosticsModel`] so every object and every reason survives; a UI
/// must render the drawer from this, not from `summary_line`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticsDrawer {
    pub model: DiagnosticsModel,
    /// The backend actually in use, when known (audit U08 "实际后端").
    pub backend: Option<String>,
    /// Recovery actions the user can take, as stable codes (localized by UI).
    pub recovery_actions: Vec<String>,
}

impl DiagnosticsDrawer {
    pub fn new() -> Self {
        DiagnosticsDrawer::default()
    }

    /// The document-level summary; keeps counts for every completeness bucket.
    pub fn summary(&self) -> DiagnosticsSummary {
        self.model.summary()
    }

    /// Every object with at least one reason, in insertion order. The UI must
    /// show all of them, not just the first.
    pub fn objects_with_reasons(&self) -> Vec<&cad_diagnostics::ObjectDiagnostics> {
        self.model
            .objects
            .iter()
            .filter(|entry| !entry.reasons.is_empty())
            .collect()
    }

    /// The total number of reasons across objects and document level.
    pub fn reason_count(&self) -> usize {
        self.summary().reasons
    }

    /// Whether the drawer has anything other than "complete".
    pub fn has_findings(&self) -> bool {
        !self.summary().complete
    }
}

#[cfg(test)]
mod tests;
