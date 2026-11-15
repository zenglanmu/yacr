use cad_domain::{Completeness, Diagnostic, ObjectId};

use super::page::VectorPage;

// ---------------------------------------------------------------------------
// Document model
// ---------------------------------------------------------------------------

/// The paper colour a path document paints a WIPEOUT mask with.
///
/// A path writer has no renderer clear colour, and the document itself carries
/// no page background: SVG/PDF leave the page transparent, which reads as paper.
/// A mask therefore floods its boundary with this paper white, which is exactly
/// what "erasing to the page background" means once the sheet is printed. It is
/// deliberately a document constant — not the raster renderer's clear colour —
/// so the two outputs can be described honestly and changed independently.
pub const PAPER_BACKGROUND: [f32; 3] = [1.0, 1.0, 1.0];

/// One path in the document, in page-millimetre y-up coordinates.
///
/// `fill = Some` paints the interior; `stroke = Some` draws the outline with
/// `stroke_width_mm`. `opacity` is the resolved per-entity opacity in `[0, 1]`.
///
/// A closed filled path uses the **even-odd** fill rule, matching the raster
/// fill rule of HATCH and of the WIPEOUT mask (a self-intersecting ring is
/// stored as given and filled even-odd, never repaired).
#[derive(Debug, Clone, PartialEq)]
pub struct VectorPath {
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
    pub fill: Option<[f32; 3]>,
    pub stroke: Option<[f32; 3]>,
    pub stroke_width_mm: f64,
    pub opacity: f32,
}

impl VectorPath {
    /// An open stroked polyline.
    pub fn polyline(points: Vec<[f64; 2]>, stroke: [f32; 3], width_mm: f64, opacity: f32) -> Self {
        VectorPath {
            points,
            closed: false,
            fill: None,
            stroke: Some(stroke),
            stroke_width_mm: width_mm,
            opacity,
        }
    }

    /// A closed filled polygon.
    pub fn polygon(points: Vec<[f64; 2]>, fill: [f32; 3], opacity: f32) -> Self {
        VectorPath {
            points,
            closed: true,
            fill: Some(fill),
            stroke: None,
            stroke_width_mm: 0.0,
            opacity,
        }
    }
}

/// A self-contained vector page: its physical size, its path geometry and the
/// completeness verdict for geometry that could not be expressed as paths.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorDocument {
    pub width_mm: f64,
    pub height_mm: f64,
    /// The colour a WIPEOUT mask floods its interior with
    /// ([`PAPER_BACKGROUND`] unless a caller supplies another page colour).
    pub background: [f32; 3],
    pub paths: Vec<VectorPath>,
    pub completeness: Completeness,
    pub diagnostics: Vec<Diagnostic>,
}

impl VectorDocument {
    /// An empty document for a page (valid SVG/PDF, no paths).
    pub fn empty(page: &VectorPage) -> Self {
        VectorDocument {
            width_mm: page.width_mm,
            height_mm: page.height_mm,
            background: PAPER_BACKGROUND,
            paths: Vec::new(),
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        }
    }

    /// Use `background` as the paper colour masks flood with (builder form).
    ///
    /// The writers do not paint a page background rectangle, so this only
    /// affects the mask fills; a caller plotting onto coloured stock supplies
    /// that colour here rather than being silently plotted on white.
    pub fn with_background(mut self, background: [f32; 3]) -> Self {
        self.background = background;
        self
    }

    pub(super) fn report(&mut self, object: Option<ObjectId>, code: &str, message: String) {
        self.completeness = self
            .completeness
            .clone()
            .combine(Completeness::Partial(vec![message.clone()]));
        self.diagnostics.push(Diagnostic {
            object,
            code: code.to_string(),
            message,
        });
    }
}
