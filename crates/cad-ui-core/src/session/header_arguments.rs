//! The typed `BASE` / `LIMITS` command lines.
//!
//! Both commands write **drawing-header vectors** (`$INSBASE`, `$LIMMIN`,
//! `$LIMMAX`) through `cad-db`'s `drawing::header_geometry`, so the typed line is
//! an argument form of the existing extended selectors — not a new command and
//! not a second write path. The bare selectors keep their read-only report.
//!
//! * `BASE <x> <y> [<z>]` — one point; `z` is `0.0` when omitted (the only
//!   default, and the one DXF itself uses for a 2D point).
//! * `LIMITS <min_x> <min_y> <max_x> <max_y>` — both corners, because
//!   `$LIMMIN`/`$LIMMAX` are one rectangle. A single corner is written by
//!   `SETVAR LIMMIN`/`SETVAR LIMMAX`, which reads the other stored corner from
//!   the drawing; this line needs both so the request is never ambiguous.
//!
//! Components may be separated by whitespace or commas (`BASE 1,2` /
//! `BASE 1 2`). Every component is a finite plain decimal: an exponent, a
//! leading `.` or a non-numeric word is refused rather than repaired, and nothing
//! is clamped (the database owns the finiteness check that turns a `NaN` into a
//! named refusal).
//!
//! `LIMITS ON|OFF|SET` (`LIMCHECK` enforcement, the AutoCAD prompt words) is
//! refused by name here: this build models no limit checking, and the command
//! layer says the same thing in its diagnostics.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId};
use cad_domain::Point3;

use super::commands::{parse_base_argument, parse_limits_argument, BASE_SELECTOR, LIMITS_SELECTOR};
use super::CadSession;

impl CadSession {
    /// Apply one typed `BASE <x> <y> [<z>]` line.
    pub(super) fn base_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some([x, y, z]) = parse_base_argument(argument) else {
            return self.refuse_argument(format!(
                "invalid BASE argument: '{text}' (expected `<x> <y> [<z>]`, one finite decimal per \
                 component)"
            ));
        };
        self.dispatch_typed_argument(
            BASE_SELECTOR,
            CommandId::Extended(ExtendedCommandId::BasePoint),
            CommandPayload::Points(vec![Point3 { x, y, z }]),
        )
    }

    /// Apply one typed `LIMITS <min_x> <min_y> <max_x> <max_y>` line.
    pub(super) fn limits_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let trimmed = argument.trim();
        if matches!(trimmed.to_ascii_uppercase().as_str(), "ON" | "OFF" | "SET") {
            return self.refuse_argument(format!(
                "limits.check_unsupported: `LIMITS {}` would switch drawing-limit checking \
                 (LIMCHECK), which this build does not model, so no point would be clamped; \
                 nothing was changed. Write the stored rectangle with `LIMITS <min_x> <min_y> \
                 <max_x> <max_y>`",
                trimmed.to_ascii_uppercase()
            ));
        }
        let Some([min_x, min_y, max_x, max_y]) = parse_limits_argument(argument) else {
            return self.refuse_argument(format!(
                "invalid LIMITS argument: '{text}' (expected `<min_x> <min_y> <max_x> <max_y>`, one \
                 finite decimal per component)"
            ));
        };
        self.dispatch_typed_argument(
            LIMITS_SELECTOR,
            CommandId::Extended(ExtendedCommandId::DrawingLimits),
            CommandPayload::Points(vec![
                Point3 {
                    x: min_x,
                    y: min_y,
                    z: 0.0,
                },
                Point3 {
                    x: max_x,
                    y: max_y,
                    z: 0.0,
                },
            ]),
        )
    }
}
