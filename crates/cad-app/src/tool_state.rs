//! Session-owned capture state for mutually exclusive interactive tools.

use crate::{DrawPreview, DrawTool, MeasurementPreview, MeasurementTool};
use cad_domain::SelectionRef;

/// Drawing capture and the immutable selection identity from tool start.
#[derive(Debug, Clone)]
pub struct DrawCapture {
    pub tool: DrawTool,
    pub selection: Vec<SelectionRef>,
}

#[derive(Debug, Clone)]
pub enum ToolState {
    Idle,
    Selecting,
    Drawing(DrawCapture),
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
