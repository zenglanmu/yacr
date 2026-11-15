//! The typed `CLIP` argument grammar.
//!
//! `CLIP` is the generic boundary form of the two clip models this build already
//! writes: the pixel-space [`cad_domain::ImageClip`] (`IMAGECLIP`) and the
//! block-local [`cad_domain::BlockClip`] (`XCLIP`). The parser is pure and strict:
//! it admits only the documented tokens and never clamps, defaults or repairs.
//!
//! * `CLIP RECT <minx> <miny> <maxx> <maxy>` — the target's own clip space
//!   (**pixel** space for an image, **block-local** for a block reference), the
//!   same numbers `IMAGECLIP` / `XCLIP RECT` read. Ordering (`minx < maxx`,
//!   `miny < maxy`) is the command's refusal, not a syntax rule here.
//! * `CLIP POLY <x1> <y1> <x2> <y2> <x3> <y3> …` — at least three **world-space**
//!   point pairs (what `IMAGECLIP POLY` reads). It is the **raster-image** form
//!   only: a block reference has no renderable polygon clip, so the command
//!   refuses the word by name (`clip.polygon_block_not_renderable`) and points at
//!   `CLIP RECT`; the mapped pixel-space ring must also be convex and simple
//!   (`clip.polygon_non_convex`), because the renderer fan-triangulates it.
//! * `CLIP ON|OFF|DELETE` — the state words. `ON`/`OFF` exist only for a block
//!   reference (an image clip has no enabled flag in this build) and are refused
//!   by name for an image; `DELETE` removes either stored clip.
//!
//! `CLIPIT` takes **no** argument: its whole input is the current selection (one
//! target plus the boundary entities), so it must not appear in
//! `ARGUMENT_SELECTORS` and a bare `CLIPIT` line reaches the command directly.
//!
//! The command layer re-validates the payload and owns every semantic refusal
//! (`clip.*`, plus the `imageclip.*` / `xclip.*` codes of the model it writes).
//! Full contract: `crate::app_drawing::clip_edit` (re-exported from `cad-app`).

use cad_app::ClipCommand;
use cad_domain::Point3;

use super::arguments::parse_signed_decimal;

pub const CLIP_SELECTOR: &str = "CLIP";

/// `CLIP RECT <minx> <miny> <maxx> <maxy>` / `CLIP POLY <x1> <y1> …` /
/// `CLIP ON|OFF|DELETE`.
pub fn parse_clip_argument(argument: &str) -> Option<ClipCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    let (first, rest) = tokens.split_first()?;
    if rest.is_empty() {
        if first.eq_ignore_ascii_case("ON") {
            return Some(ClipCommand::On);
        }
        if first.eq_ignore_ascii_case("OFF") {
            return Some(ClipCommand::Off);
        }
        if first.eq_ignore_ascii_case("DELETE") {
            return Some(ClipCommand::Delete);
        }
        return None;
    }
    if first.eq_ignore_ascii_case("RECT") {
        if rest.len() != 4 {
            return None;
        }
        return Some(ClipCommand::Rect([
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
            points.push(Point3 {
                x: parse_signed_decimal(pair[0])?,
                y: parse_signed_decimal(pair[1])?,
                z: 0.0,
            });
        }
        return Some(ClipCommand::Polygon(points));
    }
    None
}
