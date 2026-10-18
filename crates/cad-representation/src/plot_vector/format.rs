use cad_domain::{CadError, CadResult};

use super::model::VectorDocument;

// ---------------------------------------------------------------------------
// Shared formatting/validation helpers
// ---------------------------------------------------------------------------

pub(super) fn validate_document(document: &VectorDocument) -> CadResult<()> {
    if !document.width_mm.is_finite()
        || !document.height_mm.is_finite()
        || document.width_mm <= 0.0
        || document.height_mm <= 0.0
    {
        return Err(CadError::InvalidInput(format!(
            "vector document size must be positive and finite, got {}x{} mm",
            document.width_mm, document.height_mm
        )));
    }
    for path in &document.paths {
        if path.points.len() < 2 {
            return Err(CadError::InvalidInput(
                "vector path has fewer than two points".into(),
            ));
        }
        if !path.stroke_width_mm.is_finite() || path.stroke_width_mm < 0.0 {
            return Err(CadError::InvalidInput(
                "vector path has an invalid stroke width".into(),
            ));
        }
        if !path.opacity.is_finite() {
            return Err(CadError::InvalidInput(
                "vector path has a non-finite opacity".into(),
            ));
        }
        if path.fill.is_none() && path.stroke.is_none() {
            return Err(CadError::InvalidInput(
                "vector path has neither fill nor stroke".into(),
            ));
        }
        for point in &path.points {
            if !point[0].is_finite() || !point[1].is_finite() {
                return Err(CadError::InvalidInput(
                    "vector path has a non-finite coordinate".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Deterministic decimal formatting: fixed notation, no exponent, trimmed.
///
/// Finite values only; callers validate first. `-0` is normalised to `0`.
pub(super) fn number(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".to_string();
    }
    let mut text = format!("{value:.6}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".to_string();
    }
    text
}

pub(super) fn hex_color(color: [f32; 3]) -> String {
    let channel = |value: f32| -> u32 { (value.clamp(0.0, 1.0) * 255.0).round() as u32 };
    format!(
        "#{:02X}{:02X}{:02X}",
        channel(color[0]),
        channel(color[1]),
        channel(color[2])
    )
}

pub(super) fn pdf_components(color: [f32; 3]) -> String {
    let component = |value: f32| number(value.clamp(0.0, 1.0) as f64);
    format!(
        "{} {} {}",
        component(color[0]),
        component(color[1]),
        component(color[2])
    )
}
