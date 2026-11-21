//! Parametric feature definitions (`tables/features.json`, ADR-0043 P2).
//!
//! This module is the **schema half** of P2. A feature row names an ordered
//! operation over existing elements and a parameter-bound slot map; no
//! operation has an evaluator in this build, so
//! [`DrawingDatabase::evaluate_feature`] refuses every one with
//! `rel.not_implemented` instead of fabricating a solid body.
//!
//! ```json
//! [{"id":{"kind":"entity","entityId":123},"order":0,"operation":"extrude",
//!   "inputs":[{"kind":"entity","entityId":122}],
//!   "parameters":{"depth":{"ref":7}}}]
//! ```
//!
//! # Identity deviation from the ADR wording
//!
//! The ADR describes features as their own definitions. This format attaches a
//! feature to an **existing element row** by a persistent entity-family id
//! (`id.entityId`): the feature is a definition *of* that element, not a second
//! non-graphic element. That keeps the entity set, picking, DWG export and every
//! snapshot row exactly as they are — no "skip this element" branch is needed in
//! any consumer.
//!
//! # Strictness
//!
//! Duplicate ids and duplicate `order` values are refused (evaluation order must
//! be total). Inputs must not name the feature's own element. The slot map reuses
//! the element `params` binding rules, so an unknown slot or a kind mismatch is
//! the same static refusal. The row/encoded budgets are refusals, never
//! truncations.

use std::collections::BTreeMap;

use cad_domain::param::{self, ParamBindings, ParamError};
use cad_domain::relation::REL_NOT_IMPLEMENTED;
use cad_domain::{CadError, CadResult, EntityId, TransactionId};

use super::DrawingDatabase;

/// Most feature rows `tables/features.json` may hold.
pub const FEATURE_MAX_DEFINITIONS: usize = 4096;

/// Most bytes the encoded feature table may occupy.
pub const FEATURE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// One P2 operation token.
///
/// The list is the ADR's closed P2 vocabulary. `Fillet` and `Shell` exist so a
/// document can record intent, but this build evaluates **none** of them (see
/// [`DrawingDatabase::evaluate_feature`]); they are refused exactly like the
/// solid operations rather than "partially supported".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FeatureOperation {
    Extrude,
    Revolve,
    Loft,
    Sweep,
    BooleanUnion,
    BooleanSubtract,
    BooleanIntersect,
    Fillet,
    Shell,
    Hole,
    Pattern,
}

impl FeatureOperation {
    /// The persisted spelling. Part of the format.
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Extrude => "extrude",
            Self::Revolve => "revolve",
            Self::Loft => "loft",
            Self::Sweep => "sweep",
            Self::BooleanUnion => "boolean_union",
            Self::BooleanSubtract => "boolean_subtract",
            Self::BooleanIntersect => "boolean_intersect",
            Self::Fillet => "fillet",
            Self::Shell => "shell",
            Self::Hole => "hole",
            Self::Pattern => "pattern",
        }
    }

    /// Parse the persisted spelling; an unknown token is a malformed document.
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "extrude" => Self::Extrude,
            "revolve" => Self::Revolve,
            "loft" => Self::Loft,
            "sweep" => Self::Sweep,
            "boolean_union" => Self::BooleanUnion,
            "boolean_subtract" => Self::BooleanSubtract,
            "boolean_intersect" => Self::BooleanIntersect,
            "fillet" => Self::Fillet,
            "shell" => Self::Shell,
            "hole" => Self::Hole,
            "pattern" => Self::Pattern,
            _ => return None,
        })
    }
}

/// One feature definition attached to an element.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureDefinition {
    /// The **element** this feature defines (a persistent entity-family id).
    pub id: EntityId,
    /// Evaluation order; unique across the table, ascending.
    pub order: u32,
    pub operation: FeatureOperation,
    /// Input elements; must exist and must not contain `id`.
    pub inputs: Vec<EntityId>,
    /// Slot → expression map, validated against `id`'s geometry type.
    pub parameters: ParamBindings,
}

/// Every feature definition of one document, unique by id **and** by order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeatureDefinitions {
    rows: BTreeMap<EntityId, FeatureDefinition>,
    orders: BTreeMap<u32, EntityId>,
}

impl FeatureDefinitions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert one feature. Duplicate ids and duplicate orders are refused.
    pub fn insert(&mut self, row: FeatureDefinition) -> CadResult<()> {
        if self.rows.contains_key(&row.id) {
            return Err(CadError::InvalidInput(format!(
                "feature.duplicate_id: entity {} already has a feature",
                row.id.0
            )));
        }
        if self.orders.contains_key(&row.order) {
            return Err(CadError::InvalidInput(format!(
                "feature.duplicate_order: order {} is already used",
                row.order
            )));
        }
        if self.rows.len() >= FEATURE_MAX_DEFINITIONS {
            return Err(CadError::InvalidInput(format!(
                "feature.budget_exceeded: a document may hold at most \
                 {FEATURE_MAX_DEFINITIONS} features"
            )));
        }
        if row.inputs.contains(&row.id) {
            return Err(CadError::InvalidInput(format!(
                "feature.self_input: entity {} cannot be its own input",
                row.id.0
            )));
        }
        self.orders.insert(row.order, row.id);
        self.rows.insert(row.id, row);
        Ok(())
    }

    pub fn get(&self, id: EntityId) -> Option<&FeatureDefinition> {
        self.rows.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &FeatureDefinition> {
        self.rows.values()
    }

    /// Rows in evaluation order (ascending `order`).
    pub fn ordered(&self) -> impl Iterator<Item = &FeatureDefinition> {
        self.orders.values().filter_map(|id| self.rows.get(id))
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

impl DrawingDatabase {
    /// The document's feature definitions (empty when it has none).
    pub fn features(&self) -> &FeatureDefinitions {
        &self.features
    }

    /// Evaluate one feature operation.
    ///
    /// P2 is the **schema boundary**: every operation — including `fillet` and
    /// `shell`, which the ADR refuses outright — returns
    /// `rel.not_implemented` with the operation named. No geometry is
    /// fabricated, no revision is raised and no state changes.
    pub fn evaluate_feature(&mut self, id: EntityId, _transaction: TransactionId) -> CadResult<()> {
        let feature = self.features.get(id).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "feature.target_missing: entity {} has no feature definition",
                id.0
            ))
        })?;
        Err(CadError::InvalidInput(format!(
            "{REL_NOT_IMPLEMENTED}: feature operation {:?} has no evaluator",
            feature.operation.as_token()
        )))
    }
}

// ---------------------------------------------------------------------------
// Builder + finish gate.
// ---------------------------------------------------------------------------

impl crate::DrawingDatabaseBuilder {
    /// Replace the document's feature definitions (ADR-0043 P2).
    ///
    /// Row shape and uniqueness are checked by [`FeatureDefinitions::insert`];
    /// element/input existence and slot resolution are checked at
    /// [`crate::DrawingDatabaseBuilder::finish`], where all elements exist.
    pub fn set_features(&mut self, features: FeatureDefinitions) -> CadResult<()> {
        self.database.features = features;
        Ok(())
    }
}

/// Validate the feature table against the finished document.
pub(crate) fn validate_features(database: &DrawingDatabase) -> CadResult<()> {
    if database.features.len() > FEATURE_MAX_DEFINITIONS {
        return Err(CadError::InvalidInput(format!(
            "feature.budget_exceeded: {} rows over the {FEATURE_MAX_DEFINITIONS}-row budget",
            database.features.len()
        )));
    }
    for row in database.features.iter() {
        let _entity = database.entities.get(&row.id).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "feature.target_missing: feature entity {} does not exist",
                row.id.0
            ))
        })?;
        for input in &row.inputs {
            if !database.entities.contains_key(input) {
                return Err(CadError::InvalidInput(format!(
                    "feature.input_missing: feature {} names input {} which does not exist",
                    row.id.0, input.0
                )));
            }
        }
    }
    validate_feature_bindings(database, &database.parameters)
}

/// Resolve every feature slot map against a base parameter table.
///
/// Called by the `finish()` gate and again by `apply_parametric_update` with the
/// **post-patch** table, so deleting a parameter a feature binds is refused
/// before the mutation (the writer would otherwise save a table the reader
/// rejects).
pub(crate) fn validate_feature_bindings(
    database: &DrawingDatabase,
    parameters: &cad_domain::param::ParamTable,
) -> CadResult<()> {
    let values = param::evaluate_table(parameters).map_err(to_cad)?;
    for row in database.features.iter() {
        if row.parameters.is_empty() {
            continue;
        }
        let entity = database.entities.get(&row.id).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "feature.target_missing: feature entity {} does not exist",
                row.id.0
            ))
        })?;
        let type_key = param::geometry_type_key(&entity.geometry).ok_or_else(|| {
            to_cad(ParamError::new(
                param::PARAM_NO_EVALUATOR,
                format!(
                    "feature {} binds parameters but its geometry type has no evaluator",
                    row.id.0
                ),
            ))
        })?;
        row.parameters.resolve(type_key, &values).map_err(to_cad)?;
    }
    Ok(())
}

fn to_cad(error: ParamError) -> CadError {
    CadError::InvalidInput(format!("{}: {}", error.code(), error.detail()))
}
