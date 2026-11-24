//! The document's `UNITS` display settings on the status wire
//! (`cad_status.units`).
//!
//! One additive, optional block carries exactly what the React readouts need to
//! format a coordinate or a measured value: the linear format and precision,
//! the angular format and precision, and the base-angle/rotation direction. It
//! is projected verbatim from the document header (`cad_db::UnitsDisplay` plus
//! the document's `UnitContext::decimal_places`); the wire never invents or
//! re-derives a setting. `None` means "no document", and an older host simply
//! omits the key — the readouts then keep their legacy fixed format.
//!
//! `$INSUNITS` insertion scaling and the host Units dialog are deliberately not
//! part of this block: they are declared global gaps, not readout settings.

use serde::Serialize;

/// The readout-relevant slice of one document's `UNITS` settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadUnits {
    /// `scientific` | `decimal` | `engineering` | `architectural` | `fractional`.
    pub length_format: String,
    /// Linear decimal places (`UnitContext::decimal_places`, `0..=8`).
    pub length_precision: u8,
    /// `decimal_degrees` | `degrees_minutes_seconds` | `gradians` | `radians` |
    /// `surveyors_units`.
    pub angle_format: String,
    /// Angular decimal places (`0..=8`).
    pub angle_precision: u8,
    /// `ANGBASE`: the direction of zero, in degrees.
    pub angle_base_degrees: f64,
    /// `ANGDIR`: `true` means positive angles run clockwise.
    pub angle_clockwise: bool,
}

impl CadUnits {
    /// Project the stored document values onto the wire block.
    pub fn from_parts(units: &cad_domain::UnitContext, display: &cad_db::UnitsDisplay) -> Self {
        CadUnits {
            length_format: display.length_format.as_token().to_string(),
            length_precision: units.decimal_places,
            angle_format: display.angle_format.as_token().to_string(),
            angle_precision: display.angle_precision,
            angle_base_degrees: display.angle_base_degrees,
            angle_clockwise: display.angle_clockwise,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{AngleFormat, LengthFormat};

    #[test]
    fn the_wire_block_is_the_document_header_verbatim() {
        let units = cad_domain::UnitContext::from_meter();
        let display = cad_db::UnitsDisplay {
            length_format: LengthFormat::Architectural,
            angle_format: AngleFormat::SurveyorsUnits,
            angle_precision: 3,
            angle_base_degrees: 90.0,
            angle_clockwise: true,
        };
        let block = CadUnits::from_parts(&units, &display);
        assert_eq!(block.length_format, "architectural");
        assert_eq!(block.length_precision, units.decimal_places);
        assert_eq!(block.angle_format, "surveyors_units");
        assert_eq!(block.angle_precision, 3);
        assert_eq!(block.angle_base_degrees, 90.0);
        assert!(block.angle_clockwise);
    }
}
