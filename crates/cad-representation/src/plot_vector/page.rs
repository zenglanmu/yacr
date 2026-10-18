use cad_domain::{CadError, CadResult, Point3};

use crate::PlotPage;

// ---------------------------------------------------------------------------
// Page transform
// ---------------------------------------------------------------------------

/// Maps paper millimetres onto document millimetres (y-up, origin bottom-left).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorPage {
    /// Document width in millimetres.
    pub width_mm: f64,
    /// Document height in millimetres.
    pub height_mm: f64,
    /// Affine paper (mm) → document (mm) as `[a, b, c, d, e, f]`:
    /// `x = a*px + c*py + e`, `y = b*px + d*py + f`.
    pub paper_to_document: [f64; 6],
}

impl VectorPage {
    /// Derive the y-up document transform from a planned pixel page.
    ///
    /// [`PlotPage::paper_to_pixel`] is y-down and scaled by `pixels_per_mm`;
    /// dividing by that scale and flipping y about the document height yields
    /// the physical, y-up page. The physical size is therefore
    /// `width / pixels_per_mm` and `height / pixels_per_mm`.
    pub fn from_plot_page(page: &PlotPage) -> CadResult<VectorPage> {
        let ppm = page.pixels_per_mm;
        if !ppm.is_finite() || ppm <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "plot page has no positive pixels-per-mm scale ({ppm})"
            )));
        }
        let width_mm = page.width as f64 / ppm;
        let height_mm = page.height as f64 / ppm;
        if !width_mm.is_finite() || !height_mm.is_finite() || width_mm <= 0.0 || height_mm <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "plot page has no positive physical size ({width_mm}x{height_mm} mm)"
            )));
        }
        let m = page.paper_to_pixel;
        let paper_to_document = [
            m[0] / ppm,
            -m[1] / ppm,
            m[2] / ppm,
            -m[3] / ppm,
            m[4] / ppm,
            height_mm - m[5] / ppm,
        ];
        if !paper_to_document.iter().all(|v| v.is_finite()) {
            return Err(CadError::InvalidInput(
                "plot page transform has non-finite coefficients".into(),
            ));
        }
        Ok(VectorPage {
            width_mm,
            height_mm,
            paper_to_document,
        })
    }

    /// Map one paper-space point (millimetres) onto the document.
    pub fn map_paper_point(&self, point: Point3) -> [f64; 2] {
        let m = self.paper_to_document;
        [
            m[0] * point.x + m[2] * point.y + m[4],
            m[1] * point.x + m[3] * point.y + m[5],
        ]
    }
}
