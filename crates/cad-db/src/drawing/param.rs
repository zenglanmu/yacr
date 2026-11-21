//! Document parameters, element slot bindings and their merged write path
//! (ADR-0043 D4/D5/D6/D7/D8).
//!
//! The authoritative state is two pieces of document data, both optional:
//!
//! * [`ParamTable`] — the named, typed expressions (`tables/parameters.json`),
//!   keyed by the table-local [`ParamSeq`] (never a `PersistentId`, D4/D12);
//! * one [`ParamBindings`] map per element, stored **beside** the entity row
//!   exactly like render attributes and BIM data, and written as the optional
//!   `params` block of that element's shard row.
//!
//! Nothing here stores a derived value (D7): [`DrawingDatabase::param_values`]
//! evaluates the table every time, and the element's `geometry` is simply the
//! last successful evaluation. A failure to construct geometry is therefore
//! naturally "keep last good" (D6): the parameters commit, the element does not
//! receive a new geometry, and a [`ParamDiagnostic`] records why.
//!
//! The module owns three surfaces:
//!
//! * the builder side ([`crate::DrawingDatabaseBuilder::set_parameters`],
//!   [`crate::DrawingDatabaseBuilder::set_param_bindings`]) with the
//!   `finish()` gate in [`validate_all`];
//! * the read side ([`DrawingDatabase::parameters`],
//!   [`DrawingDatabase::param_bindings`], [`DrawingDatabase::param_values`]);
//! * the single merged write path
//!   ([`DrawingDatabase::apply_parametric_update`]) and the before/after record
//!   types `cad-history` carries verbatim.
//!
//! Every refusal is explicit and carries a stable `param.*` marker from
//! [`cad_domain::param`]; there is no partial commit and no "evaluate what we
//! can" mode.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::param::{
    self, ParamBindings, ParamError, ParamRow, ParamSeq, ParamTable, ParamValues,
};
use cad_domain::{
    CadError, CadResult, EntityId, ObjectId, Revision, SemanticGeometry, TransactionId,
};

use super::DrawingDatabase;
use crate::scope::TableKind;
use crate::{ChangeMask, ChangeScope, ChangeSet, ObjectChange};

/// Most bytes the encoded `tables/parameters.json` may occupy (ADR-0043 D5).
///
/// Mirrors `BIM_MAX_RELATION_BYTES`: the same budget is enforced by the writer
/// (before a save) and by the reader (before an over-budget archive is decoded).
pub const PARAM_MAX_TABLE_BYTES: usize = 8 * 1024 * 1024;

/// Most configuration rows `tables/configurations.json` may hold (ADR-0043 P1).
pub const PARAM_MAX_CONFIGS: usize = 1024;

/// Most bytes the encoded configuration table may occupy (ADR-0043 P1).
pub const PARAM_MAX_CONFIG_BYTES: usize = 8 * 1024 * 1024;

/// A before/after record of one parameter row (ADR-0043 D8).
///
/// `None` on either side means the row did not exist (a create) / no longer
/// exists (a delete). The `seq` is carried separately so a patch is addressable
/// even when both sides are absent, and so a rename can never be mistaken for a
/// different identity.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamPatch {
    pub seq: ParamSeq,
    pub before: Option<ParamRow>,
    pub after: Option<ParamRow>,
}

/// A before/after record of one element's slot bindings and the geometry each
/// side applied (ADR-0043 D8).
///
/// `after: None` removes the element's `params` block. `after_geometry` is the
/// geometry the element had when `after` applied; when present the apply path
/// installs it **verbatim** (exact replay) instead of re-deriving, which is how
/// undoing a bind restores the pre-bind geometry and why the history step no
/// longer needs to duplicate entity rows.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamBindingPatch {
    pub id: EntityId,
    pub before: Option<ParamBindings>,
    pub after: Option<ParamBindings>,
    /// The geometry the element had when `before` applied.
    pub before_geometry: Option<SemanticGeometry>,
    /// The geometry the element had when `after` applied.
    pub after_geometry: Option<SemanticGeometry>,
}

/// One refusal to re-derive an element's geometry (ADR-0043 D6 second tier).
///
/// `code` is `param.blocked`: the parameters are legal and were committed, but
/// this element's geometry construction failed (the detail carries the
/// underlying `param.geometry_failed`/`param.unknown_reference` text) and the
/// element kept its last-good geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamDiagnostic {
    pub entity: Option<EntityId>,
    pub code: &'static str,
    pub detail: String,
}

/// Everything one [`DrawingDatabase::apply_parametric_update`] produced.
#[derive(Debug, Clone, PartialEq)]
pub struct ParametricOutcome {
    /// The single [`ChangeSet`] the commit published (empty on a no-op batch).
    pub changes: ChangeSet,
    /// The evaluated table after the commit. Never persisted (D3/D7).
    pub values: ParamValues,
    /// Elements whose bindings were actually resolved against `values`.
    pub evaluated: BTreeSet<EntityId>,
    /// Elements whose geometry was replaced, in ascending id order.
    pub updated: Vec<EntityId>,
    /// Elements that kept their last-good geometry, with the reason.
    pub diagnostics: Vec<ParamDiagnostic>,
}

impl DrawingDatabase {
    /// The document's parameter rows (empty when it has none).
    pub fn parameters(&self) -> &ParamTable {
        &self.parameters
    }

    /// The slot bindings of one element, if it has any.
    pub fn param_bindings(&self, id: EntityId) -> Option<&ParamBindings> {
        self.param_bindings.get(&id)
    }

    /// Evaluate the document's **effective** parameter table (D3/D7).
    ///
    /// The active configuration (if any) is resolved first: overrides replace
    /// base expressions and suppressed rows are absent. The stored base table is
    /// never modified by this.
    ///
    /// A dangling reference, a cycle, an unknown/misconfigured active
    /// configuration is surfaced as a stable `param.*` [`CadError::InvalidInput`]
    /// rather than an empty result.
    pub fn param_values(&self) -> CadResult<ParamValues> {
        let (effective, _) = self.effective_parameters()?;
        param::evaluate_table(&effective).map_err(to_cad)
    }

    /// The document's configuration overlays (empty when it has none).
    pub fn param_configs(&self) -> &param::ParamConfigTable {
        &self.param_configs
    }

    /// The configuration currently applied to derivations, if any.
    ///
    /// Runtime only: this is not persisted, so a reopened document starts with
    /// no configuration selected.
    pub fn active_configuration(&self) -> Option<&str> {
        self.active_configuration.as_deref()
    }

    /// How the active configuration relates to the base table.
    pub fn param_config_status(&self) -> CadResult<param::ConfigStatus> {
        self.effective_parameters().map(|(_, status)| status)
    }

    /// Resolve the effective table and its status for the current selection.
    fn effective_parameters(&self) -> CadResult<(ParamTable, param::ConfigStatus)> {
        param::resolve_active(
            &self.parameters,
            &self.param_configs,
            self.active_configuration.as_deref(),
        )
        .map_err(to_cad)
    }

    /// Diagnostics produced by the last load-time reconciliation (ADR-0043 D7).
    ///
    /// Empty for a freshly imported document and for a load whose stored
    /// geometry already agreed with the parameter table. The list is in memory
    /// only — `DocumentSnapshot` never carries it.
    pub fn param_diagnostics(&self) -> &[ParamDiagnostic] {
        &self.param_diagnostics
    }

    /// Apply parameter-row and binding patches as **one** revision (ADR-0043 D8).
    ///
    /// The commit is all-or-nothing in two tiers (D6):
    ///
    /// 1. **Static** failures refuse the whole batch before anything mutates: a
    ///    patch whose sides disagree about its `seq`, a duplicate sequence or
    ///    binding target, a binding target that does not exist
    ///    (`param.target_missing`), an element with bindings but no evaluator
    ///    (`param.no_evaluator`), an unknown slot (`param.unknown_slot`), a kind
    ///    mismatch (`param.unit_mismatch`), any `resolve` failure on an affected
    ///    element (dangling reference included), and any table-level failure from
    ///    [`cad_domain::param::evaluate_table`] (dangling reference, cycle,
    ///    budget). Nothing is partially written. This is what stops the public
    ///    path from persisting a document its own builder would refuse.
    /// 2. **Derived geometry** failures do not refuse the commit: only a
    ///    `param.geometry_failed` from
    ///    [`cad_domain::param::apply_slot`] keeps the element's last-good
    ///    geometry and publishes a [`ParamDiagnostic`] whose code is
    ///    `param.blocked`. Every other slot error (`param.unknown_slot`,
    ///    `param.no_evaluator`, `param.invalid_value`) is still a static refusal.
    ///
    /// # Locked layers
    ///
    /// A real geometry replacement is a row modification and goes through
    /// [`DrawingDatabase::ensure_change_allowed_on_locked_layer`]; a real binding
    /// change rewrites the row's `params` block and goes through
    /// [`DrawingDatabase::ensure_row_writable`]. Both run before phase 3 mutates
    /// anything, so a locked layer refuses the whole batch with `layer.locked`.
    /// A no-op patch (identical bindings and geometry) is not a modification and
    /// is allowed, mirroring the display path's "only effective changes" rule.
    ///
    /// # Derived geometry carried on the patch
    ///
    /// [`ParamBindingPatch::after_geometry`], when present, is installed verbatim
    /// (exact replay) instead of re-deriving. That is how undoing a bind restores
    /// the pre-bind geometry (`after: None` plus `after_geometry: Some(..)`), and
    /// why a history step no longer duplicates entity rows.
    ///
    /// Only the changed reference closure is re-evaluated
    /// ([`cad_domain::param::affected_params`] →
    /// [`cad_domain::param::affected_entities`]), in ascending `EntityId` order,
    /// so the result is deterministic.
    ///
    /// An empty batch is a no-op: same revision, no changes, an empty scope.
    /// Any content change raises the revision by exactly one.
    ///
    /// # Why the scope must be filled here
    ///
    /// Both halves of the edit are persisted rows: `tables/parameters.json` and
    /// the element shard rows carrying the `params` block **and** the rewritten
    /// `geometry`. `cad-store-zip`'s `copyable_shard` decides raw-copy purely
    /// from `scope.shards ∪ scope.prev_shards`. An unmarked shard is copied
    /// verbatim, so a missing `element()` makes the **first** save look correct
    /// (the writer rewrote the file for an unrelated reason) and the **second**
    /// save silently re-emit the old `params`/geometry bytes while reporting
    /// success. That is the exact failure mode `apply_bim_updates` documents.
    ///
    /// Every binding-patch target is marked even when its bindings did not
    /// change, and every replaced geometry is marked; the header is never marked
    /// because parameter identity is table-local (no `IdKind`, no counter — D12).
    pub fn apply_parametric_update(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        param_patches: &[ParamPatch],
        binding_patches: &[ParamBindingPatch],
    ) -> CadResult<ParametricOutcome> {
        let before = self.revision;
        let empty = |values: ParamValues| ParametricOutcome {
            changes: ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
                scope: ChangeScope::default(),
            },
            values,
            evaluated: BTreeSet::new(),
            updated: Vec::new(),
            diagnostics: Vec::new(),
        };
        if param_patches.is_empty() && binding_patches.is_empty() {
            return Ok(empty(self.param_values()?));
        }

        // ---- Phase 1: validate everything before mutating anything. -------------

        let mut next_table = self.parameters.clone();
        let mut changed_seqs: BTreeSet<ParamSeq> = BTreeSet::new();
        let mut seen_seqs: BTreeSet<ParamSeq> = BTreeSet::new();
        for patch in param_patches {
            if !seen_seqs.insert(patch.seq) {
                return Err(CadError::Invariant(format!(
                    "a parameter batch names sequence {} more than once",
                    patch.seq.0
                )));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(format!(
                    "param.invalid_value: parameter patch {} needs a before or after row",
                    patch.seq.0
                )));
            }
            for row in patch.before.iter().chain(patch.after.iter()) {
                if row.seq != patch.seq {
                    return Err(CadError::InvalidInput(format!(
                        "param.invalid_value: parameter patch {} carries a row with sequence {}",
                        patch.seq.0, row.seq.0
                    )));
                }
            }
            // A patch that repeats the stored row changes no table content; it
            // must not make dependents look affected.
            if patch.before == patch.after {
                continue;
            }
            changed_seqs.insert(patch.seq);
            match &patch.after {
                Some(row) => next_table.insert(row.clone()).map_err(to_cad)?,
                None => {
                    next_table.remove(patch.seq);
                }
            }
        }

        // Every configuration overlay (not only the active one) must still name
        // rows of the post-patch base table, and every feature slot map must
        // still resolve. Otherwise the edit would save a table the reader
        // refuses to open.
        self.param_configs.validate(&next_table).map_err(to_cad)?;
        crate::feature::validate_feature_bindings(self, &next_table)?;

        let mut seen_entities: BTreeSet<EntityId> = BTreeSet::new();
        let mut next_bindings: BTreeMap<EntityId, Option<ParamBindings>> = BTreeMap::new();
        let mut next_geometry: BTreeMap<EntityId, SemanticGeometry> = BTreeMap::new();
        for patch in binding_patches {
            if !seen_entities.insert(patch.id) {
                return Err(CadError::Invariant(format!(
                    "a parameter batch names entity {:?} more than once",
                    patch.id
                )));
            }
            if !self.entities.contains_key(&patch.id) {
                return Err(CadError::InvalidInput(format!(
                    "param.target_missing: entity {} does not exist in this drawing",
                    patch.id.0
                )));
            }
            // `Some(empty)` is not a stored block: normalize it to `None` so the
            // element row never gains `"params": {}` (minor tier 8 for nothing).
            next_bindings.insert(
                patch.id,
                patch.after.clone().filter(|bindings| !bindings.is_empty()),
            );
            if let Some(geometry) = &patch.after_geometry {
                next_geometry.insert(patch.id, (*geometry).clone());
            }
        }

        // The new base table must be a DAG, and the active overlay must still
        // resolve over it. All four tables (base/effective before and after) are
        // kept distinct: only the base is ever stored.
        let (before_effective, _) = self.effective_parameters()?;
        let (after_effective, after_status) = param::resolve_active(
            &next_table,
            &self.param_configs,
            self.active_configuration.as_deref(),
        )
        .map_err(to_cad)?;
        let values = param::evaluate_table(&after_effective).map_err(to_cad)?;

        for patch in binding_patches {
            let Some(after) = &patch.after else { continue };
            if after.is_empty() {
                continue;
            }
            let entity = match self.entities.get(&patch.id) {
                Some(entity) => entity,
                // The target existence check above already refused a missing
                // row; an orphaned binding can never be indexed into a panic.
                None => continue,
            };
            let type_key = param::geometry_type_key(&entity.geometry).ok_or_else(|| {
                to_cad(ParamError::new(
                    param::PARAM_NO_EVALUATOR,
                    format!(
                        "entity {} has parameter bindings but its geometry type has no evaluator",
                        patch.id.0
                    ),
                ))
            })?;
            if let Err(error) = after.resolve(type_key, &values) {
                // A binding that names a suppressed parameter is not a static
                // failure: the element is blocked and keeps last-good geometry.
                if error.code() == param::PARAM_UNKNOWN_REFERENCE
                    && references_suppressed(after, &after_status.suppressed)
                {
                    continue;
                }
                return Err(to_cad(error));
            }
        }

        // ---- Phase 2: re-evaluate the affected closure, ascending id order. -----

        let mut affected = param::affected_params(&before_effective, &changed_seqs);
        affected.extend(param::affected_params(&after_effective, &changed_seqs));
        let directly_changed: BTreeSet<EntityId> = binding_patches
            .iter()
            .filter(|patch| {
                patch
                    .after
                    .as_ref()
                    .is_some_and(|bindings| !bindings.is_empty())
                    || patch.after_geometry.is_some()
            })
            .map(|patch| patch.id)
            .collect();
        // The domain helper checks this while it grows the set; check it before
        // the loop so an over-budget batch cannot slip through when every entry
        // is already in `directly_changed` (the loop then never runs).
        if directly_changed.len() > param::PARAM_MAX_AFFECTED {
            return Err(to_cad(ParamError::new(
                param::PARAM_BUDGET_EXCEEDED,
                format!(
                    "{} binding targets exceed the {}-element budget",
                    directly_changed.len(),
                    param::PARAM_MAX_AFFECTED
                ),
            )));
        }
        let entities = param::affected_entities(&self.param_bindings, &affected, &directly_changed)
            .map_err(to_cad)?;

        let mut evaluated = BTreeSet::new();
        let mut updated = Vec::new();
        let mut diagnostics = Vec::new();
        let mut geometry_writes: Vec<(EntityId, SemanticGeometry)> = Vec::new();

        for id in &entities {
            let Some(entity) = self.entities.get(id) else {
                // Referential integrity is the builder's invariant; a binding to
                // a deleted row would be a bug outside this write path.
                continue;
            };
            // An explicit `after_geometry` is an exact replay (undo of a bind,
            // or a caller that already knows the derived value); otherwise the
            // bindings are re-derived as usual.
            let target = match next_geometry.get(id) {
                Some(geometry) => (*geometry).clone(),
                None => {
                    let bindings = match next_bindings.get(id) {
                        Some(Some(bindings)) => Some(bindings),
                        Some(None) => None,
                        None => self.param_bindings.get(id),
                    };
                    let Some(bindings) = bindings else { continue };
                    if bindings.is_empty() {
                        continue;
                    }
                    evaluated.insert(*id);
                    // A type with no evaluator is a static failure: the builder
                    // already refuses such a binding, and the public path must
                    // not persist a row its own reader would refuse.
                    let type_key = param::geometry_type_key(&entity.geometry).ok_or_else(|| {
                        to_cad(ParamError::new(
                            param::PARAM_NO_EVALUATOR,
                            format!(
                                "entity {} has parameter bindings but its geometry type has no \
                                 evaluator",
                                id.0
                            ),
                        ))
                    })?;
                    // Resolution failures are static (refuse) except a reference
                    // to a parameter the active configuration suppresses: that
                    // element is blocked and keeps its last-good geometry.
                    let resolved = match bindings.resolve(type_key, &values) {
                        Ok(resolved) => resolved,
                        Err(error)
                            if error.code() == param::PARAM_UNKNOWN_REFERENCE
                                && references_suppressed(bindings, &after_status.suppressed) =>
                        {
                            diagnostics.push(suppressed(*id, &error));
                            continue;
                        }
                        Err(error) => return Err(to_cad(error)),
                    };
                    let mut geometry = entity.geometry.clone();
                    let mut failure = None;
                    for (slot, value) in resolved {
                        match param::apply_slot(&geometry, &slot, value) {
                            Ok(next) => geometry = next,
                            Err(error) if error.code() == param::PARAM_GEOMETRY_FAILED => {
                                failure = Some(error);
                                break;
                            }
                            Err(error) => return Err(to_cad(error)),
                        }
                    }
                    if let Some(error) = failure {
                        diagnostics.push(blocked(*id, &error));
                        continue;
                    }
                    geometry
                }
            };
            if target == entity.geometry {
                continue;
            }
            // Geometry is a row write: a locked layer refuses it before anything
            // is mutated, exactly like every other drawing edit.
            let mut candidate = entity.clone();
            candidate.geometry = target.clone();
            self.ensure_change_allowed_on_locked_layer(*id, Some(&candidate))?;
            geometry_writes.push((*id, target));
            updated.push(*id);
        }

        // A real binding change rewrites the `params` block of the row, which is
        // a modification too. Checked before phase 3 mutates anything.
        for patch in binding_patches {
            let before = patch.before.clone().filter(|bindings| !bindings.is_empty());
            let after = patch.after.clone().filter(|bindings| !bindings.is_empty());
            if before != after {
                self.ensure_row_writable(
                    patch.id,
                    &format!(
                        "cannot modify parameter bindings of entity {} on",
                        patch.id.0
                    ),
                )?;
            }
        }

        // ---- Phase 3: mutate, then publish one ChangeSet. -----------------------

        let table_changed = self.parameters != next_table;
        if table_changed {
            self.parameters = next_table;
        }
        for patch in binding_patches {
            match patch.after.clone().filter(|bindings| !bindings.is_empty()) {
                Some(bindings) => {
                    self.param_bindings.insert(patch.id, bindings);
                }
                None => {
                    self.param_bindings.remove(&patch.id);
                }
            }
        }
        for (id, geometry) in &geometry_writes {
            if let Some(entity) = self.entities.get_mut(id) {
                entity.geometry = geometry.clone();
            }
        }

        let mut scope = ChangeScope::default();
        if table_changed {
            scope.table(TableKind::Parameters);
        }
        // Every addressed element, even a no-op patch: the old `params` bytes are
        // not the new bytes, and a raw-copied shard would keep them.
        for patch in binding_patches {
            if let Some(entity) = self.entities.get(&patch.id) {
                scope.element(&entity.space, patch.id.0);
            }
        }
        for id in &entities {
            if let Some(entity) = self.entities.get(id) {
                scope.element(&entity.space, id.0);
            }
        }
        for id in &updated {
            if let Some(entity) = self.entities.get(id) {
                scope.element(&entity.space, id.0);
            }
        }

        let bindings_changed = binding_patches.iter().any(|patch| {
            patch.before.clone().filter(|bindings| !bindings.is_empty())
                != patch.after.clone().filter(|bindings| !bindings.is_empty())
        });
        // Carrying a derived geometry on the patch is itself a recorded change
        // (the undo/redo replay depends on it), so it counts for the bump even
        // when the applied geometry happens to equal the stored one.
        let geometry_patch_changed = binding_patches
            .iter()
            .any(|patch| patch.before_geometry != patch.after_geometry);
        let changed =
            table_changed || bindings_changed || geometry_patch_changed || !updated.is_empty();
        let after = if changed {
            Revision(before.0 + 1)
        } else {
            before
        };
        self.revision = after;

        Ok(ParametricOutcome {
            changes: ChangeSet {
                database: self.id,
                before,
                after,
                transaction,
                reason: reason.to_string(),
                changes: updated
                    .iter()
                    .map(|id| ObjectChange::Update(ObjectId(id.0), ChangeMask::GEOMETRY))
                    .collect(),
                scope,
            },
            values,
            evaluated,
            updated,
            diagnostics,
        })
    }

    /// Select (or clear) the active parameter configuration (ADR-0043 P1).
    ///
    /// The name is validated with [`cad_domain::param::resolve_active`] before
    /// anything mutates, so an unknown name or a parent cycle refuses the whole
    /// call. Selecting the name already active is a no-op (no revision bump).
    ///
    /// A configuration change is **global**: every bound element is re-derived
    /// from the new effective table, in ascending [`EntityId`] order, as one
    /// revision and one [`ChangeSet`]. The scope marks exactly the shards whose
    /// `geometry` was rewritten; the configuration table and the header are
    /// **not** marked (the overlays did not change and the selection is runtime
    /// state, never persisted).
    ///
    /// # Not undoable
    ///
    /// No history patch type exists for a selection, so `PARAM CONFIG` is
    /// deliberately not recorded as a [`cad_db::ParamPatch`] step. The geometry
    /// it derives is re-derived from the current table on the next edit; undo
    /// cannot restore "the other configuration" without recording runtime state.
    /// `cad-app` must not push a history step for this call.
    ///
    /// Locked layers still apply: a rewritten geometry is a row modification and
    /// refuses with `layer.locked` before anything mutates. A geometry that fails
    /// to construct keeps last-good (`param.blocked`); a binding to a parameter
    /// the new configuration suppresses keeps last-good (`param.suppressed`).
    pub fn set_active_configuration(
        &mut self,
        name: Option<String>,
        reason: &str,
        transaction: TransactionId,
    ) -> CadResult<ParametricOutcome> {
        let before = self.revision;
        if self.active_configuration.as_deref() == name.as_deref() {
            return Ok(ParametricOutcome {
                changes: ChangeSet {
                    database: self.id,
                    before,
                    after: before,
                    transaction,
                    reason: reason.to_string(),
                    changes: Vec::new(),
                    scope: ChangeScope::default(),
                },
                values: self.param_values()?,
                evaluated: BTreeSet::new(),
                updated: Vec::new(),
                diagnostics: Vec::new(),
            });
        }

        // Validate the target overlay before touching any state.
        let (effective, status) =
            param::resolve_active(&self.parameters, &self.param_configs, name.as_deref())
                .map_err(to_cad)?;
        let values = param::evaluate_table(&effective).map_err(to_cad)?;

        if self.param_bindings.len() > param::PARAM_MAX_AFFECTED {
            return Err(to_cad(ParamError::new(
                param::PARAM_BUDGET_EXCEEDED,
                format!(
                    "{} elements carry parameter bindings, over the {}-element budget",
                    self.param_bindings.len(),
                    param::PARAM_MAX_AFFECTED
                ),
            )));
        }

        let mut evaluated = BTreeSet::new();
        let mut updated = Vec::new();
        let mut diagnostics = Vec::new();
        let mut geometry_writes: Vec<(EntityId, SemanticGeometry)> = Vec::new();
        for (id, bindings) in &self.param_bindings {
            if bindings.is_empty() {
                continue;
            }
            let Some(entity) = self.entities.get(id) else {
                return Err(CadError::Invariant(format!(
                    "param.target_missing: bindings for unknown entity {}",
                    id.0
                )));
            };
            let type_key = param::geometry_type_key(&entity.geometry).ok_or_else(|| {
                to_cad(ParamError::new(
                    param::PARAM_NO_EVALUATOR,
                    format!(
                        "entity {} has parameter bindings but its geometry type has no evaluator",
                        id.0
                    ),
                ))
            })?;
            let resolved = match bindings.resolve(type_key, &values) {
                Ok(resolved) => resolved,
                Err(error)
                    if error.code() == param::PARAM_UNKNOWN_REFERENCE
                        && references_suppressed(bindings, &status.suppressed) =>
                {
                    diagnostics.push(suppressed(*id, &error));
                    continue;
                }
                Err(error) => return Err(to_cad(error)),
            };
            evaluated.insert(*id);
            let mut geometry = entity.geometry.clone();
            let mut failure = None;
            for (slot, value) in resolved {
                match param::apply_slot(&geometry, &slot, value) {
                    Ok(next) => geometry = next,
                    Err(error) if error.code() == param::PARAM_GEOMETRY_FAILED => {
                        failure = Some(error);
                        break;
                    }
                    Err(error) => return Err(to_cad(error)),
                }
            }
            if let Some(error) = failure {
                diagnostics.push(blocked(*id, &error));
                continue;
            }
            if geometry == entity.geometry {
                continue;
            }
            let mut candidate = entity.clone();
            candidate.geometry = geometry.clone();
            self.ensure_change_allowed_on_locked_layer(*id, Some(&candidate))?;
            updated.push(*id);
            geometry_writes.push((*id, geometry));
        }

        // ---- commit: selection + derived geometry, one revision. -----------------
        self.active_configuration = name;
        for (id, geometry) in &geometry_writes {
            if let Some(entity) = self.entities.get_mut(id) {
                entity.geometry = geometry.clone();
            }
        }
        let mut scope = ChangeScope::default();
        for id in &updated {
            if let Some(entity) = self.entities.get(id) {
                scope.element(&entity.space, id.0);
            }
        }
        // The selection is runtime state: when no geometry was rewritten the
        // revision still advances (the command happened) but the scope stays
        // empty, so a saver has nothing to write.
        self.revision = Revision(before.0 + 1);
        Ok(ParametricOutcome {
            changes: ChangeSet {
                database: self.id,
                before,
                after: self.revision,
                transaction,
                reason: reason.to_string(),
                changes: updated
                    .iter()
                    .map(|id| ObjectChange::Update(ObjectId(id.0), ChangeMask::GEOMETRY))
                    .collect(),
                scope,
            },
            values,
            evaluated,
            updated,
            diagnostics,
        })
    }
}

/// Diagnose bound geometry after a load (ADR-0043 D7/D6 second tier).
///
/// The element row's `geometry` is the **last successful evaluation** (D7); the
/// table and bindings are the authority. Opening compares them without touching
/// the database:
///
/// * a stored geometry that disagrees with the pure evaluation is kept and
///   reported as [`cad_domain::param::PARAM_RECOMPUTED`] — the archive stays the
///   authority until an edit re-derives the value and marks the shard dirty;
/// * a construction failure keeps the stored geometry and reports
///   [`cad_domain::param::PARAM_BLOCKED`]. Reloading the same archive reproduces
///   the same diagnostic because evaluation is a pure function (D6).
///
/// # Why this must not mutate
///
/// There is no dirty scope at load. Replacing the stored geometry here would make
/// the in-memory row disagree with the archive, and the first save would
/// raw-copy the stale shard forever (`copyable_shard` only looks at
/// `ChangeScope`). A load is therefore a **diagnosis**, not an edit: no
/// mutation, no revision bump, no `ChangeSet`, no shard dirty.
///
/// # No lock rule here
///
/// Opening is not an edit, so a locked layer does not refuse anything here. The
/// lock rules live only on [`DrawingDatabase::apply_parametric_update`].
///
/// The binding count is bounded by [`cad_domain::param::PARAM_MAX_AFFECTED`], and
/// the list is assigned in ascending [`EntityId`] order so the result is
/// deterministic.
pub(crate) fn reconcile_loaded_parameters(database: &mut DrawingDatabase) -> CadResult<()> {
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
    let values = param::evaluate_table(&database.parameters).map_err(to_cad)?;
    let mut diagnostics = Vec::new();
    for (id, bindings) in &database.param_bindings {
        if bindings.is_empty() {
            continue;
        }
        let Some(entity) = database.entities.get(id) else {
            return Err(CadError::Invariant(format!(
                "param.target_missing: bindings for unknown entity {}",
                id.0
            )));
        };
        let Some(type_key) = param::geometry_type_key(&entity.geometry) else {
            diagnostics.push(ParamDiagnostic {
                entity: Some(*id),
                code: param::PARAM_BLOCKED,
                detail: format!(
                    "{}: entity {} has bindings but its geometry type has no evaluator",
                    param::PARAM_NO_EVALUATOR,
                    id.0
                ),
            });
            continue;
        };
        let resolved = match bindings.resolve(type_key, &values) {
            Ok(resolved) => resolved,
            Err(error) => {
                diagnostics.push(blocked(*id, &error));
                continue;
            }
        };
        let mut derived = entity.geometry.clone();
        let mut failure = None;
        for (slot, value) in resolved {
            match param::apply_slot(&derived, &slot, value) {
                Ok(next) => derived = next,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        if let Some(error) = failure {
            diagnostics.push(blocked(*id, &error));
            continue;
        }
        if derived != entity.geometry {
            diagnostics.push(ParamDiagnostic {
                entity: Some(*id),
                code: param::PARAM_RECOMPUTED,
                detail: format!(
                    "entity {}: stored geometry disagrees with the parameter table \
                     (stored {:?}, derived {:?}); kept as stored until an edit",
                    id.0, entity.geometry, derived
                ),
            });
        }
    }
    database.param_diagnostics = diagnostics;
    Ok(())
}

fn blocked(id: EntityId, error: &ParamError) -> ParamDiagnostic {
    ParamDiagnostic {
        entity: Some(id),
        code: param::PARAM_BLOCKED,
        detail: error.to_string(),
    }
}

/// Whether any binding expression references a parameter the active
/// configuration suppresses (ADR-0043 P1).
///
/// Only used to classify a `param.unknown_reference` refusal: a binding to a
/// suppressed parameter is the documented second-tier case, every other
/// resolution failure is static.
fn references_suppressed(bindings: &ParamBindings, suppressed: &BTreeSet<ParamSeq>) -> bool {
    if suppressed.is_empty() {
        return false;
    }
    bindings.entries().any(|(_, expr)| {
        let mut refs = BTreeSet::new();
        expr.references(&mut refs);
        refs.iter().any(|seq| suppressed.contains(seq))
    })
}

/// The second-tier diagnostic for an element bound to a suppressed parameter.
fn suppressed(id: EntityId, error: &ParamError) -> ParamDiagnostic {
    ParamDiagnostic {
        entity: Some(id),
        code: param::PARAM_SUPPRESSED,
        detail: error.to_string(),
    }
}

pub(super) fn to_cad(error: ParamError) -> CadError {
    CadError::InvalidInput(format!("{}: {}", error.code(), error.detail()))
}
