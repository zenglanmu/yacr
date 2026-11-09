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
//! * this module — dispatch plus the three session-only group commands;
//! * `leader` — `QLEADER`/`LEADER`, which reuse the existing MLEADER create path;
//! * `textfit` — the glyph-height estimate;
//! * `hatch` — boundary rebuild and the origin refusal;
//! * `xdlist` — the read-only xdata report.
//!
//! The group store is [`crate::EntityGroups`] (bounded, session-only, never a
//! DWG object). Every drawing write in the other commands is one transaction /
//! one undo step, and every refusal happens before the transaction opens.

mod hatch;
mod leader;
mod textfit;
mod xdlist;

use crate::groups::EntityGroup;

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

/// The one-line report of a named group.
fn group_report(group: &EntityGroup, code: &str) -> Diagnostic {
    Diagnostic {
        object: None,
        code: code.into(),
        message: format!(
            "GROUP {}: {} member(s) (session-only; no DWG group object)",
            group.name,
            group.members.len()
        ),
    }
}

/// A direct-call group name guard (the session grammar already enforces it).
fn valid_group_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && !name.chars().any(char::is_whitespace)
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

    /// `GROUP` / `GROUP CREATE|LIST|DELETE <name>`.
    fn group_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = match &command.payload {
            CommandPayload::Annotation(payload) => match payload.as_ref() {
                AnnotationBatchPayload::Group(request) => Some(request.clone()),
                _ => {
                    return Err(invalid(
                        "group.payload",
                        "GROUP takes CREATE <name>, LIST or DELETE <name>",
                    ))
                }
            },
            // A bare `GROUP` reports the stored names (AutoCAD's default
            // `GROUP` prompt is a dialog; this build's default is the report).
            CommandPayload::None => None,
            _ => {
                return Err(invalid(
                    "group.payload",
                    "GROUP takes CREATE <name>, LIST or DELETE <name>",
                ))
            }
        };
        let drawing = drawing_of(self, command)?;
        match request {
            // Bare `GROUP` and `GROUP LIST` are the same read-only report.
            None | Some(GroupCommand::List) => {
                let mut diagnostics: Vec<Diagnostic> = session
                    .groups
                    .groups()
                    .map(|group| group_report(group, "group.entry"))
                    .collect();
                if diagnostics.is_empty() {
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "group.empty".into(),
                        message: "GROUP: no session-only groups are stored".into(),
                    });
                }
                diagnostics.push(Diagnostic {
                    object: None,
                    code: "group.list".into(),
                    message: format!(
                        "GROUP: {} of {} group(s) (cap {}; session-only, never written to the DWG)",
                        session.groups.len(),
                        session.groups.cap(),
                        session.groups.cap()
                    ),
                });
                Ok(report_outcome(diagnostics))
            }
            Some(GroupCommand::Create(name)) => {
                if !valid_group_name(&name) {
                    return Err(invalid(
                        "group.invalid_name",
                        format!("'{name}' is not a valid one-token group name"),
                    ));
                }
                if session.groups.contains(&name) {
                    return Err(invalid(
                        "group.duplicate",
                        format!(
                            "a group named '{name}' already exists (names are unique \
                             case-insensitively)"
                        ),
                    ));
                }
                if session.groups.len() >= session.groups.cap() {
                    return Err(invalid(
                        "group.cap",
                        format!(
                            "the session group store is full (cap {}); delete a group before \
                             creating another",
                            session.groups.cap()
                        ),
                    ));
                }
                if session.selection.is_empty() {
                    return Err(invalid(
                        "group.selection_required",
                        "GROUP CREATE captures the current selection; nothing is selected",
                    ));
                }
                let members = validate_refs(&drawing, command, session.selection.refs(), "group")?;
                let count = members.len();
                session.groups.create(&name, members);
                Ok(report_outcome(vec![Diagnostic {
                    object: None,
                    code: "group.created".into(),
                    message: format!(
                        "GROUP CREATE: captured {count} member(s) under '{name}' (session-only; \
                         no DWG group object, no transaction)"
                    ),
                }]))
            }
            Some(GroupCommand::Delete(name)) => {
                if !session.groups.remove(&name) {
                    return Err(invalid(
                        "group.unknown",
                        format!("no group named '{name}' exists in this session"),
                    ));
                }
                Ok(report_outcome(vec![Diagnostic {
                    object: None,
                    code: "group.deleted".into(),
                    message: format!(
                        "GROUP DELETE: removed the group '{name}'; every entity is retained \
                         (session-only)"
                    ),
                }]))
            }
        }
    }

    /// `UNGROUP <name>`: drop one group, keep every entity.
    fn ungroup_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let name = match &command.payload {
            CommandPayload::Annotation(payload) => match payload.as_ref() {
                AnnotationBatchPayload::Ungroup(name) => name.clone(),
                _ => return Err(invalid("ungroup.payload", "UNGROUP takes one group name")),
            },
            CommandPayload::None => {
                return Err(invalid(
                    "ungroup.needs_name",
                    "UNGROUP needs a group name (`UNGROUP <name>`)",
                ))
            }
            _ => return Err(invalid("ungroup.payload", "UNGROUP takes one group name")),
        };
        drawing_of(self, command)?;
        if !session.groups.remove(&name) {
            return Err(invalid(
                "ungroup.unknown",
                format!("no group named '{name}' exists in this session"),
            ));
        }
        Ok(report_outcome(vec![Diagnostic {
            object: None,
            code: "ungroup.removed".into(),
            message: format!(
                "UNGROUP: removed the group '{name}'; every entity is retained \
                 (session-only, no transaction)"
            ),
        }]))
    }

    /// `GROUPEDIT <name> ADD|REMOVE|RENAME <arg>`.
    fn group_edit_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = match &command.payload {
            CommandPayload::Annotation(payload) => match payload.as_ref() {
                AnnotationBatchPayload::GroupEdit(request) => request.clone(),
                _ => {
                    return Err(invalid(
                        "groupedit.payload",
                        "GROUPEDIT takes ADD, REMOVE or RENAME",
                    ))
                }
            },
            CommandPayload::None => {
                return Err(invalid(
                    "groupedit.needs_argument",
                    "GROUPEDIT needs `<name> ADD|REMOVE|RENAME <arg>`",
                ))
            }
            _ => {
                return Err(invalid(
                    "groupedit.payload",
                    "GROUPEDIT takes ADD, REMOVE or RENAME",
                ))
            }
        };
        let drawing = drawing_of(self, command)?;
        match request {
            GroupEditCommand::Add { name, entities } => {
                if session.groups.resolve(&name).is_none() {
                    return Err(invalid(
                        "groupedit.unknown",
                        format!("no group named '{name}' exists in this session"),
                    ));
                }
                let members = self.group_edit_members(session, command, &drawing, &entities)?;
                let added = session.groups.add_members(&name, &members).ok_or_else(|| {
                    CadError::Invariant("groupedit: the resolved group vanished".into())
                })?;
                if added == 0 {
                    return Ok(unchanged(
                        "groupedit.unchanged",
                        format!(
                            "GROUPEDIT ADD: every requested entity is already a member of \
                             '{name}' (no session change)"
                        ),
                    ));
                }
                Ok(report_outcome(vec![Diagnostic {
                    object: None,
                    code: "groupedit.added".into(),
                    message: format!(
                        "GROUPEDIT ADD: added {added} member(s) to '{name}' (session-only; no \
                         entity or database write)"
                    ),
                }]))
            }
            GroupEditCommand::Remove { name, entities } => {
                if session.groups.resolve(&name).is_none() {
                    return Err(invalid(
                        "groupedit.unknown",
                        format!("no group named '{name}' exists in this session"),
                    ));
                }
                let members = self.group_edit_members(session, command, &drawing, &entities)?;
                let removed = session
                    .groups
                    .remove_members(&name, &members)
                    .ok_or_else(|| {
                        CadError::Invariant("groupedit: the resolved group vanished".into())
                    })?;
                if removed == 0 {
                    return Ok(unchanged(
                        "groupedit.unchanged",
                        format!(
                            "GROUPEDIT REMOVE: none of the requested entities is a member of \
                             '{name}' (no session change)"
                        ),
                    ));
                }
                Ok(report_outcome(vec![Diagnostic {
                    object: None,
                    code: "groupedit.removed".into(),
                    message: format!(
                        "GROUPEDIT REMOVE: removed {removed} member(s) from '{name}'; the \
                         entities are retained (session-only)"
                    ),
                }]))
            }
            GroupEditCommand::Rename { name, new_name } => {
                if session.groups.resolve(&name).is_none() {
                    return Err(invalid(
                        "groupedit.unknown",
                        format!("no group named '{name}' exists in this session"),
                    ));
                }
                if !valid_group_name(&new_name) {
                    return Err(invalid(
                        "groupedit.invalid_name",
                        format!("'{new_name}' is not a valid one-token group name"),
                    ));
                }
                if session.groups.contains(&new_name)
                    && !new_name.trim().eq_ignore_ascii_case(name.trim())
                {
                    return Err(invalid(
                        "groupedit.rename_duplicate",
                        format!("a group named '{new_name}' already exists"),
                    ));
                }
                if name.trim() == new_name.trim() {
                    return Ok(unchanged(
                        "groupedit.unchanged",
                        format!("GROUPEDIT RENAME: '{name}' already has that name"),
                    ));
                }
                session.groups.rename(&name, &new_name);
                Ok(report_outcome(vec![Diagnostic {
                    object: None,
                    code: "groupedit.renamed".into(),
                    message: format!(
                        "GROUPEDIT RENAME: renamed '{name}' to '{new_name}'; membership is \
                         unchanged (session-only)"
                    ),
                }]))
            }
        }
    }

    /// The member refs of an `ADD`/`REMOVE`: current selection, or typed ids.
    fn group_edit_members(
        &self,
        session: &SessionState,
        command: &Command,
        drawing: &DrawingDatabase,
        entities: &[u128],
    ) -> CadResult<Vec<SelectionRef>> {
        if !session.selection.is_empty() {
            return validate_refs(drawing, command, session.selection.refs(), "groupedit");
        }
        if entities.is_empty() {
            return Err(invalid(
                "groupedit.selection_required",
                "GROUPEDIT ADD/REMOVE needs a non-empty selection or a typed entity id list",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut refs = Vec::with_capacity(entities.len());
        for id in entities {
            if *id == 0 {
                return Err(invalid(
                    "groupedit.invalid_entity",
                    "entity id 0 is not a valid entity",
                ));
            }
            if !seen.insert(*id) {
                return Err(invalid(
                    "groupedit.invalid_entity",
                    format!("entity id {id} is listed twice"),
                ));
            }
            let entity = drawing.entity(EntityId(*id)).ok_or_else(|| {
                invalid(
                    "groupedit.stale_ref",
                    format!("entity {id} does not exist in this drawing"),
                )
            })?;
            if entity.space != SpaceId::Model {
                return Err(invalid(
                    "groupedit.stale_ref",
                    format!(
                        "entity {id} is not a model-space entity; groups store model-space refs \
                         only"
                    ),
                ));
            }
            refs.push(SelectionRef {
                document: command.document,
                entity: EntityId(*id),
                instance: InstancePath::default(),
                sub_element: None,
            });
        }
        Ok(refs)
    }
}
