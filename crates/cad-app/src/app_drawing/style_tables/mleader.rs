//! The `MLEADERSTYLE` table operations (`NEW`/`RENAME`/`DELETE`/`MODIFY`/`SET`).
//!
//! The field grammar lives in `style_tables::fields`; `SET` writes the session
//!
//! Split out of `style_tables.rs` for the code-size gate; the dispatch and the
//! shared commit/name-resolution helpers stay in the parent module.

use super::*;

impl Application {
    pub(super) fn mleader_style_table_op(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        op: StyleTableOp,
    ) -> CadResult<CommandOutcome> {
        let drawing = document_drawing(self, command)?;
        match op {
            StyleTableOp::Set { name } => {
                let id = resolve_unique_mleader_style(&drawing, &name)?;
                let row = drawing
                    .mleader_style(id)
                    .ok_or_else(|| mleader_style_unknown(&name))?;
                let stored = row.name.clone();
                session.current_mleader_style = stored.clone();
                Ok(report_outcome(vec![diagnostic(
                    "mleaderstyle.set",
                    format!(
                        "MLEADERSTYLE SET: the current multileader style is now '{stored}' (id \
                     {}); session state only, MLEADER creation resolves this row",
                        id.0
                    ),
                )]))
            }
            StyleTableOp::Create { name, from } => {
                let base = match from.as_deref() {
                    Some(base) => {
                        let id = resolve_unique_mleader_style(&drawing, base)?;
                        drawing
                            .mleader_style(id)
                            .cloned()
                            .ok_or_else(|| mleader_style_unknown(base))?
                    }
                    None => MLeaderStyle::standard_metric(MLeaderStyleId(0)),
                };
                let id = drawing.next_mleader_style_id();
                let mut row = base;
                row.id = id;
                row.name = name.clone();
                session.current_mleader_style = name.trim().to_string();
                self.commit_style_write(command, "mleaderstyle create", |candidate, transaction| {
                    let changes = [(id, Some(row.clone()))];
                    let change_set = candidate.apply_style_table_changes(
                        "mleaderstyle create",
                        transaction,
                        StyleTableChanges {
                            mleader_styles: &changes,
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        change_set,
                        Vec::new(),
                        Vec::new(),
                        vec![cad_history::mleader_style_patch(
                            id,
                            None,
                            candidate.mleader_style(id).cloned(),
                        )],
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "mleaderstyle.created",
                        format!(
                        "MLEADERSTYLE NEW: created multileader style '{name}' (id {}) and made \
                         it current; one transaction / one undo step",
                        id.0
                    ),
                    ));
                    outcome
                })
            }
            StyleTableOp::Rename { old, new } => {
                let id = resolve_unique_mleader_style(&drawing, &old)?;
                let before = drawing
                    .mleader_style(id)
                    .cloned()
                    .ok_or_else(|| mleader_style_unknown(&old))?;
                if before.name == new.trim() {
                    return Ok(report_outcome(vec![diagnostic(
                        "mleaderstyle.renamed",
                        format!(
                            "MLEADERSTYLE RENAME: '{}' already has that name; nothing changed",
                            before.name
                        ),
                    )]));
                }
                let mut after = before.clone();
                after.name = new;
                let stored_name = after.name.trim().to_string();
                if session
                    .current_mleader_style
                    .eq_ignore_ascii_case(before.name.trim())
                {
                    session.current_mleader_style = stored_name;
                }
                self.commit_style_write(command, "mleaderstyle rename", |candidate, transaction| {
                    let changes = [(id, Some(after.clone()))];
                    let change_set = candidate.apply_style_table_changes(
                        "mleaderstyle rename",
                        transaction,
                        StyleTableChanges {
                            mleader_styles: &changes,
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        change_set,
                        Vec::new(),
                        Vec::new(),
                        vec![cad_history::mleader_style_patch(
                            id,
                            Some(before.clone()),
                            candidate.mleader_style(id).cloned(),
                        )],
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "mleaderstyle.renamed",
                        format!(
                        "MLEADERSTYLE RENAME: renamed '{}' to '{}' (one transaction / one undo \
                         step)",
                        before.name, after.name.trim()
                    ),
                    ));
                    outcome
                })
            }
            StyleTableOp::Delete { name } => {
                let id = resolve_unique_mleader_style(&drawing, &name)?;
                let before = drawing
                    .mleader_style(id)
                    .cloned()
                    .ok_or_else(|| mleader_style_unknown(&name))?;
                if session
                    .current_mleader_style
                    .eq_ignore_ascii_case(before.name.trim())
                {
                    session.current_mleader_style = "Standard".to_string();
                }
                self.commit_style_write(command, "mleaderstyle delete", |candidate, transaction| {
                    let changes = [(id, None)];
                    let change_set = candidate.apply_style_table_changes(
                        "mleaderstyle delete",
                        transaction,
                        StyleTableChanges {
                            mleader_styles: &changes,
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        change_set,
                        Vec::new(),
                        Vec::new(),
                        vec![cad_history::mleader_style_patch(
                            id,
                            Some(before.clone()),
                            None,
                        )],
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "mleaderstyle.deleted",
                        format!(
                        "MLEADERSTYLE DELETE: deleted multileader style '{}' (one transaction / \
                         one undo step)",
                        before.name
                    ),
                    ));
                    outcome
                })
            }
            StyleTableOp::Modify { name, assignments } => {
                let id = resolve_unique_mleader_style(&drawing, &name)?;
                let before = drawing
                    .mleader_style(id)
                    .cloned()
                    .ok_or_else(|| mleader_style_unknown(&name))?;
                let mut after = before.clone();
                let mut renamed = None;
                for (field, value) in &assignments {
                    if field.eq_ignore_ascii_case("name") {
                        if value.trim() != before.name {
                            renamed = Some((before.name.clone(), value.clone()));
                        }
                        after.name = value.clone();
                        continue;
                    }
                    fields::apply_mleader_style_field(&mut after, field, value, |text| {
                        drawing.style_by_name(text).is_some()
                    })
                    .map_err(|reason| CadError::InvalidInput(reason))?;
                }
                if after == before {
                    return Ok(report_outcome(vec![diagnostic(
                        "mleaderstyle.unchanged",
                        format!(
                            "MLEADERSTYLE MODIFY: '{}' already has those values; nothing changed",
                            before.name
                        ),
                    )]));
                }
                if let Some((old, new)) = &renamed {
                    if session
                        .current_mleader_style
                        .eq_ignore_ascii_case(old.trim())
                    {
                        session.current_mleader_style = new.trim().to_string();
                    }
                }
                self.commit_style_write(command, "mleaderstyle modify", |candidate, transaction| {
                    let changes = [(id, Some(after.clone()))];
                    let change_set = candidate.apply_style_table_changes(
                        "mleaderstyle modify",
                        transaction,
                        StyleTableChanges {
                            mleader_styles: &changes,
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        change_set,
                        Vec::new(),
                        Vec::new(),
                        vec![cad_history::mleader_style_patch(
                            id,
                            Some(before.clone()),
                            candidate.mleader_style(id).cloned(),
                        )],
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "mleaderstyle.modified",
                        format!(
                        "MLEADERSTYLE MODIFY: updated '{}' ({} field(s)); existing multileaders \
                         keep their baked snapshot",
                        before.name,
                        assignments.len()
                    ),
                    ));
                    outcome
                })
            }
            StyleTableOp::List => unreachable!("the list form is dispatched by the report path"),
        }
    }
}
