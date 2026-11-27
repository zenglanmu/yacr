//! The typed `EDGECOLOR` payload.
//!
//! `EDGECOLOR <r> <g> <b>` sets a session edge-overlay colour, `DEFAULT` resets
//! it, and a bare selector reports the current value. Nothing is written to the
//! drawing.

/// One `EDGECOLOR` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeColorCommand {
    /// Set the session body edge colour (sRGB bytes).
    Set([u8; 3]),
    /// Restore the body's own colour.
    Default,
    /// Bare selector: report the current value.
    Report,
}
