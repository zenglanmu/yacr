//! BIM property rows for the properties panel (ADR-0042 D4).
//!
//! Phase 1 emitted a fixed set of eight static keys with one `bim_properties`
//! summary line. Phase 3 replaces that summary with **one row per stored PSet
//! property**: the seven fixed rows stay (`bim_category`, `bim_name`,
//! `bim_level`, `bim_type`, `bim_material`, `bim_source`, `bim_evidence`), and
//! every non-null property of every set adds a dynamic row.
//!
//! Dynamic rows need a key that is stable and unique, because the
//! multi-selection intersection looks a row up by key
//! (`rows.iter().find(|row| row.key == key)`): `bim.pset.{set}.{property}` with
//! the stored (declared) indices. The source's own name travels in
//! [`PropertyRow::label`] instead of the key: it is source data, not catalog
//! copy, so it is never translated and never added to the i18n catalog.
//!
//! Bounds and honesty:
//! * at most [`MAX_DYNAMIC_BIM_ROWS`] dynamic rows per element, so one large
//!   import cannot blow up the panel (the importer already budgets PSet size);
//! * a `BimValue::Null` property emits **no** row — null is not a value;
//! * a dynamic row is writable (`editable = true`) only when the selection ref
//!   addresses the element directly (no block-instance member, no sub-element)
//!   and the value is a scalar; the write itself is the host's, this module only
//!   reports the capability. The seven static rows stay read-only.
//!
//! The key grammar is one implementation, not a format string plus a parser:
//! [`BimPropertyKey::key`] writes it and [`BimPropertyKey::parse`] reads it, and
//! [`is_bim_property_key`] is the wire admission's question. Only the canonical
//! decimal form round-trips, so a hand-written key cannot address a slot the
//! panel does not emit.
//!
//! Every static row is projected from data the source actually declared: an
//! element without a `bim` block produces **no** rows at all (never a
//! placeholder), and an absent field produces no row for that key rather than a
//! fabricated blank. Definition references are resolved to the definition's
//! display name; a reference this database cannot resolve is shown verbatim (the
//! id) instead of being hidden. Values carry no locale: counts are decimal
//! integers and the one text composition (`bim_source`) is a fixed ASCII layout.
//! `bim_evidence` stays the persisted machine token (`declared`/`heuristic`);
//! the catalog supplies the human copy (`properties.value.bim_evidence.*`).

use cad_db::{BimDefinition, BimPropertySet, BimSource, BimValue, DrawingDatabase};
use cad_domain::{EntityId, SelectionRef};

use crate::selection::{is_addressable, PropertyGroup, PropertyKind, PropertyRow};

/// How many leading characters of a source identifier the `bim_source` summary
/// keeps before abbreviating it. A GlobalId is 22 characters and a document id
/// can be longer still; the row is a readable pointer, not a copy of the id.
const SOURCE_ID_PREFIX_CHARS: usize = 8;

/// The fixed prefix of every dynamic PSet property key (`bim.pset.{set}.{prop}`).
const KEY_PREFIX: &str = "bim.pset.";

/// The hard cap on per-property rows for one element.
///
/// Beyond this the panel stops adding dynamic rows instead of becoming a table
/// dump. The bound counts emitted rows only, so a run of nulls never hides a
/// real value behind the cap.
pub const MAX_DYNAMIC_BIM_ROWS: usize = 64;

/// The stored position of one dynamic BIM property row.
///
/// The position is the pair of **declared indices** in the stored data (the set
/// in the element's `psets`, the property inside that set), not a source name:
/// the same slot of two elements compares under multi-selection, and renaming a
/// property never moves a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BimPropertyKey {
    /// Index of the property set in the element's stored PSet vector.
    pub set: usize,
    /// Index of the property inside that set.
    pub property: usize,
}

impl BimPropertyKey {
    /// `bim.pset.{set}.{property}`, canonical decimal (no leading zeros, no
    /// whitespace).
    ///
    /// This is the **only** writer of the grammar; [`Self::parse`] is the only
    /// reader, so the read side and the write admission can never drift.
    pub fn key(&self) -> String {
        format!("{KEY_PREFIX}{}.{}", self.set, self.property)
    }

    /// Strict parse: the prefix plus **exactly two** canonical decimal segments.
    ///
    /// `None` for anything the panel does not emit, including a leading zero
    /// (`bim.pset.00.0`), a missing or extra segment (`bim.pset.0`,
    /// `bim.pset.0.0.1`), a sign, whitespace, a non-digit (`bim.pset.x`) and an
    /// index that does not fit `usize`.
    pub fn parse(key: &str) -> Option<BimPropertyKey> {
        let segments = key.strip_prefix(KEY_PREFIX)?;
        let (set, property) = segments.split_once('.')?;
        // More than two segments is not a slot: `split_once` already consumed the
        // first separator, so a remaining one means the shape is wrong.
        if property.contains('.') {
            return None;
        }
        Some(BimPropertyKey {
            set: canonical_index(set)?,
            property: canonical_index(property)?,
        })
    }
}

/// Whether `key` is a canonical dynamic BIM property key
/// ([`BimPropertyKey::parse`] accepts it).
///
/// The wire admission asks this before queueing a property write: a matching key
/// is dispatched by `cad-app` to the BIM write path, never to
/// [`crate::property_value`] (which must not learn about these keys).
pub fn is_bim_property_key(key: &str) -> bool {
    BimPropertyKey::parse(key).is_some()
}

/// One canonical decimal `usize` segment: digits only, no leading zero (except
/// `0` itself), and it must fit the platform's `usize`.
fn canonical_index(segment: &str) -> Option<usize> {
    if segment.is_empty() || !segment.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if segment.len() > 1 && segment.starts_with('0') {
        return None;
    }
    segment.parse().ok()
}

/// The BIM rows for one element, in panel order.
///
/// Returns an empty vector for an element with no `bim` block. The caller
/// appends the rows after the identity/geometry/display rows, so every element
/// keeps its existing row set and BIM data only ever *adds* to it.
///
/// `reference` is the selected object, not just the entity: a dynamic row's
/// writability is a property of the **address** (a block-instance member or a
/// sub-element reaches the element through an indirection the write path does
/// not accept), so the read side must see the same ref the writer will.
pub(crate) fn bim_property_rows(
    database: &DrawingDatabase,
    reference: &SelectionRef,
    entity: EntityId,
) -> Vec<PropertyRow> {
    let Some(bim) = database.bim_element(entity) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    if let Some(category) = non_empty(bim.category.as_deref()) {
        rows.push(PropertyRow::new("bim_category", category));
    }
    if let Some(name) = non_empty(bim.name.as_deref()) {
        rows.push(PropertyRow::new("bim_name", name));
    }
    if let Some(reference) = non_empty(bim.level.as_deref()) {
        rows.push(PropertyRow::new(
            "bim_level",
            level_name(database, reference),
        ));
    }
    if let Some(reference) = non_empty(bim.element_type.as_deref()) {
        rows.push(PropertyRow::new("bim_type", type_name(database, reference)));
    }
    if let Some(reference) = non_empty(bim.material.as_deref()) {
        rows.push(PropertyRow::new(
            "bim_material",
            material_name(database, reference),
        ));
    }
    // The source row is always present for a BIM element: a valid element has a
    // non-empty connector, which is the minimum provenance the panel can show.
    rows.push(PropertyRow::new("bim_source", source_summary(&bim.source)));
    rows.extend(pset_property_rows(&bim.psets, reference));
    rows.push(PropertyRow::new("bim_evidence", bim.evidence.as_str()));
    rows
}

/// One row per stored, non-null PSet property, capped at
/// [`MAX_DYNAMIC_BIM_ROWS`].
///
/// The key keeps the stored position ([`BimPropertyKey`]), so the same slot of
/// two elements compares under multi-selection. The label is the source name
/// composed as `{pset}.{property}`: the group is shared (`Bim`) and two sets can
/// declare the same property name, so the set name keeps the row recognizable
/// without repeating it in the key.
fn pset_property_rows(psets: &[BimPropertySet], reference: &SelectionRef) -> Vec<PropertyRow> {
    let mut rows = Vec::new();
    'sets: for (set_index, set) in psets.iter().enumerate() {
        for (property_index, property) in set.properties.iter().enumerate() {
            if rows.len() >= MAX_DYNAMIC_BIM_ROWS {
                break 'sets;
            }
            let Some((value, kind)) = dynamic_value(&property.value) else {
                // A null property has no value, so it gets no row.
                continue;
            };
            let key = BimPropertyKey {
                set: set_index,
                property: property_index,
            }
            .key();
            let mut row = PropertyRow::in_group(key, value, PropertyGroup::Bim);
            row.label = Some(format!("{}.{}", set.name, property.name));
            row.kind = kind;
            // Writability is the conjunction the double check needs: the target is
            // directly addressable **and** the value is a scalar. A `Null` never
            // reaches here (no row is emitted for it), so the match names exactly
            // the four stored scalars.
            row.editable = is_addressable(reference)
                && matches!(
                    property.value,
                    BimValue::Text(_)
                        | BimValue::Real { .. }
                        | BimValue::Integer(_)
                        | BimValue::Bool(_)
                );
            rows.push(row);
        }
    }
    rows
}

/// The display string and editing kind for one stored property value.
///
/// `None` for [`BimValue::Null`]: an explicit null is the absence of a value, so
/// the panel omits the row rather than inventing an empty one. A real number is
/// rendered with the shortest round-trip form and, when the source declared one,
/// its unit token appended verbatim (`3.5 m`) — no locale formatting.
///
/// A **unit-bearing** real is `Text`, not `Number`: its value is `"3.5 m"`, and
/// a numeric editor would show that as an empty field. Only a unit-less real and
/// an integer are `Number`; they are the strings a number editor can round-trip.
fn dynamic_value(value: &BimValue) -> Option<(String, PropertyKind)> {
    match value {
        BimValue::Text(text) => Some((text.clone(), PropertyKind::Text)),
        BimValue::Real { value, unit, .. } => {
            let mut text = format!("{value}");
            match unit.as_deref().filter(|unit| !unit.trim().is_empty()) {
                Some(unit) => {
                    text.push(' ');
                    text.push_str(unit);
                    Some((text, PropertyKind::Text))
                }
                None => Some((text, PropertyKind::Number)),
            }
        }
        BimValue::Integer(value) => Some((value.to_string(), PropertyKind::Number)),
        BimValue::Bool(value) => Some((value.to_string(), PropertyKind::Bool)),
        BimValue::Null => None,
    }
}

/// A non-blank optional string, or `None` (no row is emitted for it).
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.trim().is_empty())
}

/// The level definition's name, or the reference itself when it does not
/// resolve (a dangling reference is shown, never hidden).
fn level_name(database: &DrawingDatabase, reference: &str) -> String {
    match database.bim_definition(reference) {
        Some(BimDefinition::Level { name, .. }) => name.clone(),
        _ => reference.to_string(),
    }
}

/// The type definition's name, or the reference itself when it does not resolve.
fn type_name(database: &DrawingDatabase, reference: &str) -> String {
    match database.bim_definition(reference) {
        Some(BimDefinition::Type { name, .. }) => name.clone(),
        _ => reference.to_string(),
    }
}

/// The material definition's name, or the reference itself when it does not
/// resolve.
fn material_name(database: &DrawingDatabase, reference: &str) -> String {
    match database.bim_definition(reference) {
        Some(BimDefinition::Material { name, .. }) => name.clone(),
        _ => reference.to_string(),
    }
}

/// One readable provenance line: `ifc / doc 2O2Fr$t4... / id 2O2Fr$t4...`.
///
/// The document id and the element id are abbreviated (they are identifiers, not
/// data); the source-native id is shown only when no element id exists, so the
/// line stays short and never repeats the same string twice for a connector that
/// uses one identifier for both.
fn source_summary(source: &BimSource) -> String {
    let connector = non_empty(Some(source.connector.as_str())).unwrap_or("unknown");
    let mut parts = vec![connector.to_string()];
    if let Some(document_id) = non_empty(source.document_id.as_deref()) {
        parts.push(format!("doc {}", abbreviate(document_id)));
    }
    match non_empty(source.id.as_deref()) {
        Some(id) => parts.push(format!("id {}", abbreviate(id))),
        None => {
            if let Some(native_id) = non_empty(source.native_id.as_deref()) {
                parts.push(format!("native {}", native_id));
            }
        }
    }
    parts.join(" / ")
}

/// The first [`SOURCE_ID_PREFIX_CHARS`] characters plus `...` when longer.
///
/// Character-based (never byte-based) so a non-ASCII identifier cannot be split
/// mid-character, and the same input always yields the same output.
fn abbreviate(value: &str) -> String {
    if value.chars().count() <= SOURCE_ID_PREFIX_CHARS {
        return value.to_string();
    }
    let prefix: String = value.chars().take(SOURCE_ID_PREFIX_CHARS).collect();
    format!("{prefix}...")
}

#[cfg(test)]
#[path = "tests/selection_bim_tests.rs"]
mod tests;
