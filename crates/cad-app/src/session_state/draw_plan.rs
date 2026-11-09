//! The parameter-selected interactive capture plan for [`SessionState`].
//!
//! Split out of `session_state.rs` to keep that module inside the code-size
//! limit. The methods stay inherent on [`SessionState`], so the call sites are
//! unchanged; [`SessionState::refresh_draw_capture_plan`] is the one entrance the
//! general parameter channel uses.

use super::*;

impl SessionState {
    /// The point target and text-step flag the **live** tool parameters select for
    /// `kind`.
    ///
    /// `None` / `false` means the kind's own static arity and text rule apply.
    pub(super) fn draw_capture_plan(&self, kind: DrawToolKind) -> (Option<usize>, bool) {
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
                (Some(2 * pairs), false)
            }
            DrawToolKind::Lengthen => match self.tool_parameter(ToolParameter::LengthenMode) {
                ToolValue::Number(mode) if mode == crate::draw_tool::LENGTHEN_MODE_DYNAMIC => {
                    (Some(2), false)
                }
                // DE/PER/TOT pick once: the pick chooses the end that moves.
                _ => (Some(1), false),
            },
            DrawToolKind::Break => match self.tool_parameter(ToolParameter::BreakFirstPoint) {
                ToolValue::Flag(true) => (Some(3), false),
                _ => (None, false),
            },
            DrawToolKind::Divide => (
                None,
                matches!(
                    self.tool_parameter(ToolParameter::DivideUseBlock),
                    ToolValue::Flag(true)
                ),
            ),
            _ => (None, false),
        }
    }

    /// Re-apply the live capture plan after a tool parameter changed.
    ///
    /// A plan that would invalidate already-captured points is **not** applied
    /// (the old sequence stays valid); the new value is still stored and applies
    /// to the next invocation. Nothing is truncated or silently re-mapped.
    pub(super) fn refresh_draw_capture_plan(&mut self) {
        let Some(kind) = self.draw_kind() else {
            return;
        };
        let (point_target, text_required) = self.draw_capture_plan(kind);
        let ToolState::Drawing(capture) = &mut self.tool else {
            return;
        };
        let captured = capture.tool.points().len();
        if point_target.is_none_or(|target| target >= captured) {
            capture.tool.set_point_target(point_target);
        }
        capture.tool.set_text_required(text_required);
    }
}
