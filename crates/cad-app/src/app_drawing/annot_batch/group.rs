//! Persisted group commands: `GROUP` / `-GROUP`, `UNGROUP`, `GROUPEDIT`.
//!
//! The authoritative table is [`cad_db::DrawingDatabase::groups`]
//! (`tables/groups.json`). Every mutation:
//!
//! * validates the whole request against the document before touching anything;
//! * clones the drawing, applies one [`cad_db::GroupPatch`] batch through
//!   [`cad_db::DrawingDatabase::apply_group_changes`] (one revision, one
//!   [`cad_db::ChangeSet`], `ChangeScope::tables = {Groups}`), and only then
//!   publishes the candidate together with one undo record;
//! * records no history and bumps no revision for a read-only report or a
//!   named refusal.
//!
//! No command reads a session group store: `SessionState::textmask_groups` is
//! TEXTMASK's private text/mask association and is deliberately not consulted
//! here.

use cad_db::{GroupPatch, GroupRow};

use super::*;

/// Most user groups a document may carry through the commands.
///
/// This is a command-layer bound (the format itself has no such limit, so a
/// DXF-imported drawing is never truncated). Creating one more is a named
/// refusal, never a silent drop of an older group.
pub(crate) const GROUP_CAP: usize = crate::ENTITY_GROUP_CAP;

/// One group line as reported by `GROUP LIST`.
fn group_report(group: &GroupRow, code: &str) -> Diagnostic {
    Diagnostic {
        object: None,
        code: code.into(),
        message: format!(
            "GROUP {}: {} member(s) (persisted in tables/groups.json; exported as AcDbGroup)",
            group.name,
            group.members.len()
        ),
    }
}

/// A direct-call group name guard (the typed grammar already enforces it).
fn valid_group_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && !name.chars().any(char::is_whitespace)
}

impl Application {
    /// Apply one group batch as one revision and record it as one undo step.
    ///
    /// The candidate is published only after [`cad_db::DrawingDatabase::apply_group_changes`]
    /// accepted the whole batch, so a refused edit leaves the open document, its
    /// revision and history untouched.
    fn commit_group_change(
        &mut self,
        document_id: DocumentId,
        drawing: &DrawingDatabase,
        label: &str,
        patches: Vec<GroupPatch>,
    ) -> CadResult<cad_db::ChangeSet> {
        let mut candidate = drawing.clone();
        let transaction = next_transaction_id();
        let changes = candidate.apply_group_changes(label, transaction, &patches)?;
        if changes.after == changes.before {
            return Err(CadError::Invariant(format!(
                "{label}: the group batch produced no revision"
            )));
        }
        let record = DrawingUndoRecord {
            transaction: changes.transaction,
            label: label.to_string(),
            patches: Vec::new(),
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            layout_patches: Vec::new(),
            style_patches: Vec::new(),
            dim_style_patches: Vec::new(),
            mleader_style_patches: Vec::new(),
            merge_key: None,
        };
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        self.history
            .entry(document_id)
            .or_default()
            .record_drawing_with_groups(record, patches)?;
        document.drawing = std::sync::Arc::new(candidate);
        Ok(changes)
    }

    /// `GROUP` / `GROUP CREATE|LIST|DELETE <name>`.
    pub(crate) fn group_command(
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
                let mut diagnostics: Vec<Diagnostic> = drawing
                    .groups()
                    .map(|group| group_report(group, "group.entry"))
                    .collect();
                if diagnostics.is_empty() {
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "group.empty".into(),
                        message: "GROUP: the drawing stores no group".into(),
                    });
                }
                diagnostics.push(Diagnostic {
                    object: None,
                    code: "group.list".into(),
                    message: format!(
                        "GROUP: {} of {} group(s) (persisted in tables/groups.json)",
                        drawing.groups().count(),
                        GROUP_CAP
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
                if drawing.group_name_taken(&name) {
                    return Err(invalid(
                        "group.duplicate",
                        format!(
                            "a group named '{name}' already exists (names are unique \
                             case-insensitively)"
                        ),
                    ));
                }
                if drawing.groups().count() >= GROUP_CAP {
                    return Err(invalid(
                        "group.cap",
                        format!(
                            "the drawing holds {GROUP_CAP} groups; delete a group before \
                             creating another"
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
                let entity_ids: Vec<cad_domain::EntityId> =
                    members.iter().map(|member| member.entity).collect();
                let count = entity_ids.len();
                let seq = drawing.next_group_seq();
                let row = GroupRow::new(seq, name.as_str(), entity_ids);
                let label = format!("group create {name}");
                let changes = self.commit_group_change(
                    command.document,
                    &drawing,
                    &label,
                    vec![GroupPatch {
                        seq,
                        before: None,
                        after: Some(row),
                    }],
                )?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(std::sync::Arc::new(changes)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "group.created".into(),
                        message: format!(
                            "GROUP CREATE: captured {count} member(s) under '{name}' \
                             (persisted; one transaction, one undo step)"
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            Some(GroupCommand::Delete(name)) => {
                let group = drawing.group_by_name(&name).ok_or_else(|| {
                    invalid(
                        "group.unknown",
                        format!("no group named '{name}' exists in this drawing"),
                    )
                })?;
                let seq = group.seq;
                let row = group.clone();
                let label = format!("group delete {}", row.name);
                let changes = self.commit_group_change(
                    command.document,
                    &drawing,
                    &label,
                    vec![GroupPatch {
                        seq,
                        before: Some(row),
                        after: None,
                    }],
                )?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(std::sync::Arc::new(changes)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "group.deleted".into(),
                        message: format!(
                            "GROUP DELETE: removed the group '{name}'; every entity is retained \
                             (one transaction, one undo step)"
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
        }
    }

    /// `UNGROUP <name>`: drop one group, keep every entity.
    pub(crate) fn ungroup_command(
        &mut self,
        _session: &mut SessionState,
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
        let drawing = drawing_of(self, command)?;
        let group = drawing.group_by_name(&name).ok_or_else(|| {
            invalid(
                "ungroup.unknown",
                format!("no group named '{name}' exists in this drawing"),
            )
        })?;
        let seq = group.seq;
        let row = group.clone();
        let label = format!("ungroup {}", row.name);
        let changes = self.commit_group_change(
            command.document,
            &drawing,
            &label,
            vec![GroupPatch {
                seq,
                before: Some(row),
                after: None,
            }],
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(std::sync::Arc::new(changes)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "ungroup.removed".into(),
                message: format!(
                    "UNGROUP: removed the group '{name}'; every entity is retained \
                     (one transaction, one undo step)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `GROUPEDIT <name> ADD|REMOVE|RENAME <arg>`.
    pub(crate) fn group_edit_command(
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
                let group = drawing.group_by_name(&name).ok_or_else(|| {
                    invalid(
                        "groupedit.unknown",
                        format!("no group named '{name}' exists in this drawing"),
                    )
                })?;
                let before = group.clone();
                let members = self.group_edit_members(session, command, &drawing, &entities)?;
                let mut after = before.clone();
                let mut added = 0usize;
                for member in members {
                    if !after.members.contains(&member.entity) {
                        after.members.push(member.entity);
                        added += 1;
                    }
                }
                if added == 0 {
                    return Ok(unchanged(
                        "groupedit.unchanged",
                        format!(
                            "GROUPEDIT ADD: every requested entity is already a member of \
                             '{name}' (no document change)"
                        ),
                    ));
                }
                let label = format!("groupedit add {name}");
                let changes = self.commit_group_change(
                    command.document,
                    &drawing,
                    &label,
                    vec![GroupPatch {
                        seq: before.seq,
                        before: Some(before),
                        after: Some(after),
                    }],
                )?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(std::sync::Arc::new(changes)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "groupedit.added".into(),
                        message: format!(
                            "GROUPEDIT ADD: added {added} member(s) to '{name}' (one \
                             transaction, one undo step)"
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            GroupEditCommand::Remove { name, entities } => {
                let group = drawing.group_by_name(&name).ok_or_else(|| {
                    invalid(
                        "groupedit.unknown",
                        format!("no group named '{name}' exists in this drawing"),
                    )
                })?;
                let before = group.clone();
                let members = self.group_edit_members(session, command, &drawing, &entities)?;
                let drop_ids: std::collections::BTreeSet<cad_domain::EntityId> =
                    members.iter().map(|member| member.entity).collect();
                let mut after = before.clone();
                let original = after.members.len();
                after.members.retain(|member| !drop_ids.contains(member));
                let removed = original - after.members.len();
                if removed == 0 {
                    return Ok(unchanged(
                        "groupedit.unchanged",
                        format!(
                            "GROUPEDIT REMOVE: none of the requested entities is a member of \
                             '{name}' (no document change)"
                        ),
                    ));
                }
                let label = format!("groupedit remove {name}");
                let changes = self.commit_group_change(
                    command.document,
                    &drawing,
                    &label,
                    vec![GroupPatch {
                        seq: before.seq,
                        before: Some(before),
                        after: Some(after),
                    }],
                )?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(std::sync::Arc::new(changes)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "groupedit.removed".into(),
                        message: format!(
                            "GROUPEDIT REMOVE: removed {removed} member(s) from '{name}'; the \
                             entities are retained (one transaction, one undo step)"
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            GroupEditCommand::Rename { name, new_name } => {
                let group = drawing.group_by_name(&name).ok_or_else(|| {
                    invalid(
                        "groupedit.unknown",
                        format!("no group named '{name}' exists in this drawing"),
                    )
                })?;
                if !valid_group_name(&new_name) {
                    return Err(invalid(
                        "groupedit.invalid_name",
                        format!("'{new_name}' is not a valid one-token group name"),
                    ));
                }
                if drawing.group_name_taken(&new_name)
                    && !new_name.trim().eq_ignore_ascii_case(group.name.trim())
                {
                    return Err(invalid(
                        "groupedit.rename_duplicate",
                        format!("a group named '{new_name}' already exists"),
                    ));
                }
                if group.name.trim() == new_name.trim() {
                    return Ok(unchanged(
                        "groupedit.unchanged",
                        format!("GROUPEDIT RENAME: '{name}' already has that name"),
                    ));
                }
                let before = group.clone();
                let mut after = before.clone();
                after.name = new_name.trim().to_string();
                let label = format!("groupedit rename {name}");
                let changes = self.commit_group_change(
                    command.document,
                    &drawing,
                    &label,
                    vec![GroupPatch {
                        seq: before.seq,
                        before: Some(before),
                        after: Some(after),
                    }],
                )?;
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: Some(std::sync::Arc::new(changes)),
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "groupedit.renamed".into(),
                        message: format!(
                            "GROUPEDIT RENAME: renamed '{name}' to '{new_name}'; membership is \
                             unchanged (one transaction, one undo step)"
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
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
