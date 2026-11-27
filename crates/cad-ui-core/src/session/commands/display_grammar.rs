//! The `SHADOWS`/`FOG` typed grammars, split out of `arguments.rs` for the
//! code-size gate.
//!
//! * `SHADOWS` (bare) reports the record; `SHADOWS ON|OFF` toggles the sun;
//!   `SHADOWS SUN <azimuth> <elevation>` stores both angles (degrees).
//! * `FOG` (bare) reports the record; `FOG ON|OFF` toggles the fog;
//!   `FOG RANGE <start> <end>` stores the band (model units);
//!   `FOG COLOR <r> <g> <b>` stores the sRGB colour bytes.
//!
//! Nothing else is admitted: an out-of-grammar line is the invalid-argument
//! refusal, and the domain re-validates every value so a direct payload caller
//! gets the same `display.invalid_value` refusal.

use cad_app::DisplayCommand;

/// The SketchUp `SHADOWS` selector.
pub const SHADOWS_SELECTOR: &str = "SHADOWS";

/// The SketchUp `FOG` selector.
pub const FOG_SELECTOR: &str = "FOG";

/// Parse one `SHADOWS` option line.
pub fn parse_shadows_argument(argument: &str) -> Option<DisplayCommand> {
    let text = argument.trim();
    if text.is_empty() {
        return Some(DisplayCommand::ReportShadows);
    }
    let mut words = text.split_whitespace();
    let keyword = words.next()?.to_ascii_uppercase();
    let rest: Vec<&str> = words.collect();
    match (keyword.as_str(), rest.as_slice()) {
        ("ON", []) => Some(DisplayCommand::Shadows(true)),
        ("OFF", []) => Some(DisplayCommand::Shadows(false)),
        ("SUN", [azimuth, elevation]) => {
            let azimuth: f64 = azimuth.parse().ok()?;
            let elevation: f64 = elevation.parse().ok()?;
            Some(DisplayCommand::Sun { azimuth, elevation })
        }
        _ => None,
    }
}

/// Parse one `FOG` option line.
pub fn parse_fog_argument(argument: &str) -> Option<DisplayCommand> {
    let text = argument.trim();
    if text.is_empty() {
        return Some(DisplayCommand::ReportFog);
    }
    let mut words = text.split_whitespace();
    let keyword = words.next()?.to_ascii_uppercase();
    let rest: Vec<&str> = words.collect();
    match (keyword.as_str(), rest.as_slice()) {
        ("ON", []) => Some(DisplayCommand::Fog(true)),
        ("OFF", []) => Some(DisplayCommand::Fog(false)),
        ("RANGE", [start, end]) => {
            let start: f64 = start.parse().ok()?;
            let end: f64 = end.parse().ok()?;
            Some(DisplayCommand::FogRange { start, end })
        }
        ("COLOR", [r, g, b]) | ("COLOUR", [r, g, b]) => {
            let channel = |word: &str| word.parse::<u8>().ok();
            Some(DisplayCommand::FogColor([
                channel(r)?,
                channel(g)?,
                channel(b)?,
            ]))
        }
        _ => None,
    }
}

/// The stable rejection reason for a `SHADOWS` line outside the grammar.
pub fn invalid_shadows_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (SHADOWS takes ON, OFF or SUN <azimuth> <elevation>)",
        text.trim()
    )
}

/// The stable rejection reason for a `FOG` line outside the grammar.
pub fn invalid_fog_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (FOG takes ON, OFF, RANGE <start> <end> or COLOR <r> <g> \
         <b>)",
        text.trim()
    )
}
