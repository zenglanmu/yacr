//! Payloads for the 2026-11 text/draw utilities: `REVCLOUD`, `WIPEOUT`,
//! `TEXTMASK` and `TXT2MTXT`.
//!
//! They travel through [`crate::AnnotationBatchPayload::Utility`] so
//! `crate::CommandPayload` stays inside its file-size budget without a new
//! top-level variant (the same grouping the dimension/annotation batches use).
//! Every field here is plain data; validation, refusals and the single
//! transaction belong to the command handlers.

use cad_domain::{Point3, SelectionRef};

use crate::WipeoutFrames;

/// The document-level option set of `TEXTMASK`.
///
/// `offset_factor` is AutoCAD's offset prompt: the estimated text box grows by
/// `offset_factor × stored text height` on every side. The documented default is
/// **0.35**. `kind` is the mask type; only [`TextMaskKind::Wipeout`] has a
/// modelled entity (`SemanticGeometry::Mask`), the other two refuse by name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMaskOptions {
    pub offset_factor: f64,
    pub kind: TextMaskKind,
}

impl Default for TextMaskOptions {
    fn default() -> Self {
        Self {
            offset_factor: 0.35,
            kind: TextMaskKind::Wipeout,
        }
    }
}

/// The three AutoCAD `TEXTMASK` mask types.
///
/// This build stores every mask as a wipeout (`SemanticGeometry::Mask`); there is
/// no 3D-face or solid entity, so those two are named `Unsupported` refusals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMaskKind {
    /// A `WIPEOUT` (`AcDbWipeout`) mask: the only modelled kind.
    Wipeout,
    /// A 3D face: no such entity exists in this build.
    Face3d,
    /// A solid: no such entity exists in this build.
    Solid,
}

/// The document-level option set of `TXT2MTXT`.
///
/// The defaults reproduce the documented initial settings: combine into one
/// MTEXT, order top-down, no word-wrap and no forced uniform line spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Txt2MTextOptions {
    pub combine: bool,
    pub order: TextOrder,
    pub word_wrap: bool,
    pub force_uniform_line_spacing: bool,
}

impl Default for Txt2MTextOptions {
    fn default() -> Self {
        Self {
            combine: true,
            order: TextOrder::TopDown,
            word_wrap: true,
            force_uniform_line_spacing: false,
        }
    }
}

/// The source order `TXT2MTXT` uses when it emits MTEXT rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOrder {
    /// Sort top-to-bottom (descending Y), then left-to-right (ascending X).
    TopDown,
    /// Keep the selection order exactly.
    Selection,
}

/// The free-standing `REVCLOUD` construction modes.
///
/// `Object` converts the one selected closed/open path; `Rectangular`,
/// `Polygonal` and `Freehand` build a path from points; `Modify` edits an
/// existing cloud; `Style` selects the cloud style.
#[derive(Debug, Clone, PartialEq)]
pub enum RevisionCloudMode {
    /// Convert the one selected `CIRCLE`, `ELLIPSE`, `LWPOLYLINE` or `SPLINE`.
    Object,
    /// Two opposite rectangle corners, then reverse the arc side when set.
    Rectangular {
        first: Point3,
        second: Point3,
        reverse: bool,
    },
    /// A vertex run. `closed` closes the ring; `reverse` flips the arc side.
    Polygonal {
        points: Vec<Point3>,
        closed: bool,
        reverse: bool,
    },
    /// A point run. `closed` closes the ring; `reverse` flips the arc side.
    Freehand {
        points: Vec<Point3>,
        closed: bool,
        reverse: bool,
    },
    /// Add points to a cloud and erase the arc nearest `erase_side` (opening it).
    Modify {
        target: SelectionRef,
        add_points: Vec<Point3>,
        erase_side: Point3,
    },
    /// `Style Normal|Calligraphy`.
    Style(RevCloudStyle),
}

/// The two AutoCAD `REVCLOUD` styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevCloudStyle {
    /// The ordinary arc cloud.
    Normal,
    /// Calligraphy: a visual style this build cannot store.
    Calligraphy,
}

/// The full `REVCLOUD` input.
///
/// `arc_length` is the approximate **chord** length of every generated arc.
/// `None` applies the declared first-cloud default (5 % of the source path's
/// bounding-box diagonal, `DEFAULT_ARC_LENGTH_FRACTION` in
/// `app_drawing::revision_cloud`).
#[derive(Debug, Clone, PartialEq)]
pub struct RevisionCloudOptions {
    pub mode: RevisionCloudMode,
    pub arc_length: Option<f64>,
}

/// The input of `WIPEOUT`.
#[derive(Debug, Clone, PartialEq)]
pub enum WipeoutPayload {
    /// The existing open-ended point run.
    Points(Vec<Point3>),
    /// Convert one closed, straight-segment, zero-width `LWPOLYLINE`; `erase`
    /// deletes the source in the same transaction.
    Polyline { target: SelectionRef, erase: bool },
    /// Set the session `WIPEOUTFRAME` state (all wipeouts in the drawing).
    Frames(WipeoutFrames),
}

/// One payload of the 2026-11 text/draw utility batch.
#[derive(Debug, Clone, PartialEq)]
pub enum UtilityPayload {
    RevisionCloud(RevisionCloudOptions),
    Wipeout(WipeoutPayload),
    TextMask(TextMaskOptions),
    Txt2MText(Txt2MTextOptions),
}
