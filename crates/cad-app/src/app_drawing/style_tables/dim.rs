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
                     this is session state only — dimensions created from now on use it, and \
                     existing dimensions keep their resolved snapshot (style edits are not \
                     retroactive, ADR 0017)",
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
                    // The template is the drawing's **stored** `Standard` row,
                    // not a fixed constant: a drawing whose Standard was
                    // imported or modified keeps those values.
                    None => drawing
                        .dim_style_by_name(cad_db::DEFAULT_DIM_STYLE_NAME)
                        .cloned()
                        .ok_or_else(|| {
                            invalid_refusal(
                                "dimstyle.standard_missing",
                                format!(
                                    "the drawing has no '{}' dimension style to copy; name an \
                                     existing row with NEW <name> <base>",
                                    cad_db::DEFAULT_DIM_STYLE_NAME
                                ),
                            )
                        })?,
                };
                let id = drawing.next_dim_style_id();
                let mut row = base;
                row.id = id;
                row.name = name.clone();
                let stored_name = row.name.trim().to_string();
                let outcome = self.commit_style_write(
                    command,
                    "dimstyle create",
                    |candidate, transaction| {
                        let change_set = candidate.apply_dim_style_change(
                            "dimstyle create",
                            transaction,
                            id,
                            Some(&row),
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
                    },
                )?;
                // The session follows the table only after the write succeeded:
                // a refused create must not leave creation pointing at a row
                // that does not exist.
                session.current_dim_style = stored_name;
                let mut outcome = outcome;
                outcome.diagnostics.push(diagnostic(
                    "dimstyle.created",
                    format!(
                        "DIMSTYLE NEW: created dimension style '{name}' (id {}) and made it \
                         current; dimensions created from now on use it, while existing \
                         dimensions keep their resolved snapshot (style edits are not \
                         retroactive, ADR 0017); one transaction / one undo step",
                        id.0
                    ),
                ));
                Ok(outcome)
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
                let follows_current = session
                    .current_dim_style
                    .eq_ignore_ascii_case(before.name.trim());
                let outcome = self.commit_style_write(
                    command,
                    "dimstyle rename",
                    |candidate, transaction| {
                        let change_set = candidate.apply_dim_style_change(
                            "dimstyle rename",
                            transaction,
                            id,
                            Some(&after),
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
                    },
                )?;
                // Only after the table write succeeds does the session follow
                // the new name; a refusal leaves creation on the old row.
                if follows_current {
                    session.current_dim_style = stored_name;
                }
                let mut outcome = outcome;
                outcome.diagnostics.push(diagnostic(
                    "dimstyle.renamed",
                    format!(
                        "DIMSTYLE RENAME: renamed '{}' to '{}'; existing dimensions keep their \
                         resolved snapshot (style edits are not retroactive, ADR 0017); one \
                         transaction / one undo step",
                        before.name,
                        after.name.trim()
                    ),
                ));
                Ok(outcome)
            }
            StyleTableOp::Delete { name } => {
                let id = resolve_unique_dim_style(&drawing, &name)?;
                let before = drawing
                    .dim_style(id)
                    .cloned()
                    .ok_or_else(|| dim_style_unknown(&name))?;
                let follows_current = session
                    .current_dim_style
                    .eq_ignore_ascii_case(before.name.trim());
                let outcome = self.commit_style_write(
                    command,
                    "dimstyle delete",
                    |candidate, transaction| {
                        let change_set = candidate.apply_dim_style_change(
                            "dimstyle delete",
                            transaction,
                            id,
                            None,
                        )?;
                        Ok((
                            change_set,
                            Vec::new(),
                            vec![cad_history::dim_style_patch(id, Some(before.clone()), None)],
                            Vec::new(),
                        ))
                    },
                )?;
                // A refused delete changes nothing, including the session's
                // current style.
                if follows_current {
                    session.current_dim_style = cad_db::DEFAULT_DIM_STYLE_NAME.to_string();
                }
                let mut outcome = outcome;
                outcome.diagnostics.push(diagnostic(
                    "dimstyle.deleted",
                    format!(
                        "DIMSTYLE DELETE: deleted dimension style '{}' (one transaction / one \
                         undo step)",
                        before.name
                    ),
                ));
                Ok(outcome)
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
                let rename_retarget = renamed
                    .as_ref()
                    .filter(|(old, _)| session.current_dim_style.eq_ignore_ascii_case(old.trim()))
                    .map(|(_, new)| new.trim().to_string());
                let outcome = self.commit_style_write(
                    command,
                    "dimstyle modify",
                    |candidate, transaction| {
                        let change_set = candidate.apply_dim_style_change(
                            "dimstyle modify",
                            transaction,
                            id,
                            Some(&after),
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
                    },
                )?;
                // The session follows a `NAME` write only after the table write
                // succeeded.
                if let Some(new_name) = rename_retarget {
                    session.current_dim_style = new_name;
                }
                let mut outcome = outcome;
                outcome.diagnostics.push(diagnostic(
                    "dimstyle.modified",
                    format!(
                        "DIMSTYLE MODIFY: updated '{}' ({} field(s)); existing dimensions keep \
                         their resolved snapshot (style edits are not retroactive, ADR 0017), so \
                         no DIMREGEN-style per-entity update is performed here",
                        before.name,
                        assignments.len()
                    ),
                ));
                Ok(outcome)
            }
            StyleTableOp::List => unreachable!("the list form is dispatched by the report path"),
        }
    }
}
