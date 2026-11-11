//! The `DIMSTYLE` table operations (`NEW`/`RENAME`/`DELETE`/`MODIFY`/`SET`).
//!
//! The field grammar lives in `style_tables::fields`; `SET` writes the session
//!
//! Split out of `style_tables.rs` for the code-size gate; the dispatch and the
//! shared commit/name-resolution helpers stay in the parent module.

use super::*;

impl Application {
    pub(super) fn dim_style_table_op(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        op: StyleTableOp,
    ) -> CadResult<CommandOutcome> {
        let drawing = document_drawing(self, command)?;
        match op {
            StyleTableOp::Set { name } => {
                let id = resolve_unique_dim_style(&drawing, &name)?;
                let row = drawing
                    .dim_style(id)
                    .ok_or_else(|| dim_style_unknown(&name))?;
                let stored = row.name.clone();
                session.current_dim_style = stored.clone();
                Ok(report_outcome(vec![diagnostic(
                    "dimstyle.set",
                    format!(
                        "DIMSTYLE SET: the current dimension style is now '{stored}' (id {}); \
                     session state only, creation resolves this row",
                        id.0
                    ),
                )]))
            }
            StyleTableOp::Create { name, from } => {
                let base = match from.as_deref() {
                    Some(base) => {
                        let id = resolve_unique_dim_style(&drawing, base)?;
                        drawing
                            .dim_style(id)
                            .cloned()
                            .ok_or_else(|| dim_style_unknown(base))?
                    }
                    None => DimStyle::standard_metric(DimStyleId(0)),
                };
                let id = drawing.next_dim_style_id();
                let mut row = base;
                row.id = id;
                row.name = name.clone();
                session.current_dim_style = name.trim().to_string();
                self.commit_style_write(command, "dimstyle create", |candidate, transaction| {
                    let changes = [(id, Some(row.clone()))];
                    let change_set = candidate.apply_style_table_changes(
                        "dimstyle create",
                        transaction,
                        StyleTableChanges {
                            dim_styles: &changes,
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        change_set,
                        Vec::new(),
                        vec![cad_history::dim_style_patch(
                            id,
                            None,
                            candidate.dim_style(id).cloned(),
                        )],
                        Vec::new(),
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "dimstyle.created",
                        format!(
                            "DIMSTYLE NEW: created dimension style '{name}' (id {}) and made it \
                         current; one transaction / one undo step",
                            id.0
                        ),
                    ));
                    outcome
                })
            }
            StyleTableOp::Rename { old, new } => {
                let id = resolve_unique_dim_style(&drawing, &old)?;
                let before = drawing
                    .dim_style(id)
                    .cloned()
                    .ok_or_else(|| dim_style_unknown(&old))?;
                if before.name == new.trim() {
                    return Ok(report_outcome(vec![diagnostic(
                        "dimstyle.renamed",
                        format!(
                            "DIMSTYLE RENAME: '{}' already has that name; nothing changed",
                            before.name
                        ),
                    )]));
                }
                let mut after = before.clone();
                after.name = new;
                let stored_name = after.name.trim().to_string();
                if session
                    .current_dim_style
                    .eq_ignore_ascii_case(before.name.trim())
                {
                    session.current_dim_style = stored_name.clone();
                }
                self.commit_style_write(command, "dimstyle rename", |candidate, transaction| {
                    let changes = [(id, Some(after.clone()))];
                    let change_set = candidate.apply_style_table_changes(
                        "dimstyle rename",
                        transaction,
                        StyleTableChanges {
                            dim_styles: &changes,
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        change_set,
                        Vec::new(),
                        vec![cad_history::dim_style_patch(
                            id,
                            Some(before.clone()),
                            candidate.dim_style(id).cloned(),
                        )],
                        Vec::new(),
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "dimstyle.renamed",
                        format!(
                            "DIMSTYLE RENAME: renamed '{}' to '{}' (one transaction / one undo \
                         step)",
                            before.name,
                            after.name.trim()
                        ),
                    ));
                    outcome
                })
            }
            StyleTableOp::Delete { name } => {
                let id = resolve_unique_dim_style(&drawing, &name)?;
                let before = drawing
                    .dim_style(id)
                    .cloned()
                    .ok_or_else(|| dim_style_unknown(&name))?;
                if session
                    .current_dim_style
                    .eq_ignore_ascii_case(before.name.trim())
                {
                    session.current_dim_style = "Standard".to_string();
                }
                self.commit_style_write(command, "dimstyle delete", |candidate, transaction| {
                    let changes = [(id, None)];
                    let change_set = candidate.apply_style_table_changes(
                        "dimstyle delete",
                        transaction,
                        StyleTableChanges {
                            dim_styles: &changes,
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        change_set,
                        Vec::new(),
                        vec![cad_history::dim_style_patch(id, Some(before.clone()), None)],
                        Vec::new(),
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "dimstyle.deleted",
                        format!(
                            "DIMSTYLE DELETE: deleted dimension style '{}' (one transaction / one \
                         undo step)",
                            before.name
                        ),
                    ));
                    outcome
                })
            }
            StyleTableOp::Modify { name, assignments } => {
                let id = resolve_unique_dim_style(&drawing, &name)?;
                let before = drawing
                    .dim_style(id)
                    .cloned()
                    .ok_or_else(|| dim_style_unknown(&name))?;
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
                    fields::apply_dim_style_field(&mut after, field, value, |text| {
                        drawing.style_by_name(text).is_some()
                    })
                    .map_err(CadError::InvalidInput)?;
                }
                if after == before {
                    return Ok(report_outcome(vec![diagnostic(
                        "dimstyle.unchanged",
                        format!(
                            "DIMSTYLE MODIFY: '{}' already has those values; nothing changed",
                            before.name
                        ),
                    )]));
                }
                if let Some((old, new)) = &renamed {
                    if session.current_dim_style.eq_ignore_ascii_case(old.trim()) {
                        session.current_dim_style = new.trim().to_string();
                    }
                }
                self.commit_style_write(command, "dimstyle modify", |candidate, transaction| {
                    let changes = [(id, Some(after.clone()))];
                    let change_set = candidate.apply_style_table_changes(
                        "dimstyle modify",
                        transaction,
                        StyleTableChanges {
                            dim_styles: &changes,
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        change_set,
                        Vec::new(),
                        vec![cad_history::dim_style_patch(
                            id,
                            Some(before.clone()),
                            candidate.dim_style(id).cloned(),
                        )],
                        Vec::new(),
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "dimstyle.modified",
                        format!(
                        "DIMSTYLE MODIFY: updated '{}' ({} field(s)); existing dimensions keep \
                         their baked snapshot, so DIMREGEN-style updates are per-entity and \
                         not performed here",
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
