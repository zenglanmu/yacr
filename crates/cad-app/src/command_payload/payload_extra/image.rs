//! The typed `IMAGEATTACH` / `IMAGECLIP` payload data.
//!
//! Split out of `payload_extra.rs` to keep every code file inside the
//! 1000-line gate. The command enum still owns the variants; this module owns
//! the data, following the existing Boxed payload pattern.

use cad_domain::Point3;

/// The input of an `IMAGEATTACH` command: one raster image placement.
///
/// The typed form is `IMAGEATTACH <file> <x> <y> <width_px> <height_px>
/// <units_per_pixel>`; the file token is a **single** token (a path with spaces
/// cannot be typed here — a declared limit of the v1 grammar).
///
/// `file` is the raw reference as typed. The command normalises it through
/// [`cad_resources::image_resource_key`] before storing, so the entity carries a
/// safe **logical** key, never a platform path; an unsafe reference (absolute
/// path, `..` traversal, URL) is the named `imageattach.unsafe_reference`
/// refusal. `pixels` is `(width, height)` in pixels and `units_per_pixel` is the
/// world length of one pixel along both image axes (`u` and `v`), so the placed
/// extent is `pixels[0] * units_per_pixel` by `pixels[1] * units_per_pixel`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageAttachCommand {
    pub file: String,
    pub origin: Point3,
    pub pixels: [f64; 2],
    pub units_per_pixel: f64,
}

/// The input of an `IMAGECLIP` command: a clip boundary, or the `OFF` form that
/// clears it.
///
/// Two boundary spaces are supported, matching AutoCAD's two clip prompts:
///
/// * [`Self::Rect`] — `[minx, miny, maxx, maxy]` in the image's own **pixel
///   coordinate space** (the same space `ImageClip::vertices` uses). The command
///   stores it as the two opposite corners of a rectangle with `inside: true`;
///   this is the original typed form `IMAGECLIP <minx> <miny> <maxx> <maxy>`.
/// * [`Self::Polygon`] — three or more **world-space** points (the `IMAGECLIP
///   POLY x1 y1 x2 y2 x3 y3 …` prompt). The command verifies every point lies on
///   the image plane, maps it into pixel space with the image's own `u`/`v`
///   basis and stores the resulting N-gon with `inside: true`. A point that is
///   not coplanar with the image plane is the named
///   `imageclip.polygon_non_coplanar` refusal.
///
/// The selection is not part of the payload: the command reads the session's
/// current selection and requires exactly one directly addressable `Image`
/// entity.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageClipCommand {
    /// `IMAGECLIP OFF`: clear the stored clip.
    Off,
    /// Pixel-space rectangle: two opposite corners `[minx, miny, maxx, maxy]`.
    Rect([f64; 4]),
    /// World-space polygon boundary (at least three distinct points).
    Polygon(Vec<Point3>),
}

/// The input of an `IMAGEADJUST` command: the four display-transfer values for
/// the one selected raster image.
///
/// The typed forms are `IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]`
/// and `IMAGEADJUST RESET`. The three percentages are integers in `0..=100`
/// (`50`/`50`/`0` is neutral). `monochrome` is `None` when the typed line omits
/// the MONO/COLOR word, which means **keep the image's current monochrome
/// state** rather than defaulting it. `RESET` restores the neutral
/// [`cad_domain::ImageAdjust`] (50/50/0/off).
///
/// The selection is not part of the payload: the command reads the session's
/// current selection and requires exactly one directly addressable `Image`
/// entity. The command re-validates the ranges (the payload carries `u8`, whose
/// type alone allows values above 100) and is the authority for
/// `imageadjust.invalid_value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageAdjustCommand {
    /// `IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]`.
    Set {
        brightness: u8,
        contrast: u8,
        fade: u8,
        /// `Some(true)` = MONO, `Some(false)` = COLOR, `None` = keep the current
        /// monochrome state.
        monochrome: Option<bool>,
    },
    /// `IMAGEADJUST RESET`: restore 50/50/0/off.
    Reset,
}
