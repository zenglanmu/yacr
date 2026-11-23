//! The value half of the BIM → IFC4 mapping (ADR-0042 D10).
//!
//! Split out of `bim.rs` so that file stays under the 1000-line gate: this
//! module owns the *values* (the `BimValue` → `PropertyValue` mapping and its
//! unit handling) and the `IfcGloballyUniqueId` codec; `bim.rs` owns the plan
//! (storeys, types, containers, materials).
//!
//! Every refusal here is a named loss counted by [`Counts`], never a silent
//! relabel: a foot value is converted to metres or not written, an explicit
//! null is skipped and counted, and an unknown measure token keeps the number
//! as an `IfcReal` while losing the token loudly.

use std::collections::BTreeSet;

use cad_db::{BimEvidence, BimPropertySet, BimValue};
use ifc_lite_export::plan::{NewProperty, NewPropertySet, PropertyValue, PsetKind};

use crate::export::bim::{BimExportPolicy, Counts};

/// The numeric IFC measures this writer lets a real value keep its type.
///
/// All are REAL/NUMBER/INTEGER `IfcValue` members of IFC4; anything else is
/// written as a plain `IfcReal` and counted, so a value is never relabelled
/// with a measure the reader would interpret differently.
const NUMERIC_MEASURES: &[&str] = &[
    "IfcAccelerationMeasure",
    "IfcAreaMeasure",
    "IfcCountMeasure",
    "IfcElectricCurrentMeasure",
    "IfcElectricVoltageMeasure",
    "IfcEnergyMeasure",
    "IfcForceMeasure",
    "IfcFrequencyMeasure",
    "IfcIlluminanceMeasure",
    "IfcInteger",
    "IfcLengthMeasure",
    "IfcLinearVelocityMeasure",
    "IfcLuminousFluxMeasure",
    "IfcMassDensityMeasure",
    "IfcMassMeasure",
    "IfcNonNegativeLengthMeasure",
    "IfcNormalisedRatioMeasure",
    "IfcNumericMeasure",
    "IfcPlaneAngleMeasure",
    "IfcPositiveLengthMeasure",
    "IfcPositivePlaneAngleMeasure",
    "IfcPositiveRatioMeasure",
    "IfcPowerMeasure",
    "IfcPressureMeasure",
    "IfcRatioMeasure",
    "IfcReal",
    "IfcSoundPowerMeasure",
    "IfcSoundPressureMeasure",
    "IfcThermalResistanceMeasure",
    "IfcThermalTransmittanceMeasure",
    "IfcThermodynamicTemperatureMeasure",
    "IfcTimeMeasure",
    "IfcVolumeMeasure",
];

/// The metre factor of an IFC length unit symbol (the symbols
/// this crate's reader stores in `BimValue::Real.unit`).
///
/// A value whose unit is not convertible here is **not** written: writing a
/// foot value as metres is exactly the silent unit error D10 forbids.
pub(crate) fn length_factor(symbol: &str) -> Option<f64> {
    match symbol.trim() {
        "m" => Some(1.0),
        "mm" => Some(0.001),
        "cm" => Some(0.01),
        "km" => Some(1000.0),
        "ft" => Some(0.3048),
        "in" => Some(0.0254),
        "yd" => Some(0.9144),
        "mi" => Some(1609.344),
        _ => None,
    }
}

/// A unit symbol that already is the project's SI base for its dimension.
fn is_si_base_unit(symbol: &str) -> bool {
    matches!(
        symbol.trim(),
        "m\u{b2}" | "m\u{b3}" | "s" | "kg" | "K" | "A" | "mol" | "cd" | "rad" | "sr"
    )
}

pub(crate) fn is_length_measure(measure: &str) -> bool {
    matches!(
        measure,
        "IfcLengthMeasure" | "IfcPositiveLengthMeasure" | "IfcNonNegativeLengthMeasure"
    )
}

/// The canonical spelling of a numeric measure token, when this writer accepts
/// it as one.
pub(crate) fn numeric_measure(raw: &str) -> Option<&'static str> {
    NUMERIC_MEASURES
        .iter()
        .copied()
        .find(|measure| measure.eq_ignore_ascii_case(raw.trim()))
}

/// A `Measure` for a numeric value, or `None` when the measure's own domain
/// rule (positivity, unit interval, whole number) rejects it.
pub(crate) fn measure_value(measure: &'static str, value: f64) -> Option<PropertyValue> {
    if !value.is_finite() {
        return None;
    }
    let in_domain = match measure {
        "IfcPositiveLengthMeasure" | "IfcPositivePlaneAngleMeasure" | "IfcPositiveRatioMeasure" => {
            value > 0.0
        }
        "IfcNonNegativeLengthMeasure" => value >= 0.0,
        "IfcNormalisedRatioMeasure" => (0.0..=1.0).contains(&value),
        "IfcInteger" => value.fract() == 0.0,
        _ => true,
    };
    in_domain.then(|| PropertyValue::measure(measure, value))
}

/// The base64 digit of one IFC GlobalId character (`0-9A-Za-z_$`).
fn base64_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'Z' => Some(byte - b'A' + 10),
        b'a'..=b'z' => Some(byte - b'a' + 36),
        b'_' => Some(62),
        b'$' => Some(63),
        _ => None,
    }
}

/// The 128 bits a 22-character `IfcGloballyUniqueId` encodes.
///
/// The inverse of [`ifc_lite_export::plan::GlobalId::encode`]: the IFC alphabet
/// (`0-9A-Za-z_$`), most significant character first, where the first character
/// carries only 2 bits (`0`..`3`) because 22 × 6 = 132 bits hold a 128-bit
/// GUID. Returns `None` for anything else — a wrong length, a character
/// outside the alphabet, or a first character that would need a 129th bit.
pub fn decode_global_id(token: &str) -> Option<u128> {
    if token.len() != 22 || !token.is_ascii() {
        return None;
    }
    let mut bits = 0u128;
    for (index, byte) in token.bytes().enumerate() {
        let digit = u128::from(base64_digit(byte)?);
        if index == 0 && digit > 3 {
            return None;
        }
        bits = (bits << 6) | digit;
    }
    Some(bits)
}

/// FNV-1a over the text, so a composite definition id maps to a stable index.
pub(crate) fn hash_text(text: &str) -> u128 {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut hash = OFFSET;
    for byte in text.as_bytes().iter().chain(&[0xff]) {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// One property value as IFC can carry it, or `None` when it cannot be written.
pub(crate) fn converted_value(value: &BimValue, counts: &mut Counts) -> Option<PropertyValue> {
    match value {
        // `IfcPropertySingleValue.NominalValue` could be `$`, but a *new* set
        // with no value cannot be written through this plan, so the value is
        // counted as not written rather than silently invented.
        BimValue::Null => {
            counts.properties += 1;
            None
        }
        BimValue::Bool(value) => Some(PropertyValue::Boolean(*value)),
        BimValue::Integer(value) => Some(PropertyValue::Integer(*value)),
        // `BimValue::Text` carries no `ifc_type` (D2 keeps that on `Real`
        // only), so there is no measure token to honour: the declared value is
        // a string and is written as an `IfcLabel`.
        BimValue::Text(value) => Some(PropertyValue::Label(value.clone())),
        BimValue::Real {
            value,
            ifc_type,
            unit,
        } => real_value(*value, ifc_type.as_deref(), unit.as_deref(), counts),
    }
}

/// A real value in the project's length unit (metres) or SI base.
pub(crate) fn real_value(
    value: f64,
    ifc_type: Option<&str>,
    unit: Option<&str>,
    counts: &mut Counts,
) -> Option<PropertyValue> {
    if !value.is_finite() {
        counts.properties += 1;
        return None;
    }
    let mut value = value;
    if let Some(symbol) = unit {
        if let Some(factor) = length_factor(symbol) {
            value *= factor;
            match ifc_type.and_then(numeric_measure) {
                Some(measure) if is_length_measure(measure) => {
                    if let Some(converted) = measure_value(measure, value) {
                        return Some(converted);
                    }
                    counts.properties += 1;
                    return Some(PropertyValue::Real(value));
                }
                // A length unit on a non-length measure is contradictory: the
                // number is converted (it is a length) and the token dropped.
                Some(_) => {
                    counts.properties += 1;
                    return Some(PropertyValue::Real(value));
                }
                None => return Some(PropertyValue::Real(value)),
            }
        }
        if !is_si_base_unit(symbol) {
            // The exact unit cannot be expressed in this skeleton's unit
            // assignment; writing the bare number would silently change it.
            counts.properties += 1;
            return None;
        }
    }
    if let Some(raw) = ifc_type {
        if let Some(measure) = numeric_measure(raw) {
            if let Some(converted) = measure_value(measure, value) {
                return Some(converted);
            }
        }
        // The measure token cannot be written (unknown, or the value violates
        // its rule): the number survives as an `IfcReal`, the label does not.
        counts.properties += 1;
    }
    Some(PropertyValue::Real(value))
}

/// Convert one element's or type's property sets.
pub(crate) fn convert_psets(
    psets: &[BimPropertySet],
    evidence: BimEvidence,
    policy: BimExportPolicy,
    counts: &mut Counts,
) -> Vec<NewPropertySet> {
    if evidence == BimEvidence::Heuristic && !policy.include_unverified {
        counts.unverified += psets.len();
        return Vec::new();
    }
    let mut converted = Vec::with_capacity(psets.len());
    let mut set_names = BTreeSet::new();
    for set in psets {
        // `ifc-lite-export` refuses two sets of one name on one object; the
        // first declared set wins and the duplicate's values are counted.
        if !set_names.insert(set.name.as_str()) {
            counts.properties += set.properties.len();
            continue;
        }
        let mut properties = Vec::with_capacity(set.properties.len());
        let mut property_names = BTreeSet::new();
        for property in &set.properties {
            if !property_names.insert(property.name.as_str()) {
                counts.properties += 1;
                continue;
            }
            if let Some(value) = converted_value(&property.value, counts) {
                properties.push(NewProperty::new(property.name.clone(), value));
            }
        }
        // An `IfcPropertySet` is `SET [1:?]`: a set whose every value was
        // skipped is not written at all (and each value is already counted).
        if !properties.is_empty() {
            converted.push(NewPropertySet::new(
                set.name.clone(),
                PsetKind::Property,
                properties,
            ));
        }
    }
    converted
}
