//! The `TEXT3D` typed grammar, split out of `arguments.rs` for the code-size
//! gate.
//!
//! `TEXT3D <x> <y> <z> <height> <extrusion> <font|-> <text…>`: five finite
//! decimals, one face token (`-` = the engine's default/fallback chain) and a
//! non-empty remainder. The remainder keeps its internal spacing (the text is a
//! string, not a token); the command layer owns the positive/finite ranges and
//! every text rule, so a direct payload caller gets the same refusals.

use cad_app::Text3dInput;
use cad_domain::Point3;

/// The extrude-text selector.
pub const TEXT3D_SELECTOR: &str = "TEXT3D";

/// Parse one `TEXT3D` line.
pub fn parse_text3d_argument(argument: &str) -> Option<Text3dInput> {
    let mut rest = argument.trim();
    let mut numbers = [0.0_f64; 5];
    for slot in &mut numbers {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        *slot = rest[..end].parse().ok()?;
        if !slot.is_finite() {
            return None;
        }
        rest = &rest[end..];
    }
    rest = rest.trim_start();
    let font_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    if font_end == 0 {
        return None;
    }
    let font = &rest[..font_end];
    let font = if font == "-" {
        None
    } else {
        Some(font.to_string())
    };
    let text = rest[font_end..].trim();
    if text.is_empty() {
        return None;
    }
    Some(Text3dInput {
        origin: Point3 {
            x: numbers[0],
            y: numbers[1],
            z: numbers[2],
        },
        height: numbers[3],
        extrusion: numbers[4],
        font,
        text: text.to_string(),
    })
}

/// The stable rejection reason for a `TEXT3D` line outside the grammar.
pub fn invalid_text3d_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (TEXT3D needs <x> <y> <z> <height> <extrusion> <font|-> \
         <text>)",
        text.trim()
    )
}
