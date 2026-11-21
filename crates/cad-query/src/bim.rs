//! BIM indexing, filtering and statistics over a [`DrawingDatabase`]
//! (ADR-0042 D11).
//!
//! The projection is deliberately read-only and derived: the database stays the
//! only authority, and nothing here writes back. [`BimIndex`] groups the BIM
//! rows by logical component ([`BimKey`], ADR-0042 D5) so one source element
//! split into several geometry rows counts once in
//! [`BimSummary`], while [`BimRow::entity_ids`] still lists every geometry row.
//!
//! # Lifecycle
//!
//! An index is built lazily per `(DatabaseId, Revision)` and cached on
//! [`QueryService`](crate::QueryService) — the same identity the existing
//! `last` binding uses. A document switch is still [`CadError::StaleResult`]
//! ([`QueryService::bim_query`] checks freshness before touching the index); a
//! revision change rebuilds the index, because a cached index from another
//! revision is never reused.
//!
//! # No silent narrowing
//!
//! A filter with more than [`BIM_MAX_FILTER_TERMS`] terms is refused with an
//! explicit [`CadError::InvalidInput`], never truncated. Unknown categories,
//! definition ids or property names are not errors: they simply select nothing.
//! A property filter only ever sees elements that actually declare that
//! `(pset, property)` pair, so `Ne` cannot be satisfied by a missing property.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use cad_db::{
    BimDefinition, BimElement, BimEvidence, BimKey, BimPropertySet, BimSource, BimValue,
    DrawingDatabase,
};
use cad_domain::{CadError, CadResult, DatabaseId, EntityId, Revision};

use crate::{QueryPage, QueryRequest, QueryService};

/// Most terms one [`BimFilter`] may carry (ADR-0042 D11).
///
/// Terms are the entries of `categories`/`levels`/`types`/`materials` plus one
/// for `property`. `evidence` is a single predicate, not a term.
pub const BIM_MAX_FILTER_TERMS: usize = 64;

/// Stable marker of a filter that exceeds [`BIM_MAX_FILTER_TERMS`].
pub const BIM_FILTER_TOO_MANY_TERMS: &str = "bim.filter.too_many_terms";

static EMPTY_ENTITIES: BTreeSet<EntityId> = BTreeSet::new();

/// ASCII case folding used for category and definition-name matching.
///
/// Only ASCII is folded: this is a comparison convenience, not Unicode case
/// mapping, and it never rewrites the stored spelling.
fn fold(value: &str) -> String {
    value.to_ascii_lowercase()
}

// ------------------------------------------------------------- scalar values ---

/// An `f64` with a total order, so it can key a `BTreeMap`.
///
/// BIM reals are validated finite at import (`cad_db`), so the interesting
/// cases are `-0.0` and equality. Ordering is [`f64::total_cmp`]; equality is
/// that ordering's equality, and hashing normalises `-0.0` to `0.0`.
#[derive(Debug, Clone, Copy)]
pub struct BimReal(f64);

impl BimReal {
    /// Wrap a real for use in a filter value or an index key.
    pub fn new(value: f64) -> BimReal {
        BimReal(value)
    }

    /// The wrapped value.
    pub fn get(self) -> f64 {
        self.0
    }
}

impl PartialEq for BimReal {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for BimReal {}

impl PartialOrd for BimReal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BimReal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Hash for BimReal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // `-0.0 == 0.0` numerically; one hash for both keeps the map honest.
        let bits = if self.0 == 0.0 {
            0.0f64.to_bits()
        } else {
            self.0.to_bits()
        };
        bits.hash(state);
    }
}

/// The sortable, hashable projection of a [`BimValue`] used by the index.
///
/// [`BimValue::Null`] has no projection (a null is "no value", not a value to
/// group by) and is therefore never indexed. Variant order is an implementation
/// detail; only determinism is promised.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BimScalar {
    Bool(bool),
    Integer(i64),
    Real(BimReal),
    Text(String),
}

impl BimScalar {
    /// A real value.
    pub fn real(value: f64) -> BimScalar {
        BimScalar::Real(BimReal::new(value))
    }

    /// A text value.
    pub fn text(value: impl Into<String>) -> BimScalar {
        BimScalar::Text(value.into())
    }

    /// Project a stored property value; `None` for [`BimValue::Null`].
    pub fn from_value(value: &BimValue) -> Option<BimScalar> {
        match value {
            BimValue::Text(text) => Some(BimScalar::Text(text.clone())),
            BimValue::Real { value, .. } => Some(BimScalar::Real(BimReal::new(*value))),
            BimValue::Integer(value) => Some(BimScalar::Integer(*value)),
            BimValue::Bool(value) => Some(BimScalar::Bool(*value)),
            BimValue::Null => None,
        }
    }

    /// The value as a number, when it is one.
    pub fn numeric(&self) -> Option<f64> {
        match self {
            BimScalar::Integer(value) => Some(*value as f64),
            BimScalar::Real(value) => Some(value.get()),
            BimScalar::Bool(_) | BimScalar::Text(_) => None,
        }
    }
}

// ------------------------------------------------------------------- filters ---

/// Which comparison a [`BimPropertyFilter`] performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BimPropertyOp {
    /// Numeric values compare numerically (`Real` and `Integer` cross-compare);
    /// `Bool` and `Text` compare exactly.
    Eq,
    /// The negation of [`Self::Eq`] **for elements that declare the property**.
    Ne,
    /// Substring match for `Text`; never true for a non-text value.
    Contains,
}

impl BimPropertyOp {
    fn matches(self, indexed: &BimScalar, wanted: &BimScalar) -> bool {
        match self {
            BimPropertyOp::Eq => scalar_eq(indexed, wanted),
            BimPropertyOp::Ne => !scalar_eq(indexed, wanted),
            BimPropertyOp::Contains => match (indexed, wanted) {
                (BimScalar::Text(haystack), BimScalar::Text(needle)) => {
                    haystack.contains(needle.as_str())
                }
                _ => false,
            },
        }
    }
}

fn scalar_eq(indexed: &BimScalar, wanted: &BimScalar) -> bool {
    match (indexed, wanted) {
        (BimScalar::Integer(left), BimScalar::Integer(right)) => left == right,
        (BimScalar::Real(left), BimScalar::Real(right)) => left.get() == right.get(),
        (BimScalar::Bool(left), BimScalar::Bool(right)) => left == right,
        (BimScalar::Text(left), BimScalar::Text(right)) => left == right,
        // A real and an integer compare as numbers (`0.2` vs `0`, `2.0` vs `2`).
        (left, right) if left.numeric().is_some() && right.numeric().is_some() => {
            left.numeric() == right.numeric()
        }
        _ => false,
    }
}

/// One property predicate: `(pset, property, op, value)`.
///
/// `pset` and `property` are exact names; `value` is compared by
/// [`BimPropertyOp`]. Only elements that declare the property are candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BimPropertyFilter {
    /// Property-set name (`Pset_WallCommon`, a DGN Item Type, …).
    pub pset: String,
    /// Property name inside that set.
    pub property: String,
    /// The comparison.
    pub op: BimPropertyOp,
    /// The value compared against each stored projection.
    pub value: BimScalar,
}

impl BimPropertyFilter {
    /// Build a predicate.
    pub fn new(
        pset: impl Into<String>,
        property: impl Into<String>,
        op: BimPropertyOp,
        value: BimScalar,
    ) -> BimPropertyFilter {
        BimPropertyFilter {
            pset: pset.into(),
            property: property.into(),
            op,
            value,
        }
    }
}

/// What to return from the BIM rows.
///
/// Facets are **ANDed**; terms inside one facet are **ORed**. Category terms and
/// definition display names are ASCII-case-insensitive; definition ids are
/// matched verbatim. `levels`/`types`/`materials` accept either the composite
/// definition id or its display name (both are tried, any hit counts).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BimFilter {
    /// Category tokens (`IfcWall`, …).
    pub categories: Vec<String>,
    /// Level definition ids or display names.
    pub levels: Vec<String>,
    /// Type definition ids or display names.
    pub types: Vec<String>,
    /// Material definition ids or display names.
    pub materials: Vec<String>,
    /// One property predicate.
    pub property: Option<BimPropertyFilter>,
    /// Restrict to elements whose BIM evidence is this one.
    pub evidence: Option<BimEvidence>,
}

impl BimFilter {
    /// A filter that selects every element with BIM data.
    pub fn new() -> BimFilter {
        BimFilter::default()
    }

    /// Whether any facet or predicate is set.
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
            && self.levels.is_empty()
            && self.types.is_empty()
            && self.materials.is_empty()
            && self.property.is_none()
            && self.evidence.is_none()
    }

    /// How many terms the term budget counts (see [`BIM_MAX_FILTER_TERMS`]).
    pub fn term_count(&self) -> usize {
        self.categories.len()
            + self.levels.len()
            + self.types.len()
            + self.materials.len()
            + usize::from(self.property.is_some())
    }

    /// Refuse a filter over [`BIM_MAX_FILTER_TERMS`]; never truncate.
    pub fn validate(&self) -> CadResult<()> {
        let terms = self.term_count();
        if terms > BIM_MAX_FILTER_TERMS {
            return Err(CadError::InvalidInput(format!(
                "{BIM_FILTER_TOO_MANY_TERMS}: {terms} terms exceeds the \
                 {BIM_MAX_FILTER_TERMS}-term budget"
            )));
        }
        Ok(())
    }
}

// --------------------------------------------------------------------- rows ---

/// One property set in a row's summary: its name and property count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BimPropertyCount {
    /// Property-set name.
    pub name: String,
    /// How many properties the set holds.
    pub count: usize,
}

/// One logical component (one [`BimKey`]) as a query row.
///
/// `entity_ids` is every geometry row that maps to this component, sorted.
/// `category`/`name`/`source`/`evidence`/`property_summary` come from the
/// representative element — the lowest [`EntityId`] of the group — so the row is
/// deterministic when segments repeat the same metadata. `*_name` fields are
/// the referenced definition's display name; a document built through
/// `DrawingDatabaseBuilder` cannot hold a dangling reference, so `None` means
/// the element simply has no such facet.
#[derive(Debug, Clone, PartialEq)]
pub struct BimRow {
    /// The component identity this row aggregates over.
    pub object: BimKey,
    /// Every geometry row of the component, ascending.
    pub entity_ids: Vec<EntityId>,
    /// The element category token, as declared.
    pub category: Option<String>,
    /// The element display name, as declared.
    pub name: Option<String>,
    /// The referenced level's display name.
    pub level_name: Option<String>,
    /// The referenced type's display name.
    pub type_name: Option<String>,
    /// The referenced material's display name.
    pub material_name: Option<String>,
    /// The representative element's source provenance.
    pub source: BimSource,
    /// The representative element's evidence.
    pub evidence: BimEvidence,
    /// The representative element's property sets, in declared order.
    pub property_summary: Vec<BimPropertyCount>,
}

/// One bucket of a [`BimSummary`].
///
/// `key` is the grouping key (folded category token, or definition id) and
/// `name` the display spelling (the lexicographically smallest declared spelling
/// for a category, the definition name otherwise).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BimFacetCount {
    /// Grouping key.
    pub key: String,
    /// Display name.
    pub name: String,
    /// Components in the bucket.
    pub count: usize,
}

/// Counts over the filtered components.
///
/// Counting goes through [`BimKey`], so a source element split into several
/// geometry rows counts once. `total` counts every matched component;
/// `by_category`/`by_level`/`by_type`/`by_material` only count components that
/// actually declare that facet. No "unknown" bucket is fabricated (ADR-0042 D1),
/// so the facet counts may sum to less than `total`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BimSummary {
    /// Matched components.
    pub total: usize,
    /// Components per category token.
    pub by_category: Vec<BimFacetCount>,
    /// Components per level definition id.
    pub by_level: Vec<BimFacetCount>,
    /// Components per type definition id.
    pub by_type: Vec<BimFacetCount>,
    /// Components per material definition id.
    pub by_material: Vec<BimFacetCount>,
}

// -------------------------------------------------------------------- index ---

/// Per-facet definition table: ids, display names and the keys referencing each.
#[derive(Debug, Clone, Default)]
struct Facet {
    keys_by_id: BTreeMap<String, BTreeSet<BimKey>>,
    names: BTreeMap<String, String>,
    ids_by_name: BTreeMap<String, BTreeSet<String>>,
}

impl Facet {
    fn insert_definition(&mut self, id: &str, name: &str) {
        self.names.insert(id.to_string(), name.to_string());
        self.ids_by_name
            .entry(fold(name))
            .or_default()
            .insert(id.to_string());
    }

    fn name(&self, id: &str) -> Option<&str> {
        self.names.get(id).map(String::as_str)
    }

    /// Resolve filter terms to definition ids. Both the id and the folded
    /// display name are tried for every term; any hit counts.
    fn resolve_ids(&self, terms: &[String]) -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        for term in terms {
            if self.names.contains_key(term) {
                ids.insert(term.clone());
            }
            if let Some(found) = self.ids_by_name.get(&fold(term)) {
                ids.extend(found.iter().cloned());
            }
        }
        ids
    }

    fn keys_of_ids(&self, ids: &BTreeSet<String>) -> BTreeSet<BimKey> {
        let mut keys = BTreeSet::new();
        for id in ids {
            if let Some(found) = self.keys_by_id.get(id) {
                keys.extend(found.iter().cloned());
            }
        }
        keys
    }
}

/// One component's representative metadata plus every geometry row.
#[derive(Debug, Clone)]
struct BimGroup {
    category: Option<String>,
    name: Option<String>,
    source: BimSource,
    level: Option<String>,
    element_type: Option<String>,
    material: Option<String>,
    evidence: BimEvidence,
    psets: Vec<BimPropertySet>,
    entities: BTreeSet<EntityId>,
}

/// The revision-bound BIM index (ADR-0042 D11).
///
/// Built by [`BimIndex::build`] over a [`DrawingDatabase`]; `QueryService`
/// caches it per `(DatabaseId, Revision)`.
#[derive(Debug)]
pub struct BimIndex {
    built_at: (DatabaseId, Revision),
    groups: BTreeMap<BimKey, BimGroup>,
    by_category: BTreeMap<String, BTreeSet<BimKey>>,
    levels: Facet,
    types: Facet,
    materials: Facet,
    by_property: BTreeMap<(String, String), BTreeMap<BimScalar, BTreeSet<BimKey>>>,
    by_evidence: BTreeMap<BimEvidence, BTreeSet<BimKey>>,
}

impl BimIndex {
    /// Index every element that carries BIM data.
    ///
    /// Categories are keyed by their ASCII-folded token; `level`/`type`/
    /// `material` are keyed by definition id; properties are keyed by
    /// `(pset, property)` and then by [`BimScalar`]. `Null` values are not
    /// indexed. Every geometry row contributes to the property/evidence sets,
    /// so a component whose segments repeat its metadata indexes once per scalar.
    pub fn build(database: &DrawingDatabase) -> BimIndex {
        let mut index = BimIndex {
            built_at: (database.id(), database.revision()),
            groups: BTreeMap::new(),
            by_category: BTreeMap::new(),
            levels: Facet::default(),
            types: Facet::default(),
            materials: Facet::default(),
            by_property: BTreeMap::new(),
            by_evidence: BTreeMap::new(),
        };

        for (entity, element) in database.bim_elements() {
            index.record(*entity, element);
        }
        for definition in database.bim_definitions() {
            match definition {
                BimDefinition::Level { id, name, .. } => index.levels.insert_definition(id, name),
                BimDefinition::Type { id, name, .. } => index.types.insert_definition(id, name),
                BimDefinition::Material { id, name, .. } => {
                    index.materials.insert_definition(id, name)
                }
            }
        }
        index
    }

    fn record(&mut self, entity: EntityId, element: &BimElement) {
        let key = element.key(entity);
        match self.groups.get_mut(&key) {
            Some(group) => {
                // The lowest entity id is the representative, so row metadata
                // never depends on insertion order.
                let replace = group.entities.first().is_none_or(|first| entity < *first);
                if replace {
                    group.category = element.category.clone();
                    group.name = element.name.clone();
                    group.source = element.source.clone();
                    group.level = element.level.clone();
                    group.element_type = element.element_type.clone();
                    group.material = element.material.clone();
                    group.evidence = element.evidence;
                    group.psets = element.psets.clone();
                }
                group.entities.insert(entity);
            }
            None => {
                self.groups.insert(
                    key.clone(),
                    BimGroup {
                        category: element.category.clone(),
                        name: element.name.clone(),
                        source: element.source.clone(),
                        level: element.level.clone(),
                        element_type: element.element_type.clone(),
                        material: element.material.clone(),
                        evidence: element.evidence,
                        psets: element.psets.clone(),
                        entities: BTreeSet::from([entity]),
                    },
                );
            }
        }

        if let Some(category) = &element.category {
            self.by_category
                .entry(fold(category))
                .or_default()
                .insert(key.clone());
        }
        if let Some(level) = &element.level {
            self.levels
                .keys_by_id
                .entry(level.clone())
                .or_default()
                .insert(key.clone());
        }
        if let Some(element_type) = &element.element_type {
            self.types
                .keys_by_id
                .entry(element_type.clone())
                .or_default()
                .insert(key.clone());
        }
        if let Some(material) = &element.material {
            self.materials
                .keys_by_id
                .entry(material.clone())
                .or_default()
                .insert(key.clone());
        }
        self.by_evidence
            .entry(element.evidence)
            .or_default()
            .insert(key.clone());

        for set in &element.psets {
            for property in &set.properties {
                let Some(scalar) = BimScalar::from_value(&property.value) else {
                    continue;
                };
                self.by_property
                    .entry((set.name.clone(), property.name.clone()))
                    .or_default()
                    .entry(scalar)
                    .or_default()
                    .insert(key.clone());
            }
        }
    }

    /// The `(DatabaseId, Revision)` the index was built from.
    pub fn built_at(&self) -> (DatabaseId, Revision) {
        self.built_at
    }

    /// Components per folded category token.
    pub fn by_category(&self) -> &BTreeMap<String, BTreeSet<BimKey>> {
        &self.by_category
    }

    /// Components per level definition id.
    pub fn by_level(&self) -> &BTreeMap<String, BTreeSet<BimKey>> {
        &self.levels.keys_by_id
    }

    /// Components per type definition id.
    pub fn by_type(&self) -> &BTreeMap<String, BTreeSet<BimKey>> {
        &self.types.keys_by_id
    }

    /// Components per material definition id.
    pub fn by_material(&self) -> &BTreeMap<String, BTreeSet<BimKey>> {
        &self.materials.keys_by_id
    }

    /// Components per `(pset, property)` and value.
    pub fn by_property(
        &self,
    ) -> &BTreeMap<(String, String), BTreeMap<BimScalar, BTreeSet<BimKey>>> {
        &self.by_property
    }

    /// Every geometry row of a component; empty for an unknown key.
    pub fn entities_of(&self, key: &BimKey) -> &BTreeSet<EntityId> {
        self.groups
            .get(key)
            .map(|group| &group.entities)
            .unwrap_or(&EMPTY_ENTITIES)
    }

    /// Every indexed component.
    pub fn keys(&self) -> impl Iterator<Item = &BimKey> {
        self.groups.keys()
    }

    /// How many components the index holds.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether the index holds no component (no BIM data in the document).
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The components selected by `filter`, de-duplicated by [`BimKey`].
    ///
    /// Over-budget filters are refused before any work is done.
    pub fn filter(&self, filter: &BimFilter) -> CadResult<BTreeSet<BimKey>> {
        filter.validate()?;
        let mut matched: Option<BTreeSet<BimKey>> = None;

        if !filter.categories.is_empty() {
            let mut selected = BTreeSet::new();
            for category in &filter.categories {
                if let Some(keys) = self.by_category.get(&fold(category)) {
                    selected.extend(keys.iter().cloned());
                }
            }
            intersect(&mut matched, selected);
        }
        apply_facet(&mut matched, &filter.levels, &self.levels);
        apply_facet(&mut matched, &filter.types, &self.types);
        apply_facet(&mut matched, &filter.materials, &self.materials);

        if let Some(property) = &filter.property {
            let mut selected = BTreeSet::new();
            let bucket = (property.pset.clone(), property.property.clone());
            if let Some(values) = self.by_property.get(&bucket) {
                for (scalar, keys) in values {
                    if property.op.matches(scalar, &property.value) {
                        selected.extend(keys.iter().cloned());
                    }
                }
            }
            intersect(&mut matched, selected);
        }

        if let Some(evidence) = filter.evidence {
            let selected = self.by_evidence.get(&evidence).cloned().unwrap_or_default();
            intersect(&mut matched, selected);
        }

        Ok(matched.unwrap_or_else(|| self.groups.keys().cloned().collect()))
    }

    /// Filtered rows, in deterministic order.
    ///
    /// Order is `(folded category, name, source id, lowest EntityId, BimKey)`;
    /// the final [`BimKey`] tiebreak makes the order total. An entity-keyed
    /// component's "source id" is its own id, so components without a source
    /// identifier still sort deterministically.
    pub fn rows(&self, filter: &BimFilter) -> CadResult<Vec<BimRow>> {
        let keys = self.filter(filter)?;
        let mut rows: Vec<BimRow> = keys.iter().filter_map(|key| self.row(key)).collect();
        rows.sort_by_key(row_sort_key);
        Ok(rows)
    }

    /// Counts over the filtered components.
    pub fn summary(&self, filter: &BimFilter) -> CadResult<BimSummary> {
        let keys = self.filter(filter)?;
        let mut by_category: BTreeMap<String, (String, usize)> = BTreeMap::new();
        let mut by_level: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_material: BTreeMap<String, usize> = BTreeMap::new();

        for key in &keys {
            let Some(group) = self.groups.get(key) else {
                continue;
            };
            if let Some(category) = &group.category {
                let entry = by_category
                    .entry(fold(category))
                    .or_insert_with(|| (category.clone(), 0));
                if category.as_str() < entry.0.as_str() {
                    entry.0 = category.clone();
                }
                entry.1 += 1;
            }
            if let Some(level) = &group.level {
                *by_level.entry(level.clone()).or_default() += 1;
            }
            if let Some(element_type) = &group.element_type {
                *by_type.entry(element_type.clone()).or_default() += 1;
            }
            if let Some(material) = &group.material {
                *by_material.entry(material.clone()).or_default() += 1;
            }
        }

        Ok(BimSummary {
            total: keys.len(),
            by_category: by_category
                .into_iter()
                .map(|(key, (name, count))| BimFacetCount { key, name, count })
                .collect(),
            by_level: facet_counts(by_level, &self.levels),
            by_type: facet_counts(by_type, &self.types),
            by_material: facet_counts(by_material, &self.materials),
        })
    }

    fn row(&self, key: &BimKey) -> Option<BimRow> {
        let group = self.groups.get(key)?;
        Some(BimRow {
            object: key.clone(),
            entity_ids: group.entities.iter().copied().collect(),
            category: group.category.clone(),
            name: group.name.clone(),
            level_name: group
                .level
                .as_deref()
                .and_then(|id| self.levels.name(id))
                .map(str::to_string),
            type_name: group
                .element_type
                .as_deref()
                .and_then(|id| self.types.name(id))
                .map(str::to_string),
            material_name: group
                .material
                .as_deref()
                .and_then(|id| self.materials.name(id))
                .map(str::to_string),
            source: group.source.clone(),
            evidence: group.evidence,
            property_summary: group
                .psets
                .iter()
                .map(|set| BimPropertyCount {
                    name: set.name.clone(),
                    count: set.properties.len(),
                })
                .collect(),
        })
    }
}

fn apply_facet(matched: &mut Option<BTreeSet<BimKey>>, terms: &[String], facet: &Facet) {
    if terms.is_empty() {
        return;
    }
    let selected = facet.keys_of_ids(&facet.resolve_ids(terms));
    intersect(matched, selected);
}

fn intersect(matched: &mut Option<BTreeSet<BimKey>>, selected: BTreeSet<BimKey>) {
    *matched = Some(match matched.take() {
        Some(current) => current.intersection(&selected).cloned().collect(),
        None => selected,
    });
}

fn facet_counts(counts: BTreeMap<String, usize>, facet: &Facet) -> Vec<BimFacetCount> {
    counts
        .into_iter()
        .map(|(id, count)| BimFacetCount {
            name: facet.name(&id).unwrap_or(&id).to_string(),
            key: id,
            count,
        })
        .collect()
}

fn row_sort_key(row: &BimRow) -> (String, String, String, EntityId, BimKey) {
    let source_id = match &row.object {
        BimKey::Source { id, .. } => id.clone(),
        BimKey::Entity(entity) => entity.0.to_string(),
    };
    (
        row.category.as_deref().map(fold).unwrap_or_default(),
        row.name.clone().unwrap_or_default(),
        source_id,
        row.entity_ids.first().copied().unwrap_or(EntityId(0)),
        row.object.clone(),
    )
}

// ------------------------------------------------------------ query service ---

impl QueryService {
    /// BIM rows for `filter`, paged exactly like [`Self::layers`].
    ///
    /// A document switch is [`CadError::StaleResult`]; the revision is taken
    /// from the database and the index is rebuilt when it does not match the
    /// cached `(DatabaseId, Revision)`.
    pub fn bim_query(
        &self,
        database: &DrawingDatabase,
        filter: &BimFilter,
        mut request: QueryRequest,
    ) -> CadResult<QueryPage<BimRow>> {
        self.check_fresh(&request)?;
        filter.validate()?;
        let rows = self.with_bim_index(database, |index| index.rows(filter))?;
        request.revision = database.revision();
        self.remember(&request.document, request.revision, Some(database.id()));
        crate::emit(request, rows)
    }

    /// Counts for `filter`.
    ///
    /// The summary is a single logical row, so it travels in the same paged
    /// envelope as [`Self::bim_query`] (`total == 1`, `rows[0]` is the summary).
    pub fn bim_summary(
        &self,
        database: &DrawingDatabase,
        filter: &BimFilter,
        mut request: QueryRequest,
    ) -> CadResult<QueryPage<BimSummary>> {
        self.check_fresh(&request)?;
        filter.validate()?;
        let summary = self.with_bim_index(database, |index| index.summary(filter))?;
        request.revision = database.revision();
        self.remember(&request.document, request.revision, Some(database.id()));
        crate::emit(request, vec![summary])
    }

    fn with_bim_index<T>(
        &self,
        database: &DrawingDatabase,
        evaluate: impl FnOnce(&BimIndex) -> T,
    ) -> T {
        let mut cache = self.bim_index.borrow_mut();
        let stale = match cache.as_ref() {
            Some((database_id, revision, _)) => {
                *database_id != database.id() || *revision != database.revision()
            }
            None => true,
        };
        if stale {
            *cache = Some((
                database.id(),
                database.revision(),
                BimIndex::build(database),
            ));
        }
        let (_, _, index) = cache.as_ref().expect("the index was built just above");
        evaluate(index)
    }
}

#[cfg(test)]
mod tests;
