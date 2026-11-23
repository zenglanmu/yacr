//! DGN tags / Item Types → BIM property sets (ADR-0042 D9).
//!
//! `cadkit-dgn` 0.5.4 already decodes DGN tags and Item Types into
//! [`cadkit_core::Entity::attributes`]: every entry carries a tag name, a typed
//! [`cadkit_core::Value`] and the owning tag set / Item Type name in
//! `Attribute::set`. This module is the only place that turns those records into
//! [`cad_db::BimElement`] blocks; the geometry mapper inserts a row and then calls
//! [`Mapper::attach_bim`], so nothing here touches geometry, layers or linetypes.
//!
//! # Rules (file data only, ADR-0042 D9)
//!
//! * One [`BimPropertySet`] per tag set / Item Type, named after that set — or
//!   [`DGN_TAGS_SET`] when the file gave the set no name. The reader's
//!   `tag_<index>` fallback for a missing tag-set definition is **not** file
//!   data: such a tag is counted as undecodable and never becomes a property
//!   with a fabricated name.
//! * `category` comes **only** from an explicit `Category` / `IfcType` /
//!   `IFCEntity` tag (ASCII case-insensitive) whose value is a single token. A
//!   level name, a shared-cell name and the element's shape are never a
//!   category; there is no heuristic path here, so every stored value is
//!   [`BimEvidence::Declared`].
//! * Values keep their decoded type: `Text`/`Integer`/`Real`/`Bool`/`Null`.
//!   Floats are not formatted into strings; a binary or list value has no
//!   [`cad_db::BimValue`] form (ADR-0042 D2) and is dropped with a report rather
//!   than flattened.
//! * Levels, cell names, type and material references stay `None`: DGN has no
//!   storey/type/material table in this reader, and inventing one is exactly
//!   what D9 forbids.
//!
//! # Budgets
//!
//! Every `cad_db::BIM_*` ceiling is a **drop with a report**, never a truncation
//! of a stored row: an over-budget string, property set or property set count
//! removes that part and reports `import.dgn.bim.budget_exceeded`. A set that
//! survives is complete; no half-written property set is stored.
//!
//! # Evidence boundary
//!
//! Values are `Declared` because the file's author wrote them, but the *BIM
//! interpretation* as a whole is `Unverified`: no real V8i/CONNECT sample with
//! Item Types backs these contract tests, so the report always carries
//! [`crate::BIM_UNVERIFIED`] once any BIM block is stored. Real DGN BIM
//! semantics remain **NOT VERIFIED**.

use std::collections::BTreeMap;

use cad_db::{
    BimElement, BimEvidence, BimProperty, BimPropertySet, BimSource, BimValue,
    BIM_MAX_PROPERTIES_PER_SET, BIM_MAX_PSETS_PER_ELEMENT, BIM_MAX_STRING_BYTES,
};
use cad_domain::{CadResult, EntityId};
use cadkit_core::{Attribute, Entity, Value};

use crate::map::Mapper;

/// The connector id every DGN BIM source row carries.
pub(crate) const CONNECTOR: &str = "dgn";

/// Name of the property set used when a tag carries no tag-set / Item Type name.
pub(crate) const DGN_TAGS_SET: &str = "DGN_TAGS";

/// Tag names that may declare the element category (ADR-0042 D9).
///
/// Compared with `eq_ignore_ascii_case`; the order is only the lookup order,
/// the first matching tag in the element's own attribute order wins.
const CATEGORY_TAGS: [&str; 3] = ["category", "ifctype", "ifcentity"];

/// Longest accepted category token. A category is a name, not a sentence.
const MAX_CATEGORY_BYTES: usize = 128;

/// What one entity's tags produced (counted for the report, ADR-0042 D9).
#[derive(Debug, Default)]
pub(crate) struct EntityBim {
    /// The BIM block to store, `None` when the tags produced nothing storable.
    pub(crate) element: Option<BimElement>,
    /// Tag values that became properties.
    pub(crate) stored: usize,
    /// Property sets stored on the element.
    pub(crate) psets: usize,
    /// The category token, when an explicit category tag declared one.
    pub(crate) category: Option<String>,
    /// Tags the reader could not name (`tag_<n>` fallback) or whose value has no
    /// [`BimValue`] form; reported as `import.dgn.tags_not_stored`.
    pub(crate) undecodable: usize,
    /// Parts dropped by a `cad_db::BIM_*` budget; reported as
    /// `import.dgn.bim.budget_exceeded`.
    pub(crate) budget_dropped: usize,
}

/// Build one element's BIM block from its source id and decoded tags.
///
/// `source_id` is the adapter's existing entity identifier spelling
/// (`format!("{id:X}")`, the same string the row's `source_handle` carries), so
/// `bim.source.id` and the entity's own handle never disagree.
///
/// Pure: no document or mapper state is read. An entity whose tags are all
/// undecodable, over budget or absent yields no block at all.
pub(crate) fn bim_for_entity(source_id: Option<&str>, attributes: &[Attribute]) -> EntityBim {
    let mut outcome = EntityBim::default();
    if attributes.is_empty() {
        return outcome;
    }

    // Grouped by set name so every tag set / Item Type becomes one property set.
    // `BTreeMap` keeps the serialised order stable (set name order, then the
    // file's own tag order inside a set).
    let mut sets: BTreeMap<String, Vec<BimProperty>> = BTreeMap::new();
    for attribute in attributes {
        let Some(name) = decoded_tag_name(&attribute.tag) else {
            outcome.undecodable += 1;
            continue;
        };
        if name.len() > BIM_MAX_STRING_BYTES {
            outcome.budget_dropped += 1;
            continue;
        }
        let Some(value) = bim_value(&attribute.value, &mut outcome) else {
            continue;
        };
        let set = set_name(attribute.set.as_deref());
        if set.len() > BIM_MAX_STRING_BYTES {
            // The set name is the property set's identity; an over-long one is
            // dropped whole instead of being shortened (which would merge or
            // rename the author's Item Type).
            outcome.budget_dropped += 1;
            continue;
        }
        sets.entry(set).or_default().push(BimProperty {
            name: name.to_string(),
            value,
        });
        outcome.stored += 1;
    }

    // Per-set budget: a set with too many properties is dropped whole, never
    // truncated to the first N (a partial Item Type would look like the file).
    let mut psets = Vec::new();
    for (name, properties) in sets {
        if properties.is_empty() {
            // Only reachable when every tag of the set was dropped above.
            continue;
        }
        if properties.len() > BIM_MAX_PROPERTIES_PER_SET {
            outcome.stored -= properties.len();
            outcome.budget_dropped += 1;
            continue;
        }
        psets.push(BimPropertySet { name, properties });
    }
    // Per-element budget: the overflow sets are dropped (each surviving set is
    // still complete) and reported once per dropped set.
    if psets.len() > BIM_MAX_PSETS_PER_ELEMENT {
        for set in psets.drain(BIM_MAX_PSETS_PER_ELEMENT..) {
            outcome.stored -= set.properties.len();
            outcome.budget_dropped += 1;
        }
    }

    let category = dgn_bim_category(attributes);
    if psets.is_empty() && category.is_none() {
        return outcome;
    }

    let source = BimSource {
        connector: CONNECTOR.to_string(),
        // A DGN file has no document GUID in this reader; the document checksum
        // lives in `tables/sources.json` (ADR-0042 D5), not on the element.
        document_id: None,
        id: source_id.map(str::to_string),
        native_id: source_id.map(str::to_string),
    };
    outcome.psets = psets.len();
    outcome.category = category.clone();
    outcome.element = Some(BimElement {
        category,
        // DGN elements carry no neutral display name; tag values are properties
        // and a cell name is not an element name (D9).
        name: None,
        source,
        level: None,
        element_type: None,
        material: None,
        psets,
        evidence: BimEvidence::Declared,
    });
    outcome
}

/// The category token an explicit `Category`/`IfcType`/`IFCEntity` tag declares.
///
/// Case-insensitive on the tag name; the value must be a single token (non-empty,
/// no whitespace, at most [`MAX_CATEGORY_BYTES`]). The first qualifying tag in the
/// element's attribute order wins, so the result is deterministic. Anything else —
/// a level name, a cell name, a free-text description — yields `None`.
pub(crate) fn dgn_bim_category(tags: &[Attribute]) -> Option<String> {
    tags.iter().find_map(|attribute| {
        let name = attribute.tag.trim();
        if !CATEGORY_TAGS
            .iter()
            .any(|known| name.eq_ignore_ascii_case(known))
        {
            return None;
        }
        let Value::Text(text) = &attribute.value else {
            return None;
        };
        let token = text.trim();
        is_category_token(token).then(|| token.to_string())
    })
}

/// A category is a name, not prose: one non-empty token with no whitespace.
fn is_category_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_CATEGORY_BYTES
        && !token.contains(char::is_whitespace)
        && !token.contains(char::is_control)
}

/// The tag name, unless the reader fabricated it.
///
/// `cadkit-dgn` names a tag `tag_<index>` when the tag-set definition is missing
/// or unreadable. That string is decoder output, not a value the file declared,
/// so the tag is counted as undecodable instead of inventing a property name.
///
/// A file that literally names a tag `tag_<digits>` is indistinguishable from the
/// reader's fallback; refusing the name (rather than guessing) is the honest
/// choice, and the loss is reported.
fn decoded_tag_name(tag: &str) -> Option<&str> {
    let tag = tag.trim();
    if tag.is_empty() || is_reader_fallback_tag(tag) {
        return None;
    }
    Some(tag)
}

fn is_reader_fallback_tag(tag: &str) -> bool {
    tag.strip_prefix("tag_")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The property set a tag belongs to: its tag set / Item Type name, or
/// [`DGN_TAGS_SET`] when the file named none.
fn set_name(set: Option<&str>) -> String {
    match set.map(str::trim) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => DGN_TAGS_SET.to_string(),
    }
}

/// Map one decoded tag value onto [`BimValue`], or account for why it cannot be.
fn bim_value(value: &Value, outcome: &mut EntityBim) -> Option<BimValue> {
    match value {
        Value::Text(text) => {
            if text.len() > BIM_MAX_STRING_BYTES {
                outcome.budget_dropped += 1;
                return None;
            }
            Some(BimValue::Text(text.clone()))
        }
        Value::Int(number) => Some(BimValue::Integer(*number)),
        Value::Float(number) if number.is_finite() => Some(BimValue::Real {
            value: *number,
            // DGN tags store no IFC measure type or unit.
            ifc_type: None,
            unit: None,
        }),
        Value::Float(_) => {
            // `cad_db` refuses a non-finite real; dropping it is the only
            // non-lossy option (there is no "NaN as text" form).
            outcome.undecodable += 1;
            None
        }
        Value::Bool(flag) => Some(BimValue::Bool(*flag)),
        Value::Null => Some(BimValue::Null),
        // `List`/`Bytes` (and any future `Value` variant) have no `BimValue`
        // form: ADR-0042 D2 forbids flattening them into text.
        _ => {
            outcome.undecodable += 1;
            None
        }
    }
}

impl Mapper<'_> {
    /// Store one inserted entity's tags as its BIM block (ADR-0042 D9).
    ///
    /// Called after the entity row exists, mirroring
    /// `set_entity_render_attributes`. A `cad_db` refusal is reported, never
    /// silent, and never fails the import: the geometry row is still valid, so
    /// only the BIM block is missing and the report says so.
    pub(super) fn attach_bim(&mut self, entity: &Entity, id: EntityId) -> CadResult<()> {
        if entity.attributes.is_empty() {
            return Ok(());
        }
        // Same spelling as the row's `source_handle`, so the two never disagree.
        let handle = entity.id.map(|id| format!("{id:X}"));
        let outcome = bim_for_entity(handle.as_deref(), &entity.attributes);
        self.bim_undecodable_tags += outcome.undecodable;
        self.bim_budget_dropped += outcome.budget_dropped;
        let Some(element) = outcome.element else {
            return Ok(());
        };
        let category = outcome.category.is_some();
        match self.builder.set_entity_bim(id, element) {
            Ok(()) => {
                self.bim_stored_tags += outcome.stored;
                self.bim_psets += outcome.psets;
                self.bim_elements += 1;
                if category {
                    self.bim_categories += 1;
                } else {
                    self.bim_no_category += 1;
                }
            }
            Err(error) => {
                // The block itself was refused, so none of its values reached the
                // database; the tags are counted as not stored, not as success.
                // `set_entity_bim` shape-checks first, and every shape rule is
                // pre-filtered above, so a refusal here is a budget one; `error`
                // still carries cad-db's own `ymodel.bim.*` marker.
                self.bim_undecodable_tags += outcome.stored;
                self.bim_budget_dropped += 1;
                self.note(
                    crate::BIM_BUDGET_EXCEEDED,
                    format!(
                        "the BIM block of entity {} was refused by cad-db and is not stored: {error}",
                        id.0
                    ),
                    None,
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
