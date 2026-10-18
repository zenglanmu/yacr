use super::model::{ParsedText, TextRun};

/// Default baseline-to-baseline spacing as a multiple of the line's height.
pub const LINE_SPACING: f64 = 1.2;

/// One laid-out paragraph before alignment/rotation.
#[derive(Clone)]
pub(super) struct ShapedLine {
    /// Glyph-local polylines with the pen already applied, width factor and
    /// oblique shear included, baseline at y=0.
    pub(super) polys: Vec<Vec<[f64; 2]>>,
    /// Horizontal advance of the line in world units.
    pub(super) width: f64,
    /// Height of the tallest run on the line (drives line spacing).
    pub(super) height: f64,
}

pub(super) struct MeasuredRun {
    pub(super) geometry: (Vec<Vec<[f64; 2]>>, f64),
    pub(super) missing_glyphs: Vec<char>,
}

/// Lay out parsed paragraphs run by run, applying per-run width factor and
/// oblique shear.
///
/// `measure(run, text)` returns the run's glyph polylines positioned with a
/// run-local pen (baseline at y=0) and the *unscaled* advance; the width factor
/// and oblique shear are applied here so the math is testable without a font.
pub(super) fn layout_parsed<F>(
    parsed: &ParsedText,
    fallback_height: f64,
    mut measure: F,
) -> Vec<ShapedLine>
where
    F: FnMut(&TextRun, &str) -> Option<(Vec<Vec<[f64; 2]>>, f64)>,
{
    let fallback = if fallback_height.is_finite() && fallback_height > 0.0 {
        fallback_height
    } else {
        1.0
    };
    let mut out = Vec::new();
    for line in &parsed.lines {
        let mut pen = 0.0f64;
        let mut polys: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut line_height = fallback;
        for run in &line.runs {
            let text = run.rendered_text();
            if text.is_empty() {
                continue;
            }
            let height = run.effective_height(fallback);
            line_height = line_height.max(height);
            // Guard against non-finite/negative style values rather than
            // emitting NaN geometry.
            let width_factor = if run.width_factor.is_finite() && run.width_factor > 0.0 {
                run.width_factor
            } else {
                1.0
            };
            let oblique = if run.oblique_deg.is_finite() {
                run.oblique_deg.to_radians().tan()
            } else {
                0.0
            };
            let Some((glyph_polys, advance)) = measure(run, &text) else {
                continue;
            };
            for poly in glyph_polys {
                let mapped: Vec<[f64; 2]> = poly
                    .iter()
                    .map(|[gx, gy]| [pen + gx * width_factor + gy * oblique, *gy])
                    .collect();
                polys.push(mapped);
            }
            if advance.is_finite() {
                pen += advance * width_factor;
            }
        }
        out.push(ShapedLine {
            polys,
            width: pen,
            height: line_height,
        });
    }
    out
}
