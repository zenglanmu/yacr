//! Attribute validation (ADR-0055), split out for the code-size gate.
//!
//! Shared by `ATTDEF` definitions and `INSERT` values: both own the same text
//! fields, so one rule keeps "a value that cannot be drawn" out of storage.

use cad_domain::{AttributeText, CadResult, Point3};

use super::degenerate_reason;

/// Stable reason code: an attribute definition/value has an empty tag.
///
/// The tag is the identity of an attribute within its block (DXF group 2), so an
/// empty one cannot be matched, edited or extracted. Duplicate tags are **not**
/// refused here: real files do carry them, and the model keeps the import honest
/// instead of dropping or renaming a definition.
pub const REASON_ATTRIBUTE_TAG: &str = "attribute.tag";

/// Stable reason code: an attribute's text placement cannot be drawn (a
/// non-finite position/oblique, a non-positive height or width factor, a
/// non-positive wrap boundary or line-spacing factor).
pub const REASON_ATTRIBUTE_TEXT: &str = "attribute.text";

/// Validate an attribute tag and its text placement (ADR-0055).
///
/// Shared by `ATTDEF` definitions and `INSERT` values: both own the same text
/// fields, and a value that cannot be drawn must not be stored silently.
pub(crate) fn validate_attribute_text(text: &AttributeText, tag: &str) -> CadResult<()> {
    if tag.trim().is_empty() {
        return Err(degenerate_reason(
            REASON_ATTRIBUTE_TAG,
            "an attribute tag may not be empty",
        ));
    }
    let finite = |p: Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
    if !finite(text.position) {
        return Err(degenerate_reason(
            REASON_ATTRIBUTE_TEXT,
            &format!("attribute {tag:?} has a non-finite position"),
        ));
    }
    if !text.height.is_finite() || text.height <= 0.0 {
        return Err(degenerate_reason(
            REASON_ATTRIBUTE_TEXT,
            &format!(
                "attribute {tag:?} height must be positive and finite, got {}",
                text.height
            ),
        ));
    }
    if !text.width_factor.is_finite() || text.width_factor <= 0.0 {
        return Err(degenerate_reason(
            REASON_ATTRIBUTE_TEXT,
            &format!(
                "attribute {tag:?} width factor must be positive and finite, got {}",
                text.width_factor
            ),
        ));
    }
    if !text.rotation.is_finite() || !text.oblique.is_finite() {
        return Err(degenerate_reason(
            REASON_ATTRIBUTE_TEXT,
            &format!("attribute {tag:?} has a non-finite rotation or oblique angle"),
        ));
    }
    if let Some(width) = text.wrap_width {
        if !width.is_finite() || width <= 0.0 {
            return Err(degenerate_reason(
                REASON_ATTRIBUTE_TEXT,
                &format!("attribute {tag:?} wrap width must be positive and finite when set"),
            ));
        }
    }
    if !text.line_spacing.factor.is_finite() || text.line_spacing.factor <= 0.0 {
        return Err(degenerate_reason(
            REASON_ATTRIBUTE_TEXT,
            &format!(
                "attribute {tag:?} line-spacing factor must be positive and finite, got {}",
                text.line_spacing.factor
            ),
        ));
    }
    Ok(())
}
