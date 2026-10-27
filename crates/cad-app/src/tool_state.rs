//! Session-owned capture state for mutually exclusive interactive tools.

use crate::{CommandId, DrawPreview, DrawTool, MeasurementPreview, MeasurementTool};
use cad_domain::SelectionRef;

/// Drawing capture and the immutable selection identity from tool start.
#[derive(Debug, Clone)]
pub struct DrawCapture {
    pub tool: DrawTool,
    pub selection: Vec<SelectionRef>,
    /// The [`CommandId`] whose selector started this capture.
    ///
    /// Stored because a [`crate::DrawToolKind`] is not always one command:
    /// `COPYBASE` and `CUTBASE` share one capture shape (selection + base point)
    /// but are distinct commands with distinct confirmed payloads (capture only vs
    /// capture **and** delete). The typed
    /// [`crate::interaction::InteractionRequest::BeginDraw`] request carries only
    /// the kind, so the session records the command it authorized here and the host
    /// maps the confirmed intent to the payload **by this command** instead of
    /// guessing. For every other kind this is simply the kind's own command.
    pub command: CommandId,
}

#[derive(Debug, Clone)]
pub enum ToolState {
    Idle,
    Selecting,
    /// Boxed so the enum stays small: the draw capture owns the point/pick
    /// buffers and the tool's text fields, while every other variant is `Idle`/
    /// `Selecting`/`Panning` or a compact measurement tool. The box is an
    /// internal layout detail; pattern matches keep reading `capture.tool`.
    Drawing(Box<DrawCapture>),
    Measuring(MeasurementTool),
    Panning,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolSnapshot {
    Idle,
    Selecting,
    Drawing(DrawPreview),
    Measuring(MeasurementPreview),
    Panning,
}
