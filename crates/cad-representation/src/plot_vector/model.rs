use cad_domain::{Completeness, Diagnostic, ObjectId};

use super::page::VectorPage;

// ---------------------------------------------------------------------------
// Document model
// ---------------------------------------------------------------------------

/// One path in the document, in page-millimetre y-up coordinates.
///
/// `fill = Some` paints the interior; `stroke = Some` draws the outline with
/// `stroke_width_mm`. `opacity` is the resolved per-entity opacity in `[0, 1]`.
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
            paths: Vec::new(),
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        }
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
