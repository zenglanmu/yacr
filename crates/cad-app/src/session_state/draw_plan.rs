//! The parameter-selected interactive capture plan for [`SessionState`].
//!
//! Split out of `session_state.rs` to keep that module inside the code-size
//! limit. The methods stay inherent on [`SessionState`], so the call sites are
//! unchanged; [`SessionState::refresh_draw_capture_plan`] is the one entrance the
//! general parameter channel uses.

use super::*;
use crate::draw_tool::{StretchWindowMode, StretchWindowStep};

/// The capture shape one tool's **live** session parameters select.
///
/// `point_target`/`text_required` are the pre-existing parameter-selected pieces
/// (ALIGN's pair count, BREAK's `F` override, LENGTHEN's `DY` mode, DIVIDE's block
/// option); `window_mode` is STRETCH's crossing-window mode (rectangle, polygon or
/// lasso). Every field defaults to "the kind's own rule" so a tool with no
/// parameter is described exactly as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DrawCapturePlan {
    /// The live point target, or `None` for the kind's static arity.
    pub point_target: Option<usize>,
    /// Whether the capture needs a terminal text step (DIVIDE's block name).
    pub text_required: bool,
    /// How a STRETCH capture collects its crossing window.
    pub window_mode: StretchWindowMode,
}

impl DrawCapturePlan {
    /// The kind's own rule: its static arity, no text step, the rectangle window.
    fn plain() -> Self {
        Self {
            point_target: None,
            text_required: false,
            window_mode: StretchWindowMode::Rect,
        }
    }
}

impl SessionState {
    /// The point target, text-step flag and crossing-window mode the **live** tool
    /// parameters select for `kind`.
    pub(super) fn draw_capture_plan(&self, kind: DrawToolKind) -> DrawCapturePlan {
        match kind {
            DrawToolKind::Align => {
                let pairs = match self.tool_parameter(ToolParameter::AlignPairs) {
                    ToolValue::Number(pairs)
                        if pairs.fract() == 0.0 && (1.0..=3.0).contains(&pairs) =>
                    {
                        pairs as usize
                    }
                    // The channel validates on write, so an out-of-range value is
                    // unreachable; fall back to the documented two-pair default
                    // rather than inventing a count.
                    _ => 2,
                };
                DrawCapturePlan {
                    point_target: Some(2 * pairs),
                    ..DrawCapturePlan::plain()
                }
            }
            DrawToolKind::Lengthen => {
                let target = match self.tool_parameter(ToolParameter::LengthenMode) {
                    ToolValue::Number(mode) if mode == crate::draw_tool::LENGTHEN_MODE_DYNAMIC => 2,
                    // DE/PER/TOT pick once: the pick chooses the end that moves.
                    _ => 1,
                };
                DrawCapturePlan {
                    point_target: Some(target),
                    ..DrawCapturePlan::plain()
                }
            }
            DrawToolKind::Break => match self.tool_parameter(ToolParameter::BreakFirstPoint) {
                ToolValue::Flag(true) => DrawCapturePlan {
                    point_target: Some(3),
                    ..DrawCapturePlan::plain()
                },
                _ => DrawCapturePlan::plain(),
            },
            DrawToolKind::Divide => DrawCapturePlan {
                text_required: matches!(
                    self.tool_parameter(ToolParameter::DivideUseBlock),
                    ToolValue::Flag(true)
                ),
                ..DrawCapturePlan::plain()
            },
            // STRETCH: the rectangle keeps the kind's static four-point arity; a
            // polygon/lasso capture collects its boundary ring first and then the
            // **same two** displacement points (base, displacement), so the fixed
            // run is two and the ring is the capture's own open-ended pre-phase.
            DrawToolKind::Stretch => {
                let window_mode = match self.tool_parameter(ToolParameter::StretchWindow) {
                    ToolValue::Number(value) => {
                        StretchWindowMode::from_parameter(value).unwrap_or_default()
                    }
                    // The channel stores a number for this parameter; a corrupt
                    // store keeps the documented rectangle default instead of
                    // inventing a fourth capture shape.
                    _ => StretchWindowMode::Rect,
                };
                DrawCapturePlan {
                    point_target: window_mode.is_ring().then_some(2),
                    text_required: false,
                    window_mode,
                }
            }
            _ => DrawCapturePlan::plain(),
        }
    }

    /// Re-apply the live capture plan after a tool parameter changed.
    ///
    /// A plan that would invalidate already-captured points is **not** applied
    /// (the old sequence stays valid); the new value is still stored and applies
    /// to the next invocation. Nothing is truncated or silently re-mapped, and a
    /// crossing-window mode never changes under an in-progress ring or a captured
    /// point — the window the user already started stays what it is.
    pub(super) fn refresh_draw_capture_plan(&mut self) {
        let Some(kind) = self.draw_kind() else {
            return;
        };
        let plan = self.draw_capture_plan(kind);
        let ToolState::Drawing(capture) = &mut self.tool else {
            return;
        };
        let captured = capture.tool.points().len();
        if plan.point_target.is_none_or(|target| target >= captured) {
            capture.tool.set_point_target(plan.point_target);
        }
        capture.tool.set_text_required(plan.text_required);
        if capture.tool.window_ring().is_empty() && capture.tool.points().is_empty() {
            capture.tool.set_window_mode(plan.window_mode);
        }
    }

    /// The crossing-window phase the active STRETCH capture is collecting, or
    /// `None`.
    ///
    /// This is the **one** query the host routes a free-form window gesture on: a
    /// `LassoTrace` step means the next drag is the lasso (rather than a box
    /// selection or a pan), and a `PolygonVertex` step means a click on the first
    /// vertex closes the ring. It reads the capture's own state, so it can never
    /// disagree with what the tool will accept.
    pub fn draw_window_step(&self) -> Option<StretchWindowStep> {
        let ToolState::Drawing(capture) = &self.tool else {
            return None;
        };
        if !capture.tool.window_open() {
            return None;
        }
        match capture.tool.window_mode() {
            StretchWindowMode::Rect => None,
            StretchWindowMode::Polygon => Some(StretchWindowStep::PolygonVertex),
            StretchWindowMode::Lasso => Some(StretchWindowStep::LassoTrace),
        }
    }

    /// The first captured crossing-window ring vertex of the active capture, or
    /// `None` while the ring is empty (or no ring capture is running).
    ///
    /// The host projects it to canvas logical pixels to apply AutoCAD's "click the
    /// first vertex to close the ring" rule with the same pick-box aperture every
    /// other pick uses.
    pub fn draw_window_first_vertex(&self) -> Option<Point3> {
        match &self.tool {
            ToolState::Drawing(capture) if capture.tool.window_open() => {
                capture.tool.first_window_vertex()
            }
            _ => None,
        }
    }

    /// The number of crossing-window ring vertices captured so far (`0` when no
    /// ring capture is running or its ring is empty).
    pub fn draw_window_ring_len(&self) -> usize {
        match &self.tool {
            ToolState::Drawing(capture) => capture.tool.window_ring().len(),
            _ => 0,
        }
    }

    /// Close the active STRETCH crossing-window ring (`CONFIRM`, or a click on its
    /// first vertex).
    ///
    /// The tool owns every refusal (an unopened/absent ring, fewer than three
    /// vertices, a zero-area ring) and stores nothing on failure, so a refused
    /// close leaves the capture exactly as it was. No database write and no
    /// history entry: the window is capture state until the command commits.
    pub fn close_draw_window_ring(&mut self) -> CadResult<()> {
        match &mut self.tool {
            ToolState::Drawing(capture) => {
                capture.tool.close_window_ring()?;
                self.bump_tool_generation();
                Ok(())
            }
            _ => Err(CadError::InvalidInput(
                "stretch.window.not_open: no drawing capture is active".into(),
            )),
        }
    }

    /// Record a **complete** crossing-window ring captured as a freehand drag (the
    /// lasso) and close it.
    ///
    /// The ring is the drag's sampled path in world coordinates in capture order,
    /// validated by the same rule a clicked polygon ring is
    /// ([`crate::draw_tool::validate_window_ring`]); nothing is written and no
    /// transaction opens. A capture that is not waiting for a ring refuses by name
    /// and stores nothing, so a stray drag can never redefine a rectangle window or
    /// another tool's step.
    pub fn set_draw_window_ring(&mut self, ring: Vec<Point3>) -> CadResult<()> {
        match &mut self.tool {
            ToolState::Drawing(capture) => {
                capture.tool.set_window_ring(ring)?;
                self.bump_tool_generation();
                Ok(())
            }
            _ => Err(CadError::InvalidInput(
                "stretch.window.not_open: no drawing capture is active".into(),
            )),
        }
    }
}
