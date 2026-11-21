//! Builder setters and the `finish()` gate for parameter state (ADR-0043).
//!
//! Split out of `drawing/param.rs` for the file-size gate: `param.rs` owns the
//! read/write semantics, this module owns the import/load construction rules.
//! `builder.rs` itself exposes `database`/`errors` crate-wide and only wires the
//! single `validate_all` call into `finish()`.

use cad_domain::param::{self, ParamBindings, ParamError, ParamTable};
use cad_domain::{CadError, CadResult, EntityId};

use super::param::{to_cad, PARAM_MAX_CONFIGS};
use super::DrawingDatabase;

// ---------------------------------------------------------------------------
// Builder path.
// ---------------------------------------------------------------------------

impl crate::DrawingDatabaseBuilder {
    /// Replace the document's parameter table (ADR-0043 D4).
    ///
    /// Every row was already shape-checked by [`ParamTable::insert`]. Cross-row
    /// references and the table budget are checked once at
    /// [`crate::DrawingDatabaseBuilder::finish`], where the whole document
    /// exists. Nothing is truncated: an over-budget table is refused.
    pub fn set_parameters(&mut self, parameters: ParamTable) -> CadResult<()> {
        self.database.parameters = parameters;
        Ok(())
    }

    /// Replace the document's configuration overlays (ADR-0043 P1).
    ///
    /// Row shape is checked by the constructors; the cross-row rules (unknown
    /// parent, cycles, overrides/suppression of a missing base sequence) are
    /// checked once at [`crate::DrawingDatabaseBuilder::finish`], where the base
    /// table is final.
    pub fn set_param_configs(&mut self, configs: param::ParamConfigTable) -> CadResult<()> {
        self.database.param_configs = configs;
        Ok(())
    }

    /// Record one element's slot bindings (ADR-0043 D3).
    ///
    /// The entity must already exist (mirroring `set_entity_bim`): a binding for
    /// an id no element row owns could never be written, and dropping it would
    /// be a silent loss. Slot validity and kind matching are checked at
    /// [`crate::DrawingDatabaseBuilder::finish`].
    pub fn set_param_bindings(&mut self, id: EntityId, bindings: ParamBindings) -> CadResult<()> {
        if !self.database.entities.contains_key(&id) {
            return Err(CadError::InvalidInput(format!(
                "param.target_missing: bindings for unknown entity {}",
                id.0
            )));
        }
        if bindings.is_empty() {
            // An empty block is no block: never stamp the element row (and the
            // document's minor tier) with `"params": {}`.
            self.database.param_bindings.remove(&id);
            return Ok(());
        }
        self.database.param_bindings.insert(id, bindings);
        Ok(())
    }
}

/// Check a whole document's parameter state (the builder `finish()` gate).
///
/// Rules, all static refusals (D6 first tier):
///
/// * every binding target exists;
/// * an element with non-empty bindings has an evaluator for its geometry type
///   (`param.no_evaluator` — P0 refuses instead of guessing);
/// * the base table evaluates (dangling references, cycles and budgets);
/// * every binding resolves against those values (unknown slot, kind mismatch);
/// * the configuration rows validate against the final base table and an active
///   selection resolves (ADR-0043 P1).
///
/// The call site in `builder.rs` stays one line because that file is at the
/// 1000-line gate.
pub(crate) fn validate_all(database: &DrawingDatabase) -> CadResult<()> {
    // A document that can be saved must be able to open: the load reconciler
    // refuses a larger binding set, so the builder gate must refuse it too.
    if database.param_bindings.len() > param::PARAM_MAX_AFFECTED {
        return Err(to_cad(ParamError::new(
            param::PARAM_BUDGET_EXCEEDED,
            format!(
                "{} elements carry parameter bindings, over the {}-element budget",
                database.param_bindings.len(),
                param::PARAM_MAX_AFFECTED
            ),
        )));
    }
    if database.param_configs.len() > PARAM_MAX_CONFIGS {
        return Err(to_cad(ParamError::new(
            param::PARAM_BUDGET_EXCEEDED,
            format!(
                "{} configurations, over the {PARAM_MAX_CONFIGS}-row budget",
                database.param_configs.len()
            ),
        )));
    }
    // Configuration rows are validated against the final base table: unknown
    // parents, cycles and overrides/suppression of a missing sequence are
    // refusals, exactly like a malformed parameter row.
    database
        .param_configs
        .validate(&database.parameters)
        .map_err(to_cad)?;
    if let Some(active) = database.active_configuration.as_deref() {
        param::resolve_active(&database.parameters, &database.param_configs, Some(active))
            .map_err(to_cad)?;
    }
    // Bindings resolve against the **base** table: a binding to a suppressed
    // parameter is the documented second tier at derivation time, not a reason
    // to refuse the document.
    let values = param::evaluate_table(&database.parameters).map_err(to_cad)?;
    for (id, bindings) in &database.param_bindings {
        let entity = database.entities.get(id).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "param.target_missing: bindings for unknown entity {}",
                id.0
            ))
        })?;
        if bindings.is_empty() {
            continue;
        }
        let type_key = param::geometry_type_key(&entity.geometry).ok_or_else(|| {
            to_cad(ParamError::new(
                param::PARAM_NO_EVALUATOR,
                format!(
                    "entity {} has parameter bindings but its geometry type has no evaluator",
                    id.0
                ),
            ))
        })?;
        bindings.resolve(type_key, &values).map_err(to_cad)?;
    }
    // P2/P3 tables: element/input/endpoint existence and slot resolution are
    // document-level rules, so they share this one finish gate.
    crate::feature::validate_features(database)?;
    crate::relation::validate_in_context_relations(database)?;
    Ok(())
}
