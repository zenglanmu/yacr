//! The typed `CLIP` payload data.
//!
//! `CLIP` is the generic boundary command over the **existing** clip models:
//! [`cad_domain::ImageClip`] (written by `IMAGECLIP`) and
//! [`cad_domain::BlockClip`] (written by `XCLIP`). It carries a boundary or one
//! of the boundary-state words; the target is the current selection, so it is
//! not part of the payload.

use cad_domain::Point3;

/// The input of a `CLIP` command: a new boundary, or one state word.
///
/// The command reads the current selection and requires exactly one directly
/// addressable **image** or **model-space block reference**. Which clip space a
/// coordinate belongs to follows the target kind, because that is the space its
/// stored model uses (there is no second clip model):
///
/// * an **image** reads [`Self::Rect`] in its own **pixel** space and
///   [`Self::Polygon`] as **world-space** points, exactly like `IMAGECLIP`;
/// * a **block reference** reads [`Self::Rect`] in the block definition's
///   **local** space, exactly like `XCLIP`. It refuses [`Self::Polygon`] by name
///   (`clip.polygon_block_not_renderable`): `BlockClip` has no polygon render
///   path (the representation layer keeps only the rectangular form), so storing
///   one would accept a boundary that is never drawn.
///
/// [`Self::On`] / [`Self::Off`] switch a stored block clip without changing its
/// geometry; an image clip has no enabled/disabled flag in this build, so those
/// two words are refused by name for an image target instead of being silently
/// reinterpreted (see the `clip.on_unsupported` / `clip.off_unsupported`
/// reasons). [`Self::Delete`] removes the stored clip, which every clip model can
/// express.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipCommand {
    /// `CLIP RECT <minx> <miny> <maxx> <maxy>`: the target's own clip space.
    Rect([f64; 4]),
    /// `CLIP POLY <x1> <y1> …`: world-space points (at least three), **raster
    /// image targets only** (a block reference has no renderable polygon clip).
    /// The mapped pixel-space ring must be convex and simple, because the renderer
    /// fan-triangulates it (`clip.polygon_non_convex` otherwise).
    Polygon(Vec<Point3>),
    /// `CLIP ON`: apply the stored boundary (block references only).
    On,
    /// `CLIP OFF`: keep the boundary but stop clipping (block references only).
    Off,
    /// `CLIP DELETE`: remove the stored boundary.
    Delete,
}
