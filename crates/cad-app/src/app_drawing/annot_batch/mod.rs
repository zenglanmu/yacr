//! The 2026-11-07 annotation/group batch: `GROUP`/`-GROUP`, `UNGROUP`,
//! `GROUPEDIT`, `QLEADER`, `LEADER`, `TEXTFIT`, `HATCHSETBOUNDARY`,
//! `HATCHSETORIGIN` and `XDLIST`.
//!
//! The full contract, including every declared boundary and the two deliberate
//! refusals (`HATCHSETORIGIN`'s missing origin field, `XDLIST`'s non-retained
//! xdata), lives in [`crate::command_id::annot_batch`].
//!
//! Split into submodules by subject so each stays inside the code-size gate:
//!
//! * this module — dispatch plus the shared refusal/report helpers;
//! * `group` — the persisted `GROUP`/`UNGROUP`/`GROUPEDIT` commands;
//! * `leader` — `QLEADER`/`LEADER`, which reuse the existing MLEADER create path;
//! * `textfit` — the glyph-height estimate;
//! * `hatch` — boundary rebuild and the origin refusal;
//! * `xdlist` — the read-only xdata report.
//!
//! Groups are document data (`tables/groups.json`); every group and drawing write
//! is one transaction / one undo step, and every refusal happens before the
//! transaction opens.

mod group;
mod hatch;
pub(crate) mod leader;
mod textfit;
mod xdlist;

// `HATCHEDIT ADDBOUNDARIES`/`REMOVEBOUNDARIES` convert selected closed entities
// into hatch-plane loops with the same projection/unprojection rules as
// `HATCHSETBOUNDARY`; share that one implementation instead of copying it.
pub(crate) use hatch::boundary_loop;

use super::*;

/// Whether `id` belongs to this batch (the dispatch predicate in
/// `application.rs`).
pub(crate) fn batch(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::Group
            | CommandId::Ungroup
            | CommandId::GroupEdit
            | CommandId::QLeader
            | CommandId::Leader
            | CommandId::TextFit
            | CommandId::HatchSetBoundary
            | CommandId::HatchSetOrigin
            | CommandId::XdList
    )
}

/// A `<command>.<code>` refusal.
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// A named no-op/read-only report outcome (no `ChangeSet`, no history).
fn report_outcome(diagnostics: Vec<Diagnostic>) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics,
        measurement: None,
        point_report: None,
    }
}

fn unchanged(code: &str, message: String) -> CommandOutcome {
    report_outcome(vec![Diagnostic {
        object: None,
        code: code.into(),
        message,
    }])
}

/// Commit one entity replacement in exactly one transaction / one undo step.
///
/// The caller owns every refusal and has already built `candidate` (a clone of
/// the document's drawing) and the `after` row. The committed row is read back so
/// the undo patch stores what the database really holds.
fn commit_one_entity(
    app: &mut Application,
    document_id: DocumentId,
    mut candidate: DrawingDatabase,
    before: DbEntity,
    after: DbEntity,
    label: &str,
    message: String,
) -> CadResult<CommandOutcome> {
    let entity_id = before.id;
    let transaction = next_transaction_id();
    let change_set = {
        let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
        staged.update_entity(after)?;
        staged.commit()?
    };
    let stored = candidate.entity(entity_id).cloned().ok_or_else(|| {
        CadError::Invariant(format!("{label}: the updated entity vanished after commit"))
    })?;
    app.publish_candidate(
        document_id,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: label.to_string(),
            patches: vec![drawing_patch(entity_id, Some(before.clone()), Some(stored))],
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            style_patches: Vec::new(),
            dim_style_patches: Vec::new(),
            mleader_style_patches: Vec::new(),
            layout_patches: Vec::new(),
            merge_key: None,
        },
    )?;
    Ok(CommandOutcome {
        objects: vec![ObjectId(entity_id.0)],
        changes: Some(Arc::new(change_set)),
        diagnostics: vec![Diagnostic {
            object: Some(ObjectId(entity_id.0)),
            code: format!("{label}.applied"),
            message,
        }],
        measurement: None,
        point_report: None,
    })
}

/// Require the command's document to be open (the ordinary stale-document
/// guard), returning its immutable drawing.
fn drawing_of(app: &Application, command: &Command) -> CadResult<Arc<DrawingDatabase>> {
    let document = app
        .workspace
        .documents
        .get(&command.document)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    if document.id != command.document {
        return Err(CadError::StaleResult);
    }
    Ok(document.drawing.clone())
}

/// Resolve every selection ref against the drawing, refusing a stale ref.
fn validate_refs(
    drawing: &DrawingDatabase,
    command: &Command,
    references: &[SelectionRef],
    prefix: &str,
) -> CadResult<Vec<SelectionRef>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut resolved = Vec::with_capacity(references.len());
    for reference in references {
        if reference.document != command.document {
            return Err(invalid(
                &format!("{prefix}.stale_ref"),
                format!(
                    "the selected ref for entity {} belongs to another document",
                    reference.entity.0
                ),
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(invalid(
                &format!("{prefix}.stale_ref"),
                format!(
                    "entity {} is a block-instance member or sub-element; groups store only \
                     directly addressable entity refs",
                    reference.entity.0
                ),
            ));
        }
        if drawing.entity(reference.entity).is_none() {
            return Err(invalid(
                &format!("{prefix}.stale_ref"),
                format!("entity {} no longer exists", reference.entity.0),
            ));
        }
        if !seen.insert(reference.entity) {
            return Err(invalid(
                &format!("{prefix}.stale_ref"),
                format!("entity {} is selected twice", reference.entity.0),
            ));
        }
        resolved.push(reference.clone());
    }
    Ok(resolved)
}

impl Application {
    /// Dispatch one command of this batch.
    pub(crate) fn annot_batch(
        &mut self,
        session: &mut SessionState,
        id: CommandId,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match id {
            CommandId::Group => self.group_command(session, command),
            CommandId::Ungroup => self.ungroup_command(session, command),
            CommandId::GroupEdit => self.group_edit_command(session, command),
            CommandId::QLeader | CommandId::Leader => self.leader_command(session, command, id),
            CommandId::TextFit => self.text_fit(session, command),
            CommandId::HatchSetBoundary => self.hatch_set_boundary(session, command),
            CommandId::HatchSetOrigin => self.hatch_set_origin(session, command),
            CommandId::XdList => self.xd_list(session, command),
            _ => Err(CadError::Invariant(format!(
                "{id:?} is not an annotation-batch command"
            ))),
        }
    }
}
