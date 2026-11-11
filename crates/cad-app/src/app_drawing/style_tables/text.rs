//! The `STYLE` text-style table operations (`NEW`/`RENAME`/`DELETE`/`SET`).
//!
//! Names use the shared exact-then-unique-case-insensitive resolution rule and a
//!
//! Split out of `style_tables.rs` for the code-size gate; the dispatch and the
//! shared commit/name-resolution helpers stay in the parent module.

use super::*;

impl Application {
    pub(super) fn text_style_table_op(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
        op: StyleTableOp,
    ) -> CadResult<CommandOutcome> {
        let drawing = document_drawing(self, command)?;
        match op {
            StyleTableOp::List => unreachable!("the list form is dispatched by the report path"),
            StyleTableOp::Set { .. } => Err(CadError::Unsupported(
                "style.set_unsupported: a text style has no current-style variable in this build; \
             use STYLE SET <name> <FONT|KEYS|ADDKEY|REMOVEKEY> <value> to modify the row"
                    .into(),
            )),
            StyleTableOp::Create { name, from } => {
                let resource_keys = match from.as_deref() {
                    Some(base) => {
                        let id = resolve_unique_style(&drawing, base)?;
                        drawing
                            .style(id)
                            .map(|style| style.resource_keys.clone())
                            .unwrap_or_default()
                    }
                    None => Vec::new(),
                };
                let id = drawing.next_style_id();
                let row = Style {
                    id,
                    name: name.clone(),
                    resource_keys,
                };
                self.commit_style_write(command, "style create", |candidate, transaction| {
                    let changes = [(id, Some(row.clone()))];
                    let change_set = candidate.apply_style_table_changes(
                        "style create",
                        transaction,
                        StyleTableChanges {
                            styles: &changes,
                            ..Default::default()
                        },
                    )?;
                    let after = candidate.style(id).cloned();
                    Ok((
                        change_set,
                        vec![cad_history::style_patch(id, None, after)],
                        Vec::new(),
                        Vec::new(),
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "style.created",
                        format!(
                            "STYLE: created text style '{name}' (id {}); one transaction / one \
                         undo step",
                            id.0
                        ),
                    ));
                    outcome
                })
            }
            StyleTableOp::Rename { old, new } => {
                let id = resolve_unique_style(&drawing, &old)?;
                let before = drawing
                    .style(id)
                    .cloned()
                    .ok_or_else(|| style_unknown(&old))?;
                if before.name == new.trim() {
                    return Ok(report_outcome(vec![diagnostic(
                        "style.renamed",
                        format!(
                            "STYLE RENAME: '{}' already has that name; nothing changed",
                            before.name
                        ),
                    )]));
                }
                let old_name = before.name.clone();
                let mut after = before.clone();
                after.name = new;
                self.commit_style_write(command, "style rename", |candidate, transaction| {
                    let style_changes = [(id, Some(after.clone()))];
                    // Cascade the name reference the other two style tables store
                    // (DIMTXSTY / MLEADERSTYLE 342), so a rename never leaves a
                    // stale name behind.
                    let dim_changes: Vec<(DimStyleId, Option<DimStyle>)> = candidate
                        .dim_styles()
                        .filter(|row| row.text_style_name.eq_ignore_ascii_case(old_name.trim()))
                        .map(|row| {
                            let mut updated = row.clone();
                            updated.text_style_name = after.name.trim().to_string();
                            (row.id, Some(updated))
                        })
                        .collect();
                    let mleader_changes: Vec<(MLeaderStyleId, Option<MLeaderStyle>)> = candidate
                        .mleader_styles()
                        .filter(|row| row.text_style_name.eq_ignore_ascii_case(old_name.trim()))
                        .map(|row| {
                            let mut updated = row.clone();
                            updated.text_style_name = after.name.trim().to_string();
                            (row.id, Some(updated))
                        })
                        .collect();
                    let change_set = candidate.apply_style_table_changes(
                        "style rename",
                        transaction,
                        StyleTableChanges {
                            styles: &style_changes,
                            dim_styles: &dim_changes,
                            mleader_styles: &mleader_changes,
                        },
                    )?;
                    let stored = candidate.style(id).cloned();
                    let dim_patches = dim_changes
                        .iter()
                        .map(|(id, change)| {
                            if change.is_some() {
                                cad_history::dim_style_patch(
                                    *id,
                                    drawing.dim_style(*id).cloned(),
                                    candidate.dim_style(*id).cloned(),
                                )
                            } else {
                                cad_history::dim_style_patch(
                                    *id,
                                    drawing.dim_style(*id).cloned(),
                                    None,
                                )
                            }
                        })
                        .collect();
                    let mleader_patches = mleader_changes
                        .iter()
                        .map(|(id, _)| {
                            cad_history::mleader_style_patch(
                                *id,
                                drawing.mleader_style(*id).cloned(),
                                candidate.mleader_style(*id).cloned(),
                            )
                        })
                        .collect();
                    Ok((
                        change_set,
                        vec![cad_history::style_patch(id, Some(before.clone()), stored)],
                        dim_patches,
                        mleader_patches,
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "style.renamed",
                        format!(
                            "STYLE RENAME: renamed '{old_name}' to '{}' (one transaction / one \
                         undo step; DIMTXSTY/leader text-style names were retargeted)",
                            after.name.trim()
                        ),
                    ));
                    outcome
                })
            }
            StyleTableOp::Delete { name } => {
                let id = resolve_unique_style(&drawing, &name)?;
                let before = drawing
                    .style(id)
                    .cloned()
                    .ok_or_else(|| style_unknown(&name))?;
                self.commit_style_write(command, "style delete", |candidate, transaction| {
                    let changes = [(id, None)];
                    let change_set = candidate.apply_style_table_changes(
                        "style delete",
                        transaction,
                        StyleTableChanges {
                            styles: &changes,
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        change_set,
                        vec![cad_history::style_patch(id, Some(before.clone()), None)],
                        Vec::new(),
                        Vec::new(),
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    outcome.diagnostics.push(diagnostic(
                        "style.deleted",
                        format!(
                            "STYLE DELETE: deleted text style '{}' (one transaction / one undo \
                         step)",
                            before.name
                        ),
                    ));
                    outcome
                })
            }
            StyleTableOp::Modify { name, assignments } => {
                let id = resolve_unique_style(&drawing, &name)?;
                let before = drawing
                    .style(id)
                    .cloned()
                    .ok_or_else(|| style_unknown(&name))?;
                let (after, renamed) = modified_style_row(&before, &assignments)?;
                if after == before {
                    return Ok(report_outcome(vec![diagnostic(
                        "style.unchanged",
                        format!(
                            "STYLE SET: text style '{}' already has those values; nothing changed",
                            before.name
                        ),
                    )]));
                }
                let old_name = before.name.clone();
                self.commit_style_write(command, "style modify", |candidate, transaction| {
                    let style_changes = [(id, Some(after.clone()))];
                    let dim_changes: Vec<(DimStyleId, Option<DimStyle>)> =
                        if let Some((old, new)) = &renamed {
                            candidate
                                .dim_styles()
                                .filter(|row| row.text_style_name.eq_ignore_ascii_case(old.trim()))
                                .map(|row| {
                                    let mut updated = row.clone();
                                    updated.text_style_name = new.trim().to_string();
                                    (row.id, Some(updated))
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                    let mleader_changes: Vec<(MLeaderStyleId, Option<MLeaderStyle>)> =
                        if let Some((old, new)) = &renamed {
                            candidate
                                .mleader_styles()
                                .filter(|row| row.text_style_name.eq_ignore_ascii_case(old.trim()))
                                .map(|row| {
                                    let mut updated = row.clone();
                                    updated.text_style_name = new.trim().to_string();
                                    (row.id, Some(updated))
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                    let change_set = candidate.apply_style_table_changes(
                        "style modify",
                        transaction,
                        StyleTableChanges {
                            styles: &style_changes,
                            dim_styles: &dim_changes,
                            mleader_styles: &mleader_changes,
                        },
                    )?;
                    let dim_patches = dim_changes
                        .iter()
                        .map(|(id, _)| {
                            cad_history::dim_style_patch(
                                *id,
                                drawing.dim_style(*id).cloned(),
                                candidate.dim_style(*id).cloned(),
                            )
                        })
                        .collect();
                    let mleader_patches = mleader_changes
                        .iter()
                        .map(|(id, _)| {
                            cad_history::mleader_style_patch(
                                *id,
                                drawing.mleader_style(*id).cloned(),
                                candidate.mleader_style(*id).cloned(),
                            )
                        })
                        .collect();
                    Ok((
                        change_set,
                        vec![cad_history::style_patch(
                            id,
                            Some(before.clone()),
                            candidate.style(id).cloned(),
                        )],
                        dim_patches,
                        mleader_patches,
                    ))
                })
                .map(|outcome| {
                    let mut outcome = outcome;
                    let mut message = format!(
                        "STYLE SET: updated text style '{}' ({} field(s)); one transaction / one \
                     undo step",
                        old_name,
                        assignments.len()
                    );
                    if renamed.is_some() {
                        message.push_str("; DIMTXSTY/leader text-style names were retargeted");
                    }
                    outcome
                        .diagnostics
                        .push(diagnostic("style.modified", message));
                    outcome
                })
            }
        }
    }
}
