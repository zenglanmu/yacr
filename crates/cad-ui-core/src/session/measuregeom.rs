//! `MEASUREGEOM <option>`: the AutoCAD 2026 inquiry router's typed form
//! (2026-11-07).
//!
//! The router adds **no** measurement algorithm. `DISTANCE`/`ANGLE`/`AREA`/
//! `POLYLINE` submit the same `BeginMeasurement` request the base selectors
//! (`DIST`/`MEASUREANGLE`/`AREA`/`MEASUREPOLYLINE`) use, so the capture, preview,
//! commit and command-log entry are the existing engine's. `RADIUS` and `VOLUME`
//! reach the command layer as a [`cad_app::CommandPayload::MeasureGeom`] option:
//! the former is a read-only report, the latter an explicit not-implemented
//! refusal. Full contract: [`cad_app::command_id::measuregeom`].

use cad_app::{CommandId, CommandPayload, MeasureGeomOption, MeasurementToolKind};

use super::commands::MEASUREGEOM_SELECTOR;
use super::diagnostics::cad_error_text;
use super::CadSession;

/// Parse the argument of a `MEASUREGEOM` selector: one of the six full option
/// words, case-insensitive and whitespace-trimmed.
///
/// Only the **full** spellings are admitted — `DISTANCE`, `ANGLE`, `AREA`,
/// `POLYLINE`, `RADIUS`, `VOLUME`. No single-letter or partial abbreviation is
/// accepted: the router never guesses which option a typist meant. The caller
/// turns `None` into [`invalid_measuregeom_argument_reason`].
pub fn parse_measuregeom_argument(argument: &str) -> Option<MeasureGeomOption> {
    let key = argument.trim();
    if key.eq_ignore_ascii_case("DISTANCE") {
        return Some(MeasureGeomOption::Distance);
    }
    if key.eq_ignore_ascii_case("ANGLE") {
        return Some(MeasureGeomOption::Angle);
    }
    if key.eq_ignore_ascii_case("AREA") {
        return Some(MeasureGeomOption::Area);
    }
    if key.eq_ignore_ascii_case("POLYLINE") {
        return Some(MeasureGeomOption::Polyline);
    }
    if key.eq_ignore_ascii_case("RADIUS") {
        return Some(MeasureGeomOption::Radius);
    }
    if key.eq_ignore_ascii_case("VOLUME") {
        return Some(MeasureGeomOption::Volume);
    }
    None
}

/// The stable rejection reason for a `MEASUREGEOM` argument outside the grammar.
///
/// The typed line is echoed verbatim and the six accepted full words are named,
/// so an unknown word (`MEASUREGEOM RAD`) is refused by name instead of being
/// silently mapped onto `DISTANCE`. A bare `MEASUREGEOM` is the command layer's
/// separate `measuregeom.needs_option` refusal.
pub fn invalid_measuregeom_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (MEASUREGEOM needs DISTANCE, ANGLE, AREA, POLYLINE, RADIUS \
         or VOLUME)",
        text.trim()
    )
}

impl CadSession {
    /// Apply one typed `MEASUREGEOM <option>` line.
    pub(super) fn measuregeom_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_measuregeom_argument(argument) else {
            return self.refuse_argument(invalid_measuregeom_argument_reason(text));
        };
        // Echo the admitted selector (never the typed option text), exactly like
        // every other argument form.
        self.core.record_command(MEASUREGEOM_SELECTOR);
        match option {
            MeasureGeomOption::Distance => self.start_measurement(MeasurementToolKind::Distance),
            MeasureGeomOption::Angle => self.start_measurement(MeasurementToolKind::Angle),
            MeasureGeomOption::Area => self.start_measurement(MeasurementToolKind::Area),
            MeasureGeomOption::Polyline => {
                self.start_measurement(MeasurementToolKind::PolylineLength)
            }
            MeasureGeomOption::Radius | MeasureGeomOption::Volume => {
                self.dispatch_measuregeom(option)
            }
        }
    }

    /// Submit the existing measurement request for one routed option.
    fn start_measurement(&self, kind: MeasurementToolKind) -> Result<(), String> {
        self.submit_interactions(vec![
            cad_app::interaction::InteractionRequest::BeginMeasurement(kind),
        ])
    }

    /// Dispatch the report-only half (`RADIUS`) or the explicit `VOLUME`
    /// refusal through the ordinary command funnel.
    fn dispatch_measuregeom(&self, option: MeasureGeomOption) -> Result<(), String> {
        match self
            .core
            .command(CommandId::MeasureGeom, CommandPayload::MeasureGeom(option))
        {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_six_full_option_words_are_the_whole_grammar() {
        for (argument, option) in [
            ("DISTANCE", MeasureGeomOption::Distance),
            ("distance", MeasureGeomOption::Distance),
            (" ANGLE ", MeasureGeomOption::Angle),
            ("Area", MeasureGeomOption::Area),
            ("POLYLINE", MeasureGeomOption::Polyline),
            ("RADIUS", MeasureGeomOption::Radius),
            ("VOLUME", MeasureGeomOption::Volume),
        ] {
            assert_eq!(
                parse_measuregeom_argument(argument),
                Some(option),
                "{argument}"
            );
        }
        for refused in [
            "",
            "D",
            "DIST",
            "A",
            "R",
            "V",
            "POLY",
            "RADIUS2",
            "RADIUS VOLUME",
            "DISTANCE 2",
        ] {
            assert_eq!(parse_measuregeom_argument(refused), None, "{refused:?}");
        }
        let reason = invalid_measuregeom_argument_reason("MEASUREGEOM RAD");
        assert!(reason.contains("MEASUREGEOM RAD"), "{reason}");
        assert!(
            reason.contains("DISTANCE, ANGLE, AREA, POLYLINE, RADIUS or VOLUME"),
            "{reason}"
        );
    }
}
