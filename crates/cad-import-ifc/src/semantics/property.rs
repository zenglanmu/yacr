//! IFC property sets and property values (ADR-0042 D2/D7).
//!
//! Only `IfcPropertySet` + `IfcPropertySingleValue` have a [`cad_db::BimValue`]
//! form. Complex, enumerated, bounded, table and quantity shapes are counted
//! under stable diagnostics and left out — never flattened into a string.

use std::collections::BTreeSet;

use cad_db::{
    BimProperty, BimPropertySet, BimValue, BIM_MAX_PROPERTIES_PER_SET, BIM_MAX_PSETS_PER_ELEMENT,
};
use ifc_lite_core::{
    resolve_unit_by_ref, slot, AttributeValue, DecodedEntity, EntityDecoder, IfcType, ProjectUnits,
};

use super::{bounded_string, bounded_text, Relations, Stats};

pub(super) fn collect_element_psets(
    product_id: u32,
    relations: &Relations,
    decoder: &mut EntityDecoder<'_>,
    units: &ProjectUnits,
    stats: &mut Stats,
) -> Vec<BimPropertySet> {
    let refs = relations
        .properties
        .get(&product_id)
        .cloned()
        .unwrap_or_default();
    collect_psets(refs, decoder, units, stats)
}

pub(super) fn collect_psets(
    refs: Vec<u32>,
    decoder: &mut EntityDecoder<'_>,
    units: &ProjectUnits,
    stats: &mut Stats,
) -> Vec<BimPropertySet> {
    let mut seen = BTreeSet::new();
    let mut sets = Vec::new();
    for definition_ref in refs {
        // One definition related twice (type + occurrence relation) is one set.
        if !seen.insert(definition_ref) {
            continue;
        }
        if sets.len() >= BIM_MAX_PSETS_PER_ELEMENT {
            // The set is dropped whole; its properties never become rows.
            stats.budget_exceeded += 1;
            continue;
        }
        let Ok(definition) = decoder.decode_by_id(definition_ref) else {
            stats.property_set_unsupported += 1;
            continue;
        };
        if definition.ifc_type == IfcType::IfcElementQuantity {
            stats.quantity_set_not_stored += 1;
            continue;
        }
        if definition.ifc_type != IfcType::IfcPropertySet {
            stats.property_set_unsupported += 1;
            continue;
        }
        let Some(name) = bounded_text(
            definition.get_string(slot::IfcPropertySet::NAME.index()),
            stats,
        ) else {
            stats.property_set_unsupported += 1;
            continue;
        };
        let properties = collect_properties(&definition, decoder, units, stats);
        sets.push(BimPropertySet { name, properties });
    }
    sets
}

fn collect_properties(
    set: &DecodedEntity,
    decoder: &mut EntityDecoder<'_>,
    units: &ProjectUnits,
    stats: &mut Stats,
) -> Vec<BimProperty> {
    let Some(refs) = set.get_refs(slot::IfcPropertySet::HAS_PROPERTIES.index()) else {
        return Vec::new();
    };
    let mut properties = Vec::new();
    for property_id in refs {
        if properties.len() >= BIM_MAX_PROPERTIES_PER_SET {
            stats.budget_exceeded += 1;
            continue;
        }
        let Some(property) = read_single_value(property_id, decoder, units, stats) else {
            continue;
        };
        properties.push(property);
    }
    properties
}

/// Read one `IfcProperty` as a stored value, or count why it is not one.
///
/// Every other property subtype (`IfcComplexProperty`,
/// `IfcPropertyEnumeratedValue`, `IfcPropertyBoundedValue`,
/// `IfcPropertyTableValue`, …) has no `BimValue` shape. It is counted as
/// `property_value_unsupported` and skipped — never turned into a string.
fn read_single_value(
    property_id: u32,
    decoder: &mut EntityDecoder<'_>,
    units: &ProjectUnits,
    stats: &mut Stats,
) -> Option<BimProperty> {
    let Ok(property) = decoder.decode_by_id(property_id) else {
        stats.property_value_unsupported += 1;
        return None;
    };
    if property.ifc_type != IfcType::IfcPropertySingleValue {
        stats.property_value_unsupported += 1;
        return None;
    }
    let Some(name) = bounded_text(
        property.get_string(slot::IfcPropertySingleValue::NAME.index()),
        stats,
    ) else {
        stats.property_value_unsupported += 1;
        return None;
    };
    let value = property_value(&property, decoder, units, stats)?;
    Some(BimProperty { name, value })
}

fn property_value(
    property: &DecodedEntity,
    decoder: &mut EntityDecoder<'_>,
    units: &ProjectUnits,
    stats: &mut Stats,
) -> Option<BimValue> {
    let Some(nominal) = property.get(slot::IfcPropertySingleValue::NOMINAL_VALUE.index()) else {
        // The property exists and the file left its value unset; an explicit null
        // is the value the source declared.
        return Some(BimValue::Null);
    };
    let (value, ifc_type) = match nominal {
        AttributeValue::Typed { name, value } => (value.as_ref(), Some(name.to_string())),
        other => (other, None),
    };
    match value {
        AttributeValue::String(text) => bounded_string(text, stats).map(BimValue::Text),
        AttributeValue::Integer(integer) => Some(BimValue::Integer(*integer)),
        AttributeValue::Float(real) => {
            if !real.is_finite() {
                stats.property_value_unsupported += 1;
                return None;
            }
            let unit = value_unit(property, decoder, units, ifc_type.as_deref());
            Some(BimValue::Real {
                value: *real,
                ifc_type,
                unit,
            })
        }
        AttributeValue::Enum(token) if token.eq_ignore_ascii_case("T") => {
            Some(BimValue::Bool(true))
        }
        AttributeValue::Enum(token) if token.eq_ignore_ascii_case("F") => {
            Some(BimValue::Bool(false))
        }
        // A logical `.U.` is "unknown", not false; an ordinary enumeration has no
        // BIM value form.
        AttributeValue::Enum(_) => {
            stats.property_value_unsupported += 1;
            None
        }
        // `*` (derived) and `$` (unset) both mean the file holds no value.
        AttributeValue::Null | AttributeValue::Derived => Some(BimValue::Null),
        AttributeValue::Binary(_) | AttributeValue::List(_) | AttributeValue::EntityRef(_) => {
            stats.property_value_unsupported += 1;
            None
        }
        // The `Typed` unwrap above already removed this arm's reachability; the
        // match stays exhaustive without a wildcard so a new variant is a compile
        // error rather than a silent value.
        AttributeValue::Typed { .. } => {
            stats.property_value_unsupported += 1;
            None
        }
    }
}

/// The unit a numeric property value is expressed in, without converting it.
///
/// An explicit `IfcPropertySingleValue.Unit` wins; otherwise the measure type's
/// project-declared unit is used (or its SI default). `None` means the value is
/// dimensionless or its measure is not a unit-bearing measure.
fn value_unit(
    property: &DecodedEntity,
    decoder: &mut EntityDecoder<'_>,
    units: &ProjectUnits,
    ifc_type: Option<&str>,
) -> Option<String> {
    if let Some(unit_ref) = property.get_ref(slot::IfcPropertySingleValue::UNIT.index()) {
        return resolve_unit_by_ref(decoder, unit_ref).map(|(_, unit, _)| unit.symbol);
    }
    units.unit_for_measure(ifc_type?).map(|unit| unit.symbol)
}
