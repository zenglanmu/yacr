//! The typed `XCLIP` argument grammar.
//!
//! The parser is pure and strict: it admits only the documented tokens and never
//! clamps, defaults or repairs. Every coordinate is **block-local** (the block
//! definition's own space), which is what makes the stored clip follow the
//! reference through move/rotate/scale. The command layer re-validates the
//! payload (rectangle ordering, polygon validity, axis-aligned world mapping,
//! singular/off-plane transforms) and owns every semantic refusal.
//!
//! Full contract: `crate::app_drawing::xclip_edit` (re-exported from `cad-app`).

use cad_app::XClipCommand;

use super::arguments::parse_signed_decimal;

pub const XCLIP_SELECTOR: &str = "XCLIP";

/// `XCLIP RECT <minx> <miny> <maxx> <maxy>` / `XCLIP POLY <x1> <y1> …` /
/// `XCLIP ON|OFF|DELETE|INVERT` / `XCLIP ?`.
///
/// The four rectangle coordinates are block-local signed plain decimals; their
/// ordering (`minx < maxx`, `miny < maxy`) is the command's
/// `xclip.degenerate_rect` refusal, not a syntax rule here.
///
/// `POLY` takes at least three block-local point pairs (`z = 0`): the command
/// validates the ring with the shared `validate_clip_polygon` rule
/// (`xclip.degenerate_polygon` / `.non_coplanar` / `.self_intersecting`).
///
/// `ON`/`OFF`/`DELETE`/`INVERT` are the one-token switch forms and `?` is the
/// read-only report; anything else is outside the grammar.
pub fn parse_xclip_argument(argument: &str) -> Option<XClipCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    let (first, rest) = tokens.split_first()?;
    if rest.is_empty() {
        if first.eq_ignore_ascii_case("ON") {
            return Some(XClipCommand::On);
        }
        if first.eq_ignore_ascii_case("OFF") {
            return Some(XClipCommand::Off);
        }
        if first.eq_ignore_ascii_case("DELETE") {
            return Some(XClipCommand::Delete);
        }
        if first.eq_ignore_ascii_case("INVERT") {
            return Some(XClipCommand::Invert);
        }
        if *first == "?" {
            return Some(XClipCommand::Report);
        }
        return None;
    }
    if first.eq_ignore_ascii_case("RECT") {
        if rest.len() != 4 {
            return None;
        }
        return Some(XClipCommand::Rect([
            parse_signed_decimal(rest[0])?,
            parse_signed_decimal(rest[1])?,
            parse_signed_decimal(rest[2])?,
            parse_signed_decimal(rest[3])?,
        ]));
    }
    if first.eq_ignore_ascii_case("POLY") {
        if rest.len() < 6 || !rest.len().is_multiple_of(2) {
            return None;
        }
        let mut points = Vec::with_capacity(rest.len() / 2);
        for pair in rest.as_chunks::<2>().0 {
            points.push([
                parse_signed_decimal(pair[0])?,
                parse_signed_decimal(pair[1])?,
            ]);
        }
        return Some(XClipCommand::Polygon(points));
    }
    None
}
