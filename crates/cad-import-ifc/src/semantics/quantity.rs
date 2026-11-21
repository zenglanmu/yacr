//! IFC element quantities (`IfcElementQuantity`) as stored property sets
//! (ADR-0042 Phase 3).
//!
//! An `IfcElementQuantity` already is a named set of named numbers
//! (`Qto_WallBaseQuantities` → `Length`, `Area`, `Volume`, …), which is exactly
//! the shape [`cad_db::BimPropertySet`] holds. It is stored under its **own**
//! source name — no `Qto_` rewriting, no prefix, no separate aspect layer — with
//! one [`cad_db::BimProperty`] per readable quantity.
//!
//! Nothing is converted and nothing is guessed:
//!
//! * the number stays the one the file declares (a 3000 mm length is stored as
//!   `3000` with `unit: "mm"`, the same rule `property.rs` uses for a measure
//!   value — the unit symbol carries the scale);
//! * `ifc_type` is the source quantity entity (`"IfcQuantityLength"`, …), so a
//!   consumer can tell a length from an area from a volume;
//! * `unit` comes from the quantity's own `Unit` override first and otherwise
//!   from the project's declared unit for the value's measure type through the
//!   same `ProjectUnits` resolver the property values use (which falls back to
//!   the dimension's IFC-canonical SI symbol when the assignment declares no
//!   unit for that dimension). A unit entity that does not resolve leaves
//!   `unit: None`; it is never replaced by another unit, and never by an
//!   assumed "metres".
//!
//! `IfcQuantityCount` is dimensionless and is stored as
//! [`cad_db::BimValue::Integer`]. Every other member of the
//! `IfcPhysicalQuantity` family (`IfcPhysicalComplexQuantity` aggregates other
//! quantities) is reported as a loss, never flattened into a number or a string.

use cad_db::{BimProperty, BimPropertySet, BimValue, BIM_MAX_PROPERTIES_PER_SET};
use ifc_lite_core::{
    resolve_unit_by_ref, slot, DecodedEntity, EntityDecoder, IfcType, ProjectUnits,
};

use super::{bounded_text, Stats};

/// Read one `IfcElementQuantity` definition into a property set.
///
/// `None` means the set itself could not be stored (no usable name, or no
/// readable quantity in it); the caller counts that once under
/// `import.ifc.bim.quantity_set_not_stored`. A set that stores a subset keeps
/// the subset and each dropped entry is counted by [`read_quantity`].
pub(super) fn collect_quantity_set(
    definition: &DecodedEntity,
    decoder: &mut EntityDecoder<'_>,
    units: &ProjectUnits,
    stats: &mut Stats,
) -> Option<BimPropertySet> {
    let name = bounded_text(
        definition.get_string(slot::IfcElementQuantity::NAME.index()),
        stats,
    )?;
    let refs = definition
        .get_refs(slot::IfcElementQuantity::QUANTITIES.index())
        .unwrap_or_default();
    let mut properties = Vec::new();
    for quantity_id in refs {
        if properties.len() >= BIM_MAX_PROPERTIES_PER_SET {
            // The surplus quantity is dropped whole, like a surplus property.
            stats.budget_exceeded += 1;
            continue;
        }
        if let Some(property) = read_quantity(quantity_id, decoder, units, stats) {
            properties.push(property);
        }
    }
    if properties.is_empty() {
        // A named set with nothing storable is not written as an empty set:
        // the document must not claim a quantity the import could not read.
        return None;
    }
    Some(BimPropertySet { name, properties })
}

/// Read one `IfcPhysicalSimpleQuantity` as a stored value, or count why not.
///
/// Every failure increments `quantity_not_stored`: the entry is left out rather
/// than turned into a number the file did not declare.
fn read_quantity(
    quantity_id: u32,
    decoder: &mut EntityDecoder<'_>,
    units: &ProjectUnits,
    stats: &mut Stats,
) -> Option<BimProperty> {
    let Ok(quantity) = decoder.decode_by_id(quantity_id) else {
        stats.quantity_not_stored += 1;
        return None;
    };
    let Some(shape) = quantity_shape(&quantity.ifc_type) else {
        // `IfcPhysicalComplexQuantity` (or a future subtype) has no single
        // value; it is reported, not flattened.
        stats.quantity_not_stored += 1;
        return None;
    };
    let Some(name) = bounded_text(
        quantity.get_string(slot::IfcPhysicalSimpleQuantity::NAME.index()),
        stats,
    ) else {
        stats.quantity_not_stored += 1;
        return None;
    };
    let Some(raw) = quantity.get(shape.value_slot) else {
        stats.quantity_not_stored += 1;
        return None;
    };
    let value = match shape.measure {
        // A count is an integer by definition; a fractional one is invalid in
        // the source and is reported instead of truncated.
        None => {
            let Some(count) = raw.as_int() else {
                stats.quantity_not_stored += 1;
                return None;
            };
            BimValue::Integer(count)
        }
        Some(measure) => {
            let Some(value) = raw.as_float() else {
                stats.quantity_not_stored += 1;
                return None;
            };
            if !value.is_finite() {
                stats.quantity_not_stored += 1;
                return None;
            }
            BimValue::Real {
                value,
                ifc_type: Some(quantity.ifc_type.name().to_string()),
                unit: quantity_unit(&quantity, decoder, units, measure),
            }
        }
    };
    Some(BimProperty { name, value })
}

/// The value slot and measure type of one simple quantity entity.
///
/// `None` for anything that is not an `IfcPhysicalSimpleQuantity` subtype: this
/// reader has no aggregate quantity shape and will not invent one.
fn quantity_shape(ifc_type: &IfcType) -> Option<QuantityShape> {
    let (measure, value_slot) = match ifc_type {
        IfcType::IfcQuantityLength => (
            Some("IfcLengthMeasure"),
            slot::IfcQuantityLength::LENGTH_VALUE.index(),
        ),
        IfcType::IfcQuantityArea => (
            Some("IfcAreaMeasure"),
            slot::IfcQuantityArea::AREA_VALUE.index(),
        ),
        IfcType::IfcQuantityVolume => (
            Some("IfcVolumeMeasure"),
            slot::IfcQuantityVolume::VOLUME_VALUE.index(),
        ),
        IfcType::IfcQuantityWeight => (
            Some("IfcMassMeasure"),
            slot::IfcQuantityWeight::WEIGHT_VALUE.index(),
        ),
        IfcType::IfcQuantityTime => (
            Some("IfcTimeMeasure"),
            slot::IfcQuantityTime::TIME_VALUE.index(),
        ),
        // A count has no dimension; `unit` stays unset.
        IfcType::IfcQuantityCount => (None, slot::IfcQuantityCount::COUNT_VALUE.index()),
        _ => return None,
    };
    Some(QuantityShape {
        measure,
        value_slot,
    })
}

struct QuantityShape {
    /// The IFC measure value type the number is expressed in (`None` for a
    /// dimensionless count), used to find the project's declared unit.
    measure: Option<&'static str>,
    /// The `IfcPhysicalSimpleQuantity` subtype's value attribute.
    value_slot: usize,
}

/// The unit a quantity number is expressed in, without converting it.
///
/// The quantity's own `Unit` override wins; otherwise the project-declared unit
/// for its measure type is used through the shared `ProjectUnits` resolver (the
/// same path a property value takes). An override whose unit entity does not
/// resolve is `None` — falling back to the project default would silently
/// relabel the number, and an assumed "m" would be worse.
fn quantity_unit(
    quantity: &DecodedEntity,
    decoder: &mut EntityDecoder<'_>,
    units: &ProjectUnits,
    measure: &str,
) -> Option<String> {
    if let Some(unit_ref) = quantity.get_ref(slot::IfcPhysicalSimpleQuantity::UNIT.index()) {
        return resolve_unit_by_ref(decoder, unit_ref).map(|(_, unit, _)| unit.symbol);
    }
    units.unit_for_measure(measure).map(|unit| unit.symbol)
}
