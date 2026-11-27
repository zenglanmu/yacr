//! The `PAINT <material>` typed argument (SketchUp Paint Bucket).
//!
//! The session does not resolve the name: the command owns the
//! document-first/built-in-palette lookup and every refusal, so a typed line and
//! a direct payload caller see the same behaviour. The session only splits the
//! name off, dispatches the payload and records the real refusal.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId};

use super::commands::{invalid_paint_argument_reason, parse_paint_argument, PAINT_SELECTOR};

impl super::CadSession {
    /// Apply one typed `PAINT <material>` line.
    pub(super) fn paint_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(name) = parse_paint_argument(argument) else {
            return self.refuse_argument(invalid_paint_argument_reason(text));
        };
        self.dispatch_typed_argument(
            PAINT_SELECTOR,
            CommandId::Extended(ExtendedCommandId::Paint),
            CommandPayload::PaintMaterial(name),
        )
    }
}
