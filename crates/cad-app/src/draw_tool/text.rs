//! The TEXT terminal channel on [`DrawTool`] (ADR 0019 decision A.1, P7a).
//!
//! Split out of [`super`] to keep the state-machine module inside the code-size
//! limit. These are inherent methods of the same type, so the public path is
//! unchanged (`cad_app::DrawTool::set_text`, …).
//!
//! TEXT is the first tool whose last step is a **typed string**: the point
//! complement (one insertion point) is captured first, then
//! `InteractionRequest::SetToolText` submits the text and the host completes the
//! command immediately. There is no second `Confirm`, and a `Confirm` while the
//! prompt is open is refused by name.

use super::*;

impl DrawTool {
    /// The submitted text for a text tool, if any (ADR 0019 decision A).
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Whether the tool's last step is typed text that has **not** been supplied
    /// yet, with its point complement already complete.
    ///
    /// This is the single state the `awaiting_text` status bit projects. It is
    /// `false` for every point-only kind, before the points are complete, and
    /// after the text was submitted (which completes the command).
    pub fn awaiting_text(&self) -> bool {
        self.kind.requires_text()
            && self.points.len() >= self.kind.required_points()
            && self.text.is_none()
    }

    /// Submit the terminal text for a text tool (ADR 0019 decision A.1).
    ///
    /// Refusals are **named** and never open a transaction (the tool holds no
    /// database handle, so a refusal cannot write anything either):
    ///
    /// * `tool.text_not_awaiting` — the active tool is not a text tool, its
    ///   point complement is already complete and the text was submitted, or no
    ///   insertion point was captured yet;
    /// * `tool.text_points_incomplete` — a text tool whose insertion point is
    ///   still missing (checked before the "not awaiting" refusal so the message
    ///   names the real cause);
    /// * `tool.text_empty` — the text is empty or whitespace-only;
    /// * `tool.text_too_long` — more than [`MAX_TOOL_TEXT_CHARS`] characters.
    ///
    /// On success the text is stored; the caller (the host) then completes the
    /// command immediately — this request is **terminal**, there is no second
    /// `Confirm`.
    pub fn set_text(&mut self, text: &str) -> CadResult<()> {
        if !self.kind.requires_text() {
            return Err(CadError::InvalidInput(format!(
                "tool.text_not_awaiting: draw kind '{}' takes no text",
                self.kind.key()
            )));
        }
        if self.points.len() < self.kind.required_points() {
            return Err(CadError::InvalidInput(format!(
                "tool.text_points_incomplete: draw kind '{}' needs {} point(s) before text, has {}",
                self.kind.key(),
                self.kind.required_points(),
                self.points.len()
            )));
        }
        if self.text.is_some() {
            return Err(CadError::InvalidInput(format!(
                "tool.text_not_awaiting: draw kind '{}' already has its text",
                self.kind.key()
            )));
        }
        if text.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "tool.text_empty: tool text must not be empty or whitespace-only".into(),
            ));
        }
        let length = text.chars().count();
        if length > MAX_TOOL_TEXT_CHARS {
            return Err(CadError::InvalidInput(format!(
                "tool.text_too_long: tool text allows at most {MAX_TOOL_TEXT_CHARS} characters, got {length}"
            )));
        }
        self.text = Some(text.to_string());
        Ok(())
    }

    /// The first `Esc` while a text prompt is open (ADR 0019 decision A.1).
    ///
    /// This is a **focus** change, not a cancel: the tool stays active and still
    /// awaits text, so the text may still be submitted. It is refused explicitly
    /// when no text prompt is open, so a host never mistakes a stray dismissal
    /// for a state change. Nothing here touches the database or the history.
    pub fn dismiss_text_focus(&self) -> CadResult<()> {
        if self.awaiting_text() {
            Ok(())
        } else {
            Err(CadError::InvalidInput(format!(
                "tool.text_not_awaiting: draw kind '{}' has no open text prompt",
                self.kind.key()
            )))
        }
    }
}
