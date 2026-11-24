//! `PARAM {op: create|set|delete|rename|bind|unbind}` — parameter table and
//! binding edits (ADR-0043 D8/D9).
//!
//! [`crate::ParamOp::SetValue`] is the table-level value/expression write; the
//! property command's selection-scoped `param.<name>` write
//! ([`super::param_properties`]) is its narrow sibling. The structural operations
//! add/remove/rename rows, and the binding operations attach/detach element slot
//! bindings.
//!
//! Every op runs through the database's one merged write path,
//! [`cad_db::DrawingDatabase::apply_parametric_update`], so one command is one
//! transaction, one revision, one [`cad_db::ChangeSet`] and one undo step — the
//! parameter row and every re-derived element geometry move together. Nothing is
//! cascaded silently: deleting a parameter that a binding or another expression
//! still references is refused (`param.referenced`) instead of dropping the edge,
//! and a `Bind` naming a slot the type does not register is refused by the
//! database's static validation (`param.unknown_slot`).
//!
//! An empty or no-op op reports `param.unchanged`, opens no transaction and
//! records no history.

use super::*;
use cad_db::{DrawingDatabase, ParamBindingPatch, ParamPatch, ParametricOutcome};
use cad_domain::param::{self, ParamBindings, ParamRow, ParamSeq};
use cad_history::DrawingUndoRecord;

impl Application {
    /// Apply one `PARAM` op to the open document.
    ///
    /// The selection is read at apply time for `Bind`/`Unbind`, so a stale UI
    /// cannot retarget another element.
    pub(crate) fn param_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Param(op) = &command.payload else {
            return Err(CadError::InvalidInput(
                "param.invalid_payload: PARAM needs a Param { op } payload".into(),
            ));
        };

        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        let label = op.label();
        if let crate::command_payload::ParamOp::Config { name } = op {
            return self.param_config(document_id, drawing, name.clone());
        }
        if matches!(
            op,
            crate::command_payload::ParamOp::Bind { .. }
                | crate::command_payload::ParamOp::Unbind { .. }
        ) && session.selection.refs().is_empty()
        {
            return Err(CadError::InvalidInput(
                "property.selection_empty: select at least one element to bind".into(),
            ));
        }
        let mut param_patches: Vec<ParamPatch> = Vec::new();
        let mut binding_patches: Vec<ParamBindingPatch> = Vec::new();
        op.plan(
            drawing.as_ref(),
            session,
            command,
            &mut param_patches,
            &mut binding_patches,
        )?;

        if param_patches.is_empty() && binding_patches.is_empty() {
            return Ok(unchanged_outcome(format!(
                "PARAM {label}: nothing to change"
            )));
        }

        let transaction = next_transaction_id();
        let reason = format!("param {label}");
        let mut candidate = drawing.as_ref().clone();
        let outcome = candidate.apply_parametric_update(
            &reason,
            transaction,
            &param_patches,
            &binding_patches,
        )?;

        // The revision is the honest no-op test: a batch whose sides are equal
        // (a rename to the same name) changes nothing and must not be recorded.
        // `ChangeSet::is_empty` only counts object changes, so it would also call
        // a table-only create unchanged.
        if outcome.changes.after == outcome.changes.before {
            return Ok(unchanged_outcome(format!(
                "PARAM {label}: nothing to change"
            )));
        }

        // Fill the `after` side of every binding patch from the committed
        // candidate. The forward bind stores the derived geometry; a forward
        // unbind stores the kept last-good geometry. Replay installs these
        // verbatim, so undoing a bind restores the pre-bind geometry without a
        // duplicate entity row in the history step.
        for patch in &mut binding_patches {
            patch.after_geometry = candidate
                .entity(patch.id)
                .map(|entity| entity.geometry.clone());
        }

        let ParametricOutcome {
            changes,
            updated,
            diagnostics,
            ..
        } = outcome;
        let objects: Vec<ObjectId> = updated.iter().map(|id| ObjectId(id.0)).collect();
        let diagnostics = diagnostics
            .into_iter()
            .map(|diagnostic| Diagnostic {
                object: diagnostic.entity.map(|id| ObjectId(id.0)),
                code: diagnostic.code.into(),
                message: diagnostic.detail,
            })
            .collect();
        self.publish_candidate_with_params(
            document_id,
            candidate,
            DrawingUndoRecord::entity_only(changes.transaction, reason, Vec::new()),
            param_patches,
            binding_patches,
        )?;

        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(changes)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
    /// `PARAM CONFIG`: switch the runtime evaluation overlay.
    ///
    /// **Not undoable.** No history patch type exists for "which configuration
    /// was active", so this path publishes the candidate directly and records no
    /// undo step. The derived geometry still lands in one revision and one
    /// `ChangeSet`; a later edit re-derives from the table in force at that time.
    /// A configuration is a runtime evaluation mode, not stored document data.
    fn param_config(
        &mut self,
        document_id: DocumentId,
        drawing: Arc<DrawingDatabase>,
        name: Option<String>,
    ) -> CadResult<CommandOutcome> {
        let reason = match name.as_deref() {
            Some(name) => format!("param config {name}"),
            None => "param config (base)".to_string(),
        };
        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let outcome = candidate.set_active_configuration(name, &reason, transaction)?;
        if outcome.changes.after == outcome.changes.before {
            return Ok(unchanged_outcome(format!(
                "PARAM config: {reason} already applies"
            )));
        }
        let ParametricOutcome {
            changes,
            updated,
            diagnostics,
            ..
        } = outcome;
        let objects: Vec<ObjectId> = updated.iter().map(|id| ObjectId(id.0)).collect();
        let diagnostics = diagnostics
            .into_iter()
            .map(|diagnostic| Diagnostic {
                object: diagnostic.entity.map(|id| ObjectId(id.0)),
                code: diagnostic.code.into(),
                message: diagnostic.detail,
            })
            .collect();
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        document.drawing = Arc::new(candidate);
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(changes)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
    /// `-PARAMETERS`: the command-line Parameter Manager list.
    ///
    /// Read-only: it evaluates the table under the active configuration and
    /// reports one line per stored row plus the explicit "solver-driven
    /// constraint parameters are not modeled" boundary: this build implements no
    /// geometric/dimensional constraint solver, so the Parameters Manager owns
    /// the user-parameter table only. No transaction, revision bump or undo
    /// entry.
    ///
    /// A returned value is the **effective** one; a row the active configuration
    /// suppresses has no effective value, so it reports `n/a` instead of a
    /// fabricated number.
    pub(crate) fn param_list_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "param.invalid_payload: -PARAMETERS takes no payload".into(),
            ));
        }
        let drawing = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        let status = drawing.param_config_status()?;
        let values = drawing.param_values()?;
        let mut diagnostics = Vec::new();
        let active = match status.name.as_deref() {
            Some(name) => format!(" under configuration {name:?}"),
            None => String::new(),
        };
        diagnostics.push(Diagnostic {
            object: None,
            code: "param.list".into(),
            message: format!(
                "-PARAMETERS: {} user parameter(s){active}",
                drawing.parameters().len()
            ),
        });
        for row in drawing.parameters().rows() {
            let status_token = if status.suppressed.contains(&row.seq) {
                "suppressed"
            } else if status.overridden.contains(&row.seq) {
                "overridden"
            } else {
                "inherited"
            };
            let value = match values.get(row.seq) {
                Some(value) => format!("{:.6}", value.value),
                None => "n/a (suppressed)".to_string(),
            };
            let unit = row
                .unit
                .or_else(|| row.kind.canonical_unit())
                .map(|unit| unit.token())
                .unwrap_or("");
            diagnostics.push(Diagnostic {
                object: None,
                code: "param.row".into(),
                message: format!(
                    "  seq {}  {}  {} {}  = {}  ({status_token})",
                    row.seq.0,
                    row.name,
                    row.kind.token(),
                    unit,
                    value
                ),
            });
        }
        diagnostics.push(Diagnostic {
            object: None,
            code: "param.constraint_not_modeled".into(),
            message: "PARAMETERS: dimensional/geometric constraint parameters are solver-driven \
                      and are not modeled by this build; only the user-parameter table above \
                      exists"
                .into(),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

impl crate::command_payload::ParamOp {
    /// The undo label / transaction reason for this op.
    fn label(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::SetValue { .. } => "set",
            Self::Config { .. } => "config",
            Self::Delete { .. } => "delete",
            Self::Rename { .. } => "rename",
            Self::Bind { .. } => "bind",
            Self::Unbind { .. } => "unbind",
        }
    }

    /// Turn one op into the before/after patch lists (validated by the database).
    fn plan(
        &self,
        drawing: &DrawingDatabase,
        session: &SessionState,
        command: &Command,
        param_patches: &mut Vec<ParamPatch>,
        binding_patches: &mut Vec<ParamBindingPatch>,
    ) -> CadResult<()> {
        match self {
            Self::Create { name, kind, expr } => {
                let seq = match drawing.parameters().seqs().max() {
                    Some(seq) => ParamSeq(seq.0 + 1),
                    None => ParamSeq(1),
                };
                let row = ParamRow::new(seq, name.clone(), *kind, expr.clone()).map_err(to_cad)?;
                param_patches.push(ParamPatch {
                    seq,
                    before: None,
                    after: Some(row),
                });
            }
            Self::SetValue { seq, expr } => {
                let row = drawing.parameters().get(*seq).cloned().ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "param.unknown_reference: this document has no parameter {}",
                        seq.0
                    ))
                })?;
                // A base-row edit cannot change a value the active configuration
                // overrides or suppresses: the derived value comes from the
                // overlay. Refuse by name instead of reporting a write with no
                // visible effect (the same rule as the `param.<name>` property
                // write).
                let status = drawing.param_config_status()?;
                if let Some(config) = status.name.as_deref() {
                    if status.overridden.contains(seq) {
                        return Err(CadError::InvalidInput(format!(
                            "param.config_override: parameter {} is overridden by configuration \
                             {config}",
                            seq.0
                        )));
                    }
                    if status.suppressed.contains(seq) {
                        return Err(CadError::InvalidInput(format!(
                            "param.config_override: parameter {} is suppressed by configuration \
                             {config}",
                            seq.0
                        )));
                    }
                }
                let mut after = row.clone();
                after.expr = expr.clone();
                // Structural validation only (empty name, expression shape/budget);
                // the dimension and reference checks belong to the merged write
                // path, which evaluates the whole table before committing.
                after.validate().map_err(to_cad)?;
                param_patches.push(ParamPatch {
                    seq: *seq,
                    before: Some(row),
                    after: Some(after),
                });
            }
            Self::Config { .. } => {
                return Err(CadError::Invariant(
                    "a configuration op is handled before the patch planner".into(),
                ));
            }
            Self::Delete { seq } => {
                let row = drawing.parameters().get(*seq).cloned().ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "param.unknown_reference: this document has no parameter {}",
                        seq.0
                    ))
                })?;
                refuse_if_referenced(drawing, *seq)?;
                param_patches.push(ParamPatch {
                    seq: *seq,
                    before: Some(row),
                    after: None,
                });
            }
            Self::Rename { seq, name } => {
                let row = drawing.parameters().get(*seq).cloned().ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "param.unknown_reference: this document has no parameter {}",
                        seq.0
                    ))
                })?;
                let renamed = ParamRow {
                    seq: row.seq,
                    name: name.clone(),
                    kind: row.kind,
                    unit: row.unit,
                    expr: row.expr.clone(),
                };
                renamed.validate().map_err(to_cad)?;
                param_patches.push(ParamPatch {
                    seq: *seq,
                    before: Some(row),
                    after: Some(renamed),
                });
            }
            Self::Bind { slots } => {
                for reference in session.selection.refs() {
                    let id = reference.entity;
                    super::properties::ensure_addressable(drawing, command, reference)?;
                    let before = drawing.param_bindings(id).cloned();
                    let mut after = before.clone().unwrap_or_default();
                    for (slot, expr) in slots {
                        after.insert(slot.clone(), expr.clone()).map_err(to_cad)?;
                    }
                    push_binding_patch(binding_patches, drawing, id, before, after);
                }
            }
            Self::Unbind { slots } => {
                for reference in session.selection.refs() {
                    let id = reference.entity;
                    super::properties::ensure_addressable(drawing, command, reference)?;
                    let before = drawing.param_bindings(id).cloned();
                    let mut after = before.clone().unwrap_or_default();
                    for slot in slots {
                        after.remove(slot);
                    }
                    push_binding_patch(binding_patches, drawing, id, before, after);
                }
            }
        }
        Ok(())
    }
}

/// Normalise one binding patch: an empty result removes the block, and a patch
/// whose two sides are equal is dropped (no write, no undo row).
fn push_binding_patch(
    patches: &mut Vec<ParamBindingPatch>,
    drawing: &DrawingDatabase,
    id: EntityId,
    before: Option<ParamBindings>,
    after: ParamBindings,
) {
    let after = if after.is_empty() { None } else { Some(after) };
    if before == after {
        return;
    }
    // The side that `before` applied to is the geometry this element has now.
    // `after_geometry` is filled from the committed candidate after the write.
    let before_geometry = drawing.entity(id).map(|entity| entity.geometry.clone());
    patches.push(ParamBindingPatch {
        id,
        before,
        after,
        before_geometry,
        after_geometry: None,
    });
}

/// Refuse a delete whose target is still used by a binding or another row.
///
/// Nothing cascades: a parameter edit is one row, and dropping an edge another
/// expression depends on would silently change that expression's meaning.
fn refuse_if_referenced(drawing: &DrawingDatabase, seq: ParamSeq) -> CadResult<()> {
    for entity in drawing.entities() {
        let Some(bindings) = drawing.param_bindings(entity.id) else {
            continue;
        };
        for (slot, expr) in bindings.entries() {
            let mut references = std::collections::BTreeSet::new();
            expr.references(&mut references);
            if references.contains(&seq) {
                return Err(CadError::InvalidInput(format!(
                    "param.referenced: parameter {} is bound to {slot:?} on entity {}",
                    seq.0, entity.id.0
                )));
            }
        }
    }
    for row in drawing.parameters().rows() {
        if row.seq == seq {
            continue;
        }
        let mut references = std::collections::BTreeSet::new();
        row.expr.references(&mut references);
        if references.contains(&seq) {
            return Err(CadError::InvalidInput(format!(
                "param.referenced: parameter {} is referenced by parameter {}",
                seq.0, row.seq.0
            )));
        }
    }
    Ok(())
}

fn to_cad(error: param::ParamError) -> CadError {
    CadError::InvalidInput(format!("{}: {}", error.code(), error.detail()))
}

/// The one "nothing changed" outcome: no objects, no ChangeSet, no history.
fn unchanged_outcome(message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: "param.unchanged".into(),
            message,
        }],
        measurement: None,
        point_report: None,
    }
}
