//! Typed-argument grammars for the raster-image commands (`IMAGEATTACH`,
//! `IMAGECLIP`).
//!
//! Every parser is pure and strict: it admits only the documented tokens and
//! never clamps, defaults or repairs. The command layer re-validates the payload
//! (safe resource key, positive pixel size, positive units-per-pixel, ordered
//! clip rectangle) and is the authority for the named refusals.
//!
//! Full contracts: `crate::app_drawing::image_edit` (re-exported from `cad-app`).

use cad_app::{ImageAttachCommand, ImageClipCommand};
use cad_domain::Point3;

use super::arguments::{parse_positive_decimal, parse_signed_decimal};

pub const IMAGEATTACH_SELECTOR: &str = "IMAGEATTACH";
pub const IMAGECLIP_SELECTOR: &str = "IMAGECLIP";

/// `IMAGEATTACH <file> <x> <y> <width_px> <height_px> <units_per_pixel>`.
///
/// The file is a **single** token (the declared v1 limit: a path with spaces
/// cannot be typed here). `x`/`y` are signed plain decimals, and the pixel size
/// plus the units-per-pixel scale are positive plain decimals; the command
/// re-checks them and refuses an unsafe reference by name.
pub fn parse_imageattach_argument(argument: &str) -> Option<ImageAttachCommand> {
    let tokens = tokens(argument);
    if tokens.len() != 6 {
        return None;
    }
    Some(ImageAttachCommand {
        file: tokens[0].to_string(),
        origin: Point3 {
            x: parse_signed_decimal(tokens[1])?,
            y: parse_signed_decimal(tokens[2])?,
            z: 0.0,
        },
        pixels: [
            parse_positive_decimal(tokens[3])?,
            parse_positive_decimal(tokens[4])?,
        ],
        units_per_pixel: parse_positive_decimal(tokens[5])?,
    })
}

/// `IMAGECLIP <minx> <miny> <maxx> <maxy>` or `IMAGECLIP OFF`.
///
/// The four coordinates are pixel-space signed plain decimals. Ordering
/// (`minx < maxx`, `miny < maxy`) is the command's `imageclip.invalid_rect`
/// refusal, not a syntax rule here.
pub fn parse_imageclip_argument(argument: &str) -> Option<ImageClipCommand> {
    let tokens = tokens(argument);
    if tokens.len() == 1 && tokens[0].eq_ignore_ascii_case("OFF") {
        return Some(ImageClipCommand { rect: None });
    }
    if tokens.len() != 4 {
        return None;
    }
    Some(ImageClipCommand {
        rect: Some([
            parse_signed_decimal(tokens[0])?,
            parse_signed_decimal(tokens[1])?,
            parse_signed_decimal(tokens[2])?,
            parse_signed_decimal(tokens[3])?,
        ]),
    })
}

fn tokens(argument: &str) -> Vec<&str> {
    argument.split_whitespace().collect()
}
