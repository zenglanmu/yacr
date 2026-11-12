//! The typed `XCLIP` payload data.
//!
//! Split out of `payload_extra.rs` to keep every code file inside the
//! 1000-line gate. The command enum owns the variant; this module owns the data,
//! following the existing Boxed payload pattern.

/// The input of an `XCLIP` command: a block-local clip boundary or one of the
/// boundary/switch/report forms.
///
/// Every coordinate in this payload is in the block definition's **local**
/// coordinate space — the space the block's children are authored in, not world
/// space. Storing the boundary there is what makes the clip follow the reference
/// through move/rotate/scale: the representation layer maps it through the
/// composed INSERT transform at expansion time.
///
/// The selection is not part of the payload: the command reads the session's
/// current selection and requires exactly one directly addressable model-space
/// `INSERT`.
///
/// * [`Self::Rect`] — `[minx, miny, maxx, maxy]` with `min < max`; stored as the
///   two opposite corners of a rectangle. The command refuses a rectangle whose
///   transformed world quad would not be axis-aligned
///   (`xclip.rotation_not_renderable`), because the renderer only cuts
///   axis-aligned world rectangles exactly.
/// * [`Self::Polygon`] — three or more block-local points (`XCLIP POLY …`). The
///   N-gon is validated and stored verbatim, but this build has no exact polygon
///   clip path, so the command reports `xclip.polygon_not_renderable` and never
///   claims the boundary is drawn (the same declared boundary `VPCLIP` has).
/// * [`Self::On`] / [`Self::Off`] — switch the stored boundary on or off.
/// * [`Self::Delete`] — remove the boundary entirely.
/// * [`Self::Invert`] — refused (`xclip.invert_not_renderable`), never stored
///   invisibly: an inverted region is not something this build can draw.
/// * [`Self::Report`] — the bare `XCLIP ?` read-only report.
#[derive(Debug, Clone, PartialEq)]
pub enum XClipCommand {
    /// `XCLIP RECT <minx> <miny> <maxx> <maxy>` in block-local coordinates.
    Rect([f64; 4]),
    /// `XCLIP POLY <x1> <y1> …` in block-local coordinates (at least 3 points).
    Polygon(Vec<[f64; 2]>),
    /// `XCLIP ON`: apply the stored boundary.
    On,
    /// `XCLIP OFF`: keep the boundary but stop clipping.
    Off,
    /// `XCLIP DELETE`: remove the stored boundary.
    Delete,
    /// `XCLIP INVERT`: refused by the command (not renderable here).
    Invert,
    /// `XCLIP ?`: read-only report of the stored boundary.
    Report,
}
