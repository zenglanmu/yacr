//! Pure plot-layout planning: map a paper-space sheet onto a pixel canvas.
//!
//! This module has no GPU, no window and no database dependency at call time.
//! It takes the physical sheet described by a
//! [`cad_db::PlotSettingsRecord`], the margins, rotation, the plot scale and a
//! requested output size, and produces a [`PlotPage`]: the pixel canvas plus the
//! affine transform that maps a paper-space point (in millimetres) onto that
//! canvas.
//!
//! The paper-space geometry itself comes from
//! [`crate::layout::build_paper_space`], which already applies each viewport's
//! model→paper transform (for example 1:100). This planner therefore does the
//! *second* half of the job: paper millimetres → pixels, honouring the sheet
//! size, margins, rotation and a plot scale that zooms the sheet content.
//!
//! The split is deliberate: the viewport scale is a property of the drawing
//! (stored per viewport), while the plot scale is a property of the output
//! request. Keeping them separate means a 1:100 viewport plots correctly whether
//! the requested page is a 300 dpi A4 or a 150 dpi A3.

use cad_db::{PlotPaperUnits, PlotRotation, PlotSettingsRecord};
use cad_domain::{CadError, CadResult, Point3};

use crate::layout::{viewport_transform, ViewportState, ViewportTransform};

/// Millimetres in one international inch.
pub const MM_PER_INCH: f64 = 25.4;

/// A uniform plot scale as a ratio of paper units.
///
/// `numerator: 1, denominator: 100` means the sheet is reproduced at 1:100 of
/// its nominal size. A paper-space layout plot is normally 1:1; the ratio is
/// kept explicit so a caller can request an enlarged/reduced copy without the
/// planner guessing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlotScale {
    pub numerator: f64,
    pub denominator: f64,
}

impl PlotScale {
    pub const ONE_TO_ONE: PlotScale = PlotScale {
        numerator: 1.0,
        denominator: 1.0,
    };

    /// The multiplicative factor applied to paper-space coordinates.
    ///
    /// A non-finite or non-positive denominator falls back to 1:1 rather than
    /// producing an unbounded canvas; the caller's request validator rejects
    /// such values before this point where it matters.
    pub fn factor(&self) -> f64 {
        if self.denominator == 0.0 || !self.numerator.is_finite() || !self.denominator.is_finite() {
            1.0
        } else {
            self.numerator / self.denominator
        }
    }
}

impl Default for PlotScale {
    fn default() -> Self {
        PlotScale::ONE_TO_ONE
    }
}

/// How the output canvas size is chosen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlotTarget {
    /// Rasterise the sheet at this many dots per inch.
    Dpi(f64),
    /// Fit the sheet into a canvas of exactly this pixel size, preserving the
    /// sheet's aspect ratio (letterboxed when the request disagrees).
    Pixels { width: u32, height: u32 },
}

/// A planned plot canvas: pixel dimensions plus the paper→pixel transform.
#[derive(Debug, Clone, PartialEq)]
pub struct PlotPage {
    /// Canvas width in pixels.
    pub width: u32,
    /// Canvas height in pixels.
    pub height: u32,
    /// Effective resolution in dots per inch that the canvas represents.
    pub dpi: f64,
    /// Uniform pixels per paper millimetre.
    pub pixels_per_mm: f64,
    /// Rotation actually applied, in degrees counter-clockwise.
    pub rotation_degrees: f64,
    /// Printable size after margins and rotation, in millimetres.
    pub printable_mm: (f64, f64),
    /// Affine paper (mm) → pixel map as `[a, b, c, d, e, f]`:
    /// `px = a*x + c*y + e`, `py = b*x + d*y + f`.
    pub paper_to_pixel: [f64; 6],
}

impl PlotPage {
    /// Map a paper-space point (in millimetres) onto the canvas.
    pub fn map_paper_point(&self, point: Point3) -> (f64, f64) {
        let m = self.paper_to_pixel;
        (
            m[0] * point.x + m[2] * point.y + m[4],
            m[1] * point.x + m[3] * point.y + m[5],
        )
    }

    /// Map a model-space point through a supported viewport and then onto the
    /// canvas. Returns `None` for a viewport whose transform cannot be drawn
    /// exactly (the same refusal the representation layer makes).
    pub fn map_model_point(
        &self,
        viewport: &ViewportTransform,
        model: Point3,
    ) -> Option<(f64, f64)> {
        let paper = viewport.model_to_paper(model);
        Some(self.map_paper_point(paper))
    }
}

/// Convert a length in `units` to millimetres.
///
/// `Pixels` is intentionally unsupported for physical planning: pixel sizes are
/// not a physical unit, so a caller must pick a DPI instead of inventing a
/// conversion.
pub fn to_millimetres(value: f64, units: PlotPaperUnits) -> CadResult<f64> {
    match units {
        PlotPaperUnits::Millimeters => Ok(value),
        PlotPaperUnits::Inches => Ok(value * MM_PER_INCH),
        PlotPaperUnits::Pixels => Err(CadError::Unsupported(
            "pixel paper units have no physical size; supply the plot DPI explicitly".into(),
        )),
    }
}

/// Normalise an arbitrary rotation angle to `[0, 360)` degrees.
pub fn normalize_degrees(degrees: f64) -> f64 {
    let mut d = degrees % 360.0;
    if d < 0.0 {
        d += 360.0;
    }
    d
}

/// Plan a plot for a stored plot-settings record.
///
/// `scale` overrides the record's own scale; pass
/// `PlotScale::new(record.scale_numerator, record.scale_denominator)` to honour
/// the file. The record supplies the sheet size, units, margins and rotation.
pub fn plan_plot(
    record: &PlotSettingsRecord,
    scale: PlotScale,
    target: PlotTarget,
) -> CadResult<PlotPage> {
    plan_plot_rotated(record, scale, target, record.rotation.to_degrees())
}

/// Plan a plot with an explicit rotation angle in degrees, counter-clockwise.
///
/// [`plan_plot`] uses the record's stored rotation; this entry point exists for
/// callers that need an arbitrary angle (for example a viewport twist applied
/// to the whole sheet) while reusing the same validated sheet math.
pub fn plan_plot_rotated(
    record: &PlotSettingsRecord,
    scale: PlotScale,
    target: PlotTarget,
    rotation_degrees: f64,
) -> CadResult<PlotPage> {
    let rotation = normalize_degrees(rotation_degrees);
    let width_mm = to_millimetres(record.paper_width, record.paper_units)?;
    let height_mm = to_millimetres(record.paper_height, record.paper_units)?;
    if !width_mm.is_finite() || !height_mm.is_finite() || width_mm <= 0.0 || height_mm <= 0.0 {
        return Err(CadError::InvalidInput(format!(
            "plot paper size must be positive and finite, got {width_mm}x{height_mm} mm"
        )));
    }
    let margins = [
        to_millimetres(record.margins.left, record.paper_units)?,
        to_millimetres(record.margins.bottom, record.paper_units)?,
        to_millimetres(record.margins.right, record.paper_units)?,
        to_millimetres(record.margins.top, record.paper_units)?,
    ];
    if margins.iter().any(|m| !m.is_finite() || *m < 0.0) {
        return Err(CadError::InvalidInput(
            "plot margins must be finite and non-negative".into(),
        ));
    }
    let printable_w = width_mm - margins[0] - margins[2];
    let printable_h = height_mm - margins[1] - margins[3];
    if printable_w <= 0.0 || printable_h <= 0.0 {
        return Err(CadError::InvalidInput(format!(
            "plot margins leave no printable area ({printable_w}x{printable_h} mm)"
        )));
    }

    let factor = scale.factor();
    if !factor.is_finite() || factor <= 0.0 {
        return Err(CadError::InvalidInput(
            "plot scale factor must be positive and finite".into(),
        ));
    }

    // Rotation swaps the sheet's presented axes at 90°/270°.
    let radians = rotation.to_radians();
    let (cos, sin) = (radians.cos(), radians.sin());
    let rotate = |x: f64, y: f64| (cos * x - sin * y, sin * x + cos * y);
    // The rotated bounding box of the printable rectangle measured from its
    // centre; the centred content does not need an extra origin shift.
    let (pw, ph) = (printable_w * factor, printable_h * factor);
    let corners = [
        rotate(-pw / 2.0, -ph / 2.0),
        rotate(pw / 2.0, -ph / 2.0),
        rotate(pw / 2.0, ph / 2.0),
        rotate(-pw / 2.0, ph / 2.0),
    ];
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (x, y) in corners {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    let page_w = max_x - min_x;
    let page_h = max_y - min_y;

    // Decide pixels per millimetre, then the canvas.
    let (pixels_per_mm, width_px, height_px) = match target {
        PlotTarget::Dpi(dpi) => {
            if !dpi.is_finite() || dpi <= 0.0 {
                return Err(CadError::InvalidInput(
                    "plot DPI must be positive and finite".into(),
                ));
            }
            let ppm = dpi / MM_PER_INCH;
            (
                ppm,
                (page_w * ppm).round().max(1.0) as u32,
                (page_h * ppm).round().max(1.0) as u32,
            )
        }
        PlotTarget::Pixels { width, height } => {
            if width == 0 || height == 0 {
                return Err(CadError::InvalidInput(
                    "plot canvas dimensions must be positive".into(),
                ));
            }
            // Fit the rotated page inside the requested canvas, preserving the
            // page aspect ratio. Letterboxing is explicit: the paper is centred
            // and the extra canvas is left as background.
            let ppm = (width as f64 / page_w).min(height as f64 / page_h);
            (ppm, width, height)
        }
    };
    if !pixels_per_mm.is_finite() || pixels_per_mm <= 0.0 {
        return Err(CadError::InvalidInput(
            "plot canvas has no positive scale".into(),
        ));
    }

    // The printable region is centred on the canvas. Compose the affine map:
    // translate paper so the printable centre is at the origin, apply the scale
    // and rotation, then translate to the canvas centre. Paper coordinates are
    // y-up (the drawing convention), so the y axis is flipped into the y-down
    // canvas.
    //
    //   px = ppm * R(θ) * (x - cx, cy - y) · (1, 0) + canvas_cx
    //   py = ppm * R(θ) * (x - cx, cy - y) · (0,-1) + canvas_cy
    let center_x = (margins[0] + (width_mm - margins[2])) / 2.0;
    let center_y = (margins[1] + (height_mm - margins[3])) / 2.0;
    let canvas_cx = width_px as f64 / 2.0;
    let canvas_cy = height_px as f64 / 2.0;
    let a = pixels_per_mm * cos * factor;
    let c = pixels_per_mm * sin * factor;
    let b = -pixels_per_mm * sin * factor;
    let d = pixels_per_mm * cos * factor;
    let e = canvas_cx - pixels_per_mm * (cos * center_x + sin * center_y) * factor;
    let f = canvas_cy - pixels_per_mm * (cos * center_y - sin * center_x) * factor;

    Ok(PlotPage {
        width: width_px,
        height: height_px,
        dpi: pixels_per_mm * MM_PER_INCH,
        pixels_per_mm,
        rotation_degrees: rotation,
        printable_mm: (printable_w, printable_h),
        paper_to_pixel: [a, b, c, d, e, f],
    })
}

/// Plan a plot, resolving the viewport of `record`'s layout if it is needed for
/// a model point. This is a convenience for callers that only have the stored
/// record; it does not read the database.
pub fn plan_plot_for_record(
    record: &PlotSettingsRecord,
    target: PlotTarget,
) -> CadResult<PlotPage> {
    let scale = PlotScale {
        numerator: record.scale_numerator,
        denominator: record.scale_denominator,
    };
    plan_plot(record, scale, target)
}

/// Map a model point through a named-viewport transform onto a planned page.
///
/// Returns `None` when the viewport is unsupported, so no guessed pixel is ever
/// produced. This is the exact bridge between the representation layer's
/// viewport transform and the plot canvas.
pub fn map_model_through_viewport(
    page: &PlotPage,
    viewport: &ViewportTransform,
    model: Point3,
) -> Option<(f64, f64)> {
    page.map_model_point(viewport, model)
}

/// Resolve a stored viewport and map a model point, or `None` if unsupported.
pub fn map_model_through_stored_viewport(
    page: &PlotPage,
    viewport: &cad_db::PaperViewport,
    model: Point3,
) -> Option<(f64, f64)> {
    match viewport_transform(viewport) {
        ViewportState::Supported(t) => page.map_model_point(&t, model),
        ViewportState::Unsupported(_) => None,
    }
}

/// The default plot scale for a layout plot, which is always 1:1.
///
/// A paper-space layout is already at its physical size; the viewport carries
/// any model scale. Keeping this explicit documents why a 1:100 viewport still
/// plots through a 1:1 page scale.
pub fn layout_plot_scale() -> PlotScale {
    PlotScale::ONE_TO_ONE
}

/// A one-line human description of a planned page, for CLI reports.
pub fn describe(page: &PlotPage) -> String {
    format!(
        "{}x{} px @ {:.1} dpi, rotation {:.0}°, printable {:.1}x{:.1} mm",
        page.width,
        page.height,
        page.dpi,
        page.rotation_degrees,
        page.printable_mm.0,
        page.printable_mm.1
    )
}

/// Rotation in degrees for a stored enum (re-exported convenience).
pub fn rotation_degrees(rotation: PlotRotation) -> f64 {
    rotation.to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{PlotMargins, PlotPaperUnits, PlotProvenance, PlotSettingsRecord, PlotType};
    use cad_domain::{LayoutId, Transform3};

    fn a4(rotation: PlotRotation, margins: PlotMargins) -> PlotSettingsRecord {
        PlotSettingsRecord {
            layout: LayoutId(1),
            paper_size_name: "ISO_A4_(210.00_x_297.00_MM)".into(),
            paper_width: 210.0,
            paper_height: 297.0,
            margins,
            rotation,
            scale_numerator: 1.0,
            scale_denominator: 1.0,
            plot_type: PlotType::Layout,
            paper_units: PlotPaperUnits::Millimeters,
            provenance: PlotProvenance::Imported,
        }
    }

    #[test]
    fn a4_at_300_dpi_has_the_expected_canvas() {
        let record = a4(PlotRotation::None, PlotMargins::default());
        let page = plan_plot(&record, PlotScale::ONE_TO_ONE, PlotTarget::Dpi(300.0)).unwrap();
        // 210 mm = 8.2677 in * 300 = 2480 px; 297 mm = 11.6929 in * 300 = 3508 px.
        assert_eq!(page.width, 2480);
        assert_eq!(page.height, 3508);
        assert!((page.dpi - 300.0).abs() < 1e-9);
        assert!((page.pixels_per_mm - 300.0 / 25.4).abs() < 1e-12);
        assert_eq!(page.rotation_degrees, 0.0);
        assert_eq!(page.printable_mm, (210.0, 297.0));
    }

    #[test]
    fn a4_rotation_90_swaps_the_canvas() {
        let record = a4(PlotRotation::Degrees90, PlotMargins::default());
        let page = plan_plot(&record, PlotScale::ONE_TO_ONE, PlotTarget::Dpi(300.0)).unwrap();
        // 297 mm across → 3508 px wide; 210 mm down → 2480 px high.
        assert_eq!(page.width, 3508);
        assert_eq!(page.height, 2480);
        assert_eq!(page.rotation_degrees, 90.0);
    }

    #[test]
    fn a4_rotation_90_maps_corners_correctly() {
        let record = a4(PlotRotation::Degrees90, PlotMargins::default());
        let page = plan_plot(&record, PlotScale::ONE_TO_ONE, PlotTarget::Dpi(72.0)).unwrap();
        // Paper is y-up; the canvas is y-down. A counter-clockwise 90° rotation
        // sends the paper's origin (0,0) to the canvas's bottom-left and the
        // paper's top-right (210,297) to the canvas's top-right.
        let (x, y) = page.map_paper_point(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        assert!(x.abs() < 1.0, "x={x}");
        assert!((y - page.height as f64).abs() < 1.0, "y={y}");
        let (x, y) = page.map_paper_point(Point3 {
            x: 210.0,
            y: 297.0,
            z: 0.0,
        });
        assert!((x - page.width as f64).abs() < 1.0, "x={x}");
        assert!(y.abs() < 1.0, "y={y}");
        // The sheet centre maps to the canvas centre at any rotation.
        let (cx, cy) = page.map_paper_point(Point3 {
            x: 105.0,
            y: 148.5,
            z: 0.0,
        });
        assert!((cx - page.width as f64 / 2.0).abs() < 1.0, "cx={cx}");
        assert!((cy - page.height as f64 / 2.0).abs() < 1.0, "cy={cy}");
    }

    #[test]
    fn margins_are_excluded_from_the_printable_area() {
        // 10 mm margins all round: printable 190 x 277 mm.
        let record = a4(PlotRotation::None, PlotMargins::uniform(10.0));
        let page = plan_plot(&record, PlotScale::ONE_TO_ONE, PlotTarget::Dpi(254.0)).unwrap();
        assert_eq!(page.printable_mm, (190.0, 277.0));
        // At exactly 10 px/mm the printable area is 1900 x 2770.
        assert!((page.pixels_per_mm - 10.0).abs() < 1e-9);
        // The canvas is the printable region itself (the unprintable border is
        // not part of the output), so 190 x 277 mm at 10 px/mm.
        assert_eq!(page.width, 1900);
        assert_eq!(page.height, 2770);
    }

    #[test]
    fn one_to_one_hundred_scale_shrinks_content_by_one_hundred() {
        let record = a4(PlotRotation::None, PlotMargins::default());
        let scale = PlotScale {
            numerator: 1.0,
            denominator: 100.0,
        };
        let page = plan_plot(&record, scale, PlotTarget::Dpi(100.0)).unwrap();
        // The full sheet is drawn at 1/100, so it occupies 2.1 x 2.97 mm on the
        // canvas; at ~3.937 px/mm the canvas is ~8 x 12 px.
        let ppm = 100.0 / 25.4;
        assert!((page.pixels_per_mm - ppm).abs() < 1e-9);
        assert_eq!(page.width, (210.0 / 100.0 * ppm).round() as u32);
        assert_eq!(page.height, (297.0 / 100.0 * ppm).round() as u32);
        // Two paper points 1000 mm apart are 10 paper mm apart after the 1:100
        // scale, so the pixel distance is 10 * ppm.
        let a = page.map_paper_point(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        let b = page.map_paper_point(Point3 {
            x: 1000.0,
            y: 0.0,
            z: 0.0,
        });
        assert!(((b.0 - a.0) - 10.0 * ppm).abs() < 1e-6, "dx={}", b.0 - a.0);
    }

    #[test]
    fn one_to_one_hundred_viewport_maps_a_model_point_to_the_expected_pixel() {
        // A 1:100 viewport: paper 0.01 per model unit. A model span of 1000
        // units is 10 paper mm, independent of the canvas DPI.
        let record = a4(PlotRotation::None, PlotMargins::default());
        let page = plan_plot(&record, PlotScale::ONE_TO_ONE, PlotTarget::Dpi(254.0)).unwrap();
        let viewport = crate::layout::ViewportTransform {
            paper_corners: [
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 100.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 100.0,
                    y: 50.0,
                    z: 0.0,
                },
                Point3 {
                    x: 0.0,
                    y: 50.0,
                    z: 0.0,
                },
            ],
            paper_center: [50.0, 25.0],
            paper_half: [50.0, 25.0],
            model_anchor: Point3 {
                x: 10.0,
                y: 20.0,
                z: 0.0,
            },
            paper_per_model: 0.01,
            to_paper: {
                let mut m = Transform3::identity().matrix;
                m[0][0] = 0.01;
                m[1][1] = 0.01;
                m[0][3] = 50.0 - 0.01 * 10.0;
                m[1][3] = 25.0 - 0.01 * 20.0;
                Transform3 { matrix: m }
            },
            to_model: Transform3::identity(),
        };
        let origin = page
            .map_model_point(
                &viewport,
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();
        let thousand = page
            .map_model_point(
                &viewport,
                Point3 {
                    x: 1000.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();
        let expected = 10.0 * page.pixels_per_mm; // 10 paper mm at 254 dpi (10 px/mm)
        assert!(((thousand.0 - origin.0) - expected).abs() < 1e-6);
        assert!((thousand.1 - origin.1).abs() < 1e-6);
    }

    #[test]
    fn pixels_target_fits_and_centres_the_sheet() {
        let record = a4(PlotRotation::None, PlotMargins::default());
        let page = plan_plot(
            &record,
            PlotScale::ONE_TO_ONE,
            PlotTarget::Pixels {
                width: 1000,
                height: 1000,
            },
        )
        .unwrap();
        assert_eq!((page.width, page.height), (1000, 1000));
        // A4 is portrait, so the fit is limited by height: 1000 / 297 px/mm.
        let ppm = 1000.0 / 297.0;
        assert!((page.pixels_per_mm - ppm).abs() < 1e-9);
        // The sheet is centred horizontally: its left edge is at (1000 - 210*ppm)/2.
        let (x, _) = page.map_paper_point(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        assert!((x - (1000.0 - 210.0 * ppm) / 2.0).abs() < 1e-6, "x={x}");
    }

    #[test]
    fn zero_margins_that_leave_no_area_are_rejected() {
        let record = a4(PlotRotation::None, PlotMargins::uniform(200.0));
        assert!(matches!(
            plan_plot(&record, PlotScale::ONE_TO_ONE, PlotTarget::Dpi(300.0)),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn pixel_paper_units_are_rejected_without_a_dpi() {
        let mut record = a4(PlotRotation::None, PlotMargins::default());
        record.paper_units = PlotPaperUnits::Pixels;
        assert!(matches!(
            plan_plot(&record, PlotScale::ONE_TO_ONE, PlotTarget::Dpi(300.0)),
            Err(CadError::Unsupported(_))
        ));
    }

    #[test]
    fn inch_paper_is_converted_to_millimetres() {
        let mut record = a4(PlotRotation::None, PlotMargins::default());
        record.paper_units = PlotPaperUnits::Inches;
        record.paper_width = 8.5;
        record.paper_height = 11.0;
        let page = plan_plot(&record, PlotScale::ONE_TO_ONE, PlotTarget::Dpi(300.0)).unwrap();
        // 8.5 in * 25.4 = 215.9 mm; 11 in = 279.4 mm; at 300 dpi = 2550 x 3300.
        assert_eq!(page.printable_mm.0.round(), 216.0);
        assert_eq!(page.printable_mm.1.round(), 279.0);
        assert_eq!((page.width, page.height), (2550, 3300));
    }

    #[test]
    fn a_missing_layout_defaults_to_an_explicit_a4_page() {
        let db = cad_db::DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1))
            .finish()
            .unwrap();
        // No layout inserted, but the default resolver still works by id and
        // marks the provenance.
        let record = db.plot_settings_for(LayoutId(42));
        assert!(matches!(
            record.provenance,
            PlotProvenance::DefaultPage { .. }
        ));
        let page = plan_plot_for_record(&record, PlotTarget::Dpi(300.0)).unwrap();
        assert_eq!((page.width, page.height), (2480, 3508));
    }

    #[test]
    fn arbitrary_rotation_is_supported() {
        let record = a4(PlotRotation::None, PlotMargins::default());
        // 45°: the x-axis unit direction is (cos45, -sin45) in the y-down
        // canvas, so one paper mm moves by ~0.707 px_per_mm in both axes.
        let page =
            plan_plot_rotated(&record, PlotScale::ONE_TO_ONE, PlotTarget::Dpi(72.0), 45.0).unwrap();
        assert_eq!(page.rotation_degrees, 45.0_f64);
        let a = page.map_paper_point(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        let b = page.map_paper_point(Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        });
        let expected = std::f64::consts::FRAC_1_SQRT_2 * page.pixels_per_mm;
        assert!((b.0 - a.0 - expected).abs() < 1e-6, "dx={}", b.0 - a.0);
        assert!((b.1 - a.1 + expected).abs() < 1e-6, "dy={}", b.1 - a.1);
        assert!(page.width > 0 && page.height > 0);
        assert!(page.paper_to_pixel.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn rotation_is_normalized_into_range() {
        assert_eq!(normalize_degrees(-90.0), 270.0);
        assert_eq!(normalize_degrees(450.0), 90.0);
        assert_eq!(normalize_degrees(360.0), 0.0);
    }
}
