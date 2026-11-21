//! `SETENTITYPROPERTY` for one document parameter (`param.<name>`).
//!
//! The properties panel emits one dynamic row per parameter the selected
//! element's bindings directly reference ([`crate::selection_param`]). This
//! module is the write half: it is reached from the one property command,
//! **before** the display-property grammar, exactly like the dynamic BIM PSet
//! path, and [`crate::property_value`] never learns these keys.
//!
//! # One parameter, every selected element
//!
//! The key names a document parameter, not a per-element value. A multi-selection
//! can therefore write it only when **every** selected element has a binding
//! expression that directly references that parameter's sequence. One element
//! without such a binding is `param.mixed` and writes nothing: a parameter edit
//! with a partial target set would silently leave one object on its old value.
//!
//! # The edit is a table row, not an element value
//!
//! The draft becomes the parameter's new expression (`Expr::Num`); the elements
//! are re-derived by [`cad_db::DrawingDatabase::apply_parametric_update`], which
//! is the one merged write path (ADR-0043 D8). Writing the same number back is
//! `property.unchanged`: no transaction, no revision, no undo entry.
//!
//! # One revision, one undo step
//!
//! The parameter row and every re-derived element land in one transaction and
//! one [`cad_db::ChangeSet`], and the same transaction is recorded as one undo
//! step carrying the exact before/after parameter row
//! ([`cad_db::ParamPatch`]) — recorded **before** the candidate is published, so
//! a refused step leaves the open document and its history untouched. The
//! element geometry is not part of the record: it is derived from the restored
//! table on undo/redo (ADR-0043 D7, see
//! [`super::helpers::apply_drawing_patches`]).

use super::*;
use cad_db::{DrawingDatabase, ParamBindingPatch, ParamPatch, ParametricOutcome};
use cad_domain::param::{Expr, ParamSeq};
use cad_history::DrawingUndoRecord;

impl Application {
    /// Write one document parameter on every currently selected element.
    ///
    /// `name` is already parsed from the `param.<name>` key by the property
    /// command's prefix dispatch; the selection is read from the session at
    /// apply time, so a stale UI cannot retarget another element.
    pub(crate) fn set_param_property(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        name: &str,
        value: &str,
    ) -> CadResult<CommandOutcome> {
        let key = format!("{}{name}", crate::selection_param::KEY_PREFIX);
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        let references = session.selection.refs();
        if references.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "property.selection_empty: select at least one element to change {key}"
            )));
        }

        // Preflight the whole selection before anything is staged. The first
        // mixed element refuses the write; a partial write would leave the
        // remaining elements on an old value with no indication why.
        for reference in references {
            super::properties::ensure_addressable(drawing.as_ref(), command, reference)?;
        }

        let row = drawing
            .parameters()
            .get_by_name(name)
            .cloned()
            .ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "param.unknown_name: this document has no parameter named {name:?}"
                ))
            })?;

        // A base-row edit cannot change a value the active configuration
        // overrides or suppresses: the derived value comes from the overlay.
        // Refuse by name instead of reporting a successful write that has no
        // visible effect.
        let status = drawing.param_config_status()?;
        if let Some(config) = status.name.as_deref() {
            if status.overridden.contains(&row.seq) {
                return Err(CadError::InvalidInput(format!(
                    "param.config_override: {name} is overridden by configuration {config}"
                )));
            }
            if status.suppressed.contains(&row.seq) {
                return Err(CadError::InvalidInput(format!(
                    "param.config_override: {name} is suppressed by configuration {config}"
                )));
            }
        }

        for reference in references {
            if !binds_parameter(drawing.as_ref(), reference.entity, row.seq) {
                return Err(CadError::InvalidInput(format!(
                    "param.mixed: {name} is not bound on every selected element \
                     (entity {} does not reference it)",
                    reference.entity.0
                )));
            }
        }

        let number = parse_draft(&key, value)?;
        let next_expr = Expr::Num(number);
        if row.expr == next_expr {
            return Ok(unchanged_outcome(format!(
                "SETENTITYPROPERTY: {key} already has the requested value"
            )));
        }
        let mut after = row.clone();
        after.expr = next_expr;
        let before_row = row.clone();

        let transaction = next_transaction_id();
        let reason = format!("set parameter {name}");
        let mut candidate = drawing.as_ref().clone();
        let outcome = candidate.apply_parametric_update(
            &reason,
            transaction,
            &[ParamPatch {
                seq: row.seq,
                before: Some(row),
                after: Some(after),
            }],
            &[],
        )?;

        // A batch that changed neither the table nor a binding is not a write.
        // `ChangeSet::is_empty` only counts object changes, so it would call a
        // table-only commit (a parameter whose derived geometry failed) unchanged
        // and lose the edit. The revision is the honest no-op test: the database
        // raises it by exactly one for any content change and not at all
        // otherwise.
        if outcome.changes.after == outcome.changes.before {
            return Ok(unchanged_outcome(format!(
                "SETENTITYPROPERTY: {key} already has the requested value"
            )));
        }

        // The patch carries the stored rows, never a re-derivation of the draft:
        // `before` is the row this command replaced and `after` is what the
        // candidate actually committed.
        let after_row = candidate
            .parameters()
            .get_by_name(name)
            .expect("the parameter was just committed")
            .clone();
        let patches = vec![ParamPatch {
            seq: after_row.seq,
            before: Some(before_row),
            after: Some(after_row),
        }];
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
            patches,
            Vec::new(),
        )?;

        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(changes)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// [`Self::publish_candidate`] for a step whose rows are parameter/binding
    /// patches.
    ///
    /// The patches are validated and recorded **with** the record, before the
    /// candidate is published, so a refused step leaves the open document, its
    /// revision and history untouched. A parametric step may carry the record's
    /// entity patches (ADR-0043 D8) but this write path derives geometry from the
    /// restored table instead, so it records none.
    pub(crate) fn publish_candidate_with_params(
        &mut self,
        document_id: DocumentId,
        candidate: DrawingDatabase,
        record: DrawingUndoRecord,
        param_patches: Vec<ParamPatch>,
        param_binding_patches: Vec<ParamBindingPatch>,
    ) -> CadResult<()> {
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        self.history
            .entry(document_id)
            .or_default()
            .record_drawing_with_params(record, param_patches, param_binding_patches)?;
        document.drawing = Arc::new(candidate);
        Ok(())
    }
}

/// Whether one element has a binding expression that directly references `seq`.
fn binds_parameter(drawing: &DrawingDatabase, id: EntityId, seq: ParamSeq) -> bool {
    let Some(bindings) = drawing.param_bindings(id) else {
        return false;
    };
    for (_, expr) in bindings.entries() {
        let mut references = std::collections::BTreeSet::new();
        expr.references(&mut references);
        if references.contains(&seq) {
            return true;
        }
    }
    false
}

/// Parse the draft as a finite decimal number.
///
/// The whole string is the number (no unit suffix): a parameter's canonical unit
/// comes from its kind and is never converted here. Empty, non-numeric and
/// non-finite drafts are `property.invalid_value`, never clamped.
fn parse_draft(key: &str, draft: &str) -> CadResult<f64> {
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        return Err(CadError::InvalidInput(format!(
            "property.invalid_value: {key} takes a number, not an empty value"
        )));
    }
    let number = trimmed.parse::<f64>().map_err(|_| {
        CadError::InvalidInput(format!(
            "property.invalid_value: {key} takes a number, not {draft:?}"
        ))
    })?;
    if !number.is_finite() {
        return Err(CadError::InvalidInput(format!(
            "property.invalid_value: {key} takes a finite number, not {draft:?}"
        )));
    }
    Ok(number)
}

/// The one "nothing changed" outcome: no objects, no ChangeSet, no history.
fn unchanged_outcome(message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: "property.unchanged".into(),
            message,
        }],
        measurement: None,
        point_report: None,
    }
}
