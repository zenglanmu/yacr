//! Typed-argument grammars for the raster-image commands (`IMAGEATTACH`,
//! `IMAGECLIP`, `IMAGEADJUST`).
//!
//! Every parser is pure and strict: it admits only the documented tokens and
//! never clamps, defaults or repairs. The command layer re-validates the payload
//! (safe resource key, positive pixel size, positive units-per-pixel, ordered
//! clip rectangle, 0..=100 transfer percentages) and is the authority for the
//! named refusals.
//!
//! Full contracts: `crate::app_drawing::image_edit` (re-exported from `cad-app`).

use cad_app::{ImageAdjustCommand, ImageAttachCommand, ImageClipCommand};
use cad_domain::Point3;

use super::arguments::{parse_positive_decimal, parse_signed_decimal};

pub const IMAGEATTACH_SELECTOR: &str = "IMAGEATTACH";
pub const IMAGECLIP_SELECTOR: &str = "IMAGECLIP";
pub const IMAGEADJUST_SELECTOR: &str = "IMAGEADJUST";
pub const DASH_IMAGEADJUST_SELECTOR: &str = "-IMAGEADJUST";

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
///
/// `IMAGECLIP POLY <x1> <y1> <x2> <y2> <x3> <y3> …` is the polygonal form: at
/// least three **world-space** point pairs on the flat work plane (`z = 0`, the
/// same plane `IMAGEATTACH` places its images on). The command is the authority
/// for the coplanarity, distinct-point and self-intersection refusals.
pub fn parse_imageclip_argument(argument: &str) -> Option<ImageClipCommand> {
    let tokens = tokens(argument);
    if tokens.len() == 1 && tokens[0].eq_ignore_ascii_case("OFF") {
        return Some(ImageClipCommand::Off);
    }
    if tokens.len() >= 2 && tokens[0].eq_ignore_ascii_case("POLY") {
        let coordinates = &tokens[1..];
        if coordinates.len() < 6 || !coordinates.len().is_multiple_of(2) {
            return None;
        }
        let mut points = Vec::with_capacity(coordinates.len() / 2);
        for pair in coordinates.as_chunks::<2>().0 {
            points.push(Point3 {
                x: parse_signed_decimal(pair[0])?,
                y: parse_signed_decimal(pair[1])?,
                z: 0.0,
            });
        }
        return Some(ImageClipCommand::Polygon(points));
    }
    if tokens.len() != 4 {
        return None;
    }
    Some(ImageClipCommand::Rect([
        parse_signed_decimal(tokens[0])?,
        parse_signed_decimal(tokens[1])?,
        parse_signed_decimal(tokens[2])?,
        parse_signed_decimal(tokens[3])?,
    ]))
}

fn tokens(argument: &str) -> Vec<&str> {
    argument.split_whitespace().collect()
}

/// `IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` or
/// `IMAGEADJUST RESET`.
///
/// The three values are plain non-negative integers in **`u8` shape**
/// (`0..=255`); the command is the authority for the real `0..=100` range and
/// refuses an overshoot with `imageadjust.invalid_value` instead of clamping.
/// The optional fourth token is the case-insensitive `MONO`/`COLOR` word and
/// `None` (the word omitted) means "keep the image's current monochrome state".
/// `RESET` is its own single-token form.
pub fn parse_imageadjust_argument(argument: &str) -> Option<ImageAdjustCommand> {
    let tokens = tokens(argument);
    if tokens.len() == 1 && tokens[0].eq_ignore_ascii_case("RESET") {
        return Some(ImageAdjustCommand::Reset);
    }
    if tokens.len() != 3 && tokens.len() != 4 {
        return None;
    }
    let monochrome = match tokens.get(3) {
        None => None,
        Some(word) if word.eq_ignore_ascii_case("MONO") => Some(true),
        Some(word) if word.eq_ignore_ascii_case("COLOR") => Some(false),
        Some(_) => return None,
    };
    Some(ImageAdjustCommand::Set {
        brightness: parse_percentage(tokens[0])?,
        contrast: parse_percentage(tokens[1])?,
        fade: parse_percentage(tokens[2])?,
        monochrome,
    })
}

/// One plain non-negative integer that fits `u8` (the percentage's *shape*;
/// `0..=100` is the command's rule, not this grammar's).
fn parse_percentage(token: &str) -> Option<u8> {
    if !token.bytes().all(|byte| byte.is_ascii_digit()) || token.is_empty() {
        return None;
    }
    token.parse().ok()
}
