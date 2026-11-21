//! BIM property rows for the properties panel (ADR-0042 D4).
//!
//! Phase 1 emits a **fixed** set of eight static keys — `bim_category`,
//! `bim_name`, `bim_level`, `bim_type`, `bim_material`, `bim_source`,
//! `bim_properties`, `bim_evidence` — and nothing else. This is a hard contract,
//! not a style choice: [`PropertyRow::key`](crate::selection::PropertyRow::key)
//! is `&'static str`, and the multi-selection intersection looks a row up by
//! `rows.iter().find(|row| row.key == key)`, so a dynamic or duplicated key
//! would silently drop rows. Per-property rows, dynamic property names and
//! editing are Phase 3 and require a wire change first (ADR-0042 D4).
//!
//! Every row is projected from data the source actually declared: an element
//! without a `bim` block produces **no** rows at all (never a placeholder), and
//! an absent field produces no row for that key rather than a fabricated blank.
//! Definition references are resolved to the definition's display name; a
//! reference this database cannot resolve is shown verbatim (the id) instead of
//! being hidden. Values carry no locale: counts are decimal integers and the one
//! text composition (`bim_properties`, `bim_source`) is a fixed ASCII layout.
//! `bim_evidence` stays the persisted machine token (`declared`/`heuristic`);
//! the catalog supplies the human copy (`properties.value.bim_evidence.*`).

use cad_db::{BimDefinition, BimPropertySet, BimSource, DrawingDatabase};
use cad_domain::EntityId;

use crate::selection::PropertyRow;

/// How many leading characters of a source identifier the `bim_source` summary
/// keeps before abbreviating it. A GlobalId is 22 characters and a document id
/// can be longer still; the row is a readable pointer, not a copy of the id.
const SOURCE_ID_PREFIX_CHARS: usize = 8;

/// The BIM rows for one element, in panel order.
///
/// Returns an empty vector for an element with no `bim` block. The caller
/// appends the rows after the identity/geometry/display rows, so every element
/// keeps its existing row set and BIM data only ever *adds* to it.
pub(crate) fn bim_property_rows(database: &DrawingDatabase, entity: EntityId) -> Vec<PropertyRow> {
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
    if !bim.psets.is_empty() {
        rows.push(PropertyRow::new(
            "bim_properties",
            psets_summary(&bim.psets),
        ));
    }
    rows.push(PropertyRow::new("bim_evidence", bim.evidence.as_str()));
    rows
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

/// One-line property-set summary: `Pset_WallCommon (3), Pset_Revit_Type (5)`.
///
/// The count is the number of properties actually stored in that set; order is
/// the stored (declared) order, so the line is deterministic for one document.
fn psets_summary(psets: &[BimPropertySet]) -> String {
    psets
        .iter()
        .map(|set| format!("{} ({})", set.name, set.properties.len()))
        .collect::<Vec<String>>()
        .join(", ")
}

#[cfg(test)]
#[path = "tests/selection_bim_tests.rs"]
mod tests;
