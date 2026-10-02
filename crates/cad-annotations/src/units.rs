//! Unit-context encoding and strict decoding (audit B09/B11).

use super::*;

pub(crate) fn encode_units(units: &UnitContext) -> Value {
    json!({
        "source": encode_unit(&units.source),
        "display": encode_unit(&units.display),
        "display_per_source": units.display_per_source,
        "decimal_places": units.decimal_places,
    })
}

fn encode_unit(unit: &Unit) -> Value {
    json!(match unit {
        Unit::DrawingUnits => "DrawingUnits",
        Unit::Millimeter => "Millimeter",
        Unit::Meter => "Meter",
        Unit::Inch => "Inch",
        Unit::Foot => "Foot",
    })
}

fn decode_unit(value: Option<&Value>) -> CadResult<Unit> {
    match value {
        // A missing unit is intentionally Unknown (DrawingUnits); this is the
        // documented "unreadable source stays Unknown" rule (audit B09).
        None | Some(Value::Null) => Ok(Unit::DrawingUnits),
        Some(v) => match v.as_str() {
            Some("DrawingUnits") => Ok(Unit::DrawingUnits),
            Some("Millimeter") => Ok(Unit::Millimeter),
            Some("Meter") => Ok(Unit::Meter),
            Some("Inch") => Ok(Unit::Inch),
            Some("Foot") => Ok(Unit::Foot),
            // An unrecognized unit string is corruption, not silence.
            Some(other) => Err(corrupt(format!("unknown unit '{other}'"))),
            None => Err(corrupt("unit must be a string")),
        },
    }
}

/// Decode a unit context, bounded and strict (audit B09/B11).
///
/// `decimal_places` is range-checked instead of being truncated by an `as u8`
/// cast; `display_per_source` must be finite when present.
pub(crate) fn decode_units(value: Option<&Value>) -> CadResult<UnitContext> {
    let Some(v) = value else {
        return Ok(UnitContext::drawing_units());
    };
    let object = v
        .as_object()
        .ok_or_else(|| corrupt("unit_context must be an object"))?;
    let decimal_places = match object.get("decimal_places") {
        None | Some(Value::Null) => 3u8,
        Some(d) => {
            let raw = d
                .as_u64()
                .ok_or_else(|| corrupt("decimal_places must be an unsigned integer"))?;
            u8::try_from(raw).map_err(|_| corrupt("decimal_places does not fit in 8 bits"))?
        }
    };
    let display_per_source = match object.get("display_per_source") {
        None | Some(Value::Null) => None,
        Some(d) => {
            let n = d
                .as_f64()
                .ok_or_else(|| corrupt("display_per_source must be a number"))?;
            if !n.is_finite() {
                return Err(corrupt("display_per_source must be finite"));
            }
            Some(n)
        }
    };
    Ok(UnitContext {
        source: decode_unit(object.get("source"))?,
        display: decode_unit(object.get("display"))?,
        display_per_source,
        decimal_places,
    })
}
