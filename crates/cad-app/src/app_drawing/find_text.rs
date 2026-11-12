//! `-FIND` / `FIND` (2026-11-07): report or replace text across model-space
//! TEXT, MTEXT and MULTILEADER text.
//!
//! Search is a plain **ASCII case-insensitive substring** over the stored raw
//! string. MTEXT formatting runs are not parsed: `\P`, `\f…;` and `{`/`}` are
//! ordinary characters here. A MULTILEADER's string is reached through
//! [`cad_domain::MultiLeaderGeometry::text`] and replaced with
//! [`cad_domain::MultiLeaderGeometry::with_text`], which rebuilds the derived
//! display; a TEXT/MTEXT keeps every other field of its `Text` geometry.
//!
//! `-FIND` is report-only. `FIND <needle>` reports; `FIND <needle> => <text>`
//! replaces every occurrence on every match in **one** transaction / one undo
//! step. An `AcDbText` target refuses a replacement containing `\r`/`\n`
//! (`text.single_line_newline`); MTEXT/MULTILEADER store line breaks as `\P`
//! via the one policy in [`super::text_policy`]. A refusal happens before the
//! transaction is opened, so nothing is written and no undo entry is recorded.

use super::*;
use cad_domain::{MLeaderContent, SemanticGeometry};
use text_policy::{has_line_break, newlines_to_paragraph_breaks};

/// Preview length of one match in the report diagnostic.
const PREVIEW_CHARS: usize = 60;

impl Application {
    /// Apply one `-FIND` (report only) or `FIND` (report or replace) command.
    pub(crate) fn find_text(
        &mut self,
        command: &Command,
        replace_allowed: bool,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::TextFind {
            needle,
            replacement,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "find.needs_needle: FIND expects TextFind { needle, replacement }".into(),
            ));
        };
        if needle.is_empty() {
            return Err(CadError::InvalidInput(
                "find.empty_needle: the search string must not be empty".into(),
            ));
        }
        if replacement.is_some() && !replace_allowed {
            return Err(CadError::InvalidInput(
                "find.report_only: -FIND reports matches and never replaces text".into(),
            ));
        }

        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        // Search **cell by cell** for a table: a needle must never match across
        // two cells (`id` + ` ` + `name`). Each hit records the cell it matched
        // so the report can name it.
        let hits: Vec<FindHit> = drawing
            .model_space()
            .into_iter()
            .flat_map(|entity| find_hits(entity, needle))
            .collect();

        // Replacement works per entity; several cell hits on one table collapse
        // into one update so the table is rebuilt once.
        let mut seen = std::collections::HashSet::new();
        let matches: Vec<DbEntity> = hits
            .iter()
            .filter(|hit| seen.insert(hit.entity.id.0))
            .map(|hit| hit.entity.clone())
            .collect();

        let Some(replacement) = replacement else {
            return Ok(report_outcome(&hits, needle));
        };
        if matches.is_empty() {
            return Ok(report_outcome(&hits, needle));
        }

        // Plan every replacement first: all-or-nothing, no transaction on a
        // refusal.
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::new();
        for before in &matches {
            let after = replace_in_entity(before, needle, replacement)?;
            if after != *before {
                planned.push((before.clone(), after));
            }
        }
        if planned.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "find.unchanged".into(),
                    message: format!(
                        "FIND: every match already contains the replacement text ({needle})"
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("find text", transaction)?;
            for (_, after) in &planned {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };

        let mut patches = Vec::with_capacity(planned.len());
        let mut objects = Vec::with_capacity(planned.len());
        for (before, _) in &planned {
            let after = candidate
                .entity(before.id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("edited text vanished after commit".into()))?;
            objects.push(ObjectId(before.id.0));
            patches.push(drawing_patch(before.id, Some(before.clone()), Some(after)));
        }
        let count = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "find text".into(),
                patches,
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
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "find.replaced".into(),
                message: format!("FIND: replaced text on {count} object(s)"),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `-FIND` wrapper: report only.
    pub(crate) fn find_report(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        self.find_text(command, false)
    }

    /// `FIND` wrapper: report, or replace when the payload carries one.
    pub(crate) fn find_replace(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        self.find_text(command, true)
    }
}

/// One search hit: the entity, the matching cell when the entity is a table,
/// and the raw text that matched.
struct FindHit {
    entity: DbEntity,
    cell: Option<(usize, usize)>,
    text: String,
}

impl FindHit {
    fn entity_hit(entity: &DbEntity, text: String) -> Self {
        FindHit {
            entity: entity.clone(),
            cell: None,
            text,
        }
    }

    fn cell_hit(entity: &DbEntity, row: usize, column: usize, text: String) -> Self {
        FindHit {
            entity: entity.clone(),
            cell: Some((row, column)),
            text,
        }
    }
}

/// Every match in one entity. A table is searched cell by cell so a needle
/// cannot span two cells; every other text-bearing entity contributes at most
/// one hit.
fn find_hits(entity: &DbEntity, needle: &str) -> Vec<FindHit> {
    match &entity.geometry {
        SemanticGeometry::Text { text, .. } => {
            if contains(text, needle) {
                vec![FindHit::entity_hit(entity, text.clone())]
            } else {
                Vec::new()
            }
        }
        SemanticGeometry::MultiLeader(leader) if leader.content() == MLeaderContent::Text => {
            let text = &leader.text().string;
            if contains(text, needle) {
                vec![FindHit::entity_hit(entity, text.clone())]
            } else {
                Vec::new()
            }
        }
        SemanticGeometry::Table(table) => {
            let mut hits = Vec::new();
            for (row_index, row) in table.rows().iter().enumerate() {
                for (column_index, cell) in row.cells.iter().enumerate() {
                    if !cell.text.is_empty() && contains(&cell.text, needle) {
                        hits.push(FindHit::cell_hit(
                            entity,
                            row_index,
                            column_index,
                            cell.text.clone(),
                        ));
                    }
                }
            }
            hits
        }
        _ => Vec::new(),
    }
}

fn report_outcome(hits: &[FindHit], needle: &str) -> CommandOutcome {
    let mut ids: Vec<String> = Vec::new();
    for hit in hits {
        let id = hit.entity.id.0.to_string();
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let total_ids = ids.len();
    ids.truncate(10);
    let more = total_ids.saturating_sub(ids.len());
    let ids = if more > 0 {
        format!("{}, +{more} more", ids.join(", "))
    } else {
        ids.join(", ")
    };
    let preview = hits
        .first()
        .map(|hit| preview(&hit.text, needle))
        .unwrap_or_default();
    // Name the matching cells, so a table's per-cell matches are visible rather
    // than summarised as one joined string.
    let cells: Vec<String> = hits
        .iter()
        .filter_map(|hit| hit.cell)
        .take(10)
        .map(|(row, column)| format!("row {row}, column {column}"))
        .collect();
    let cells = if cells.is_empty() {
        String::new()
    } else {
        format!("; cells: {}", cells.join(", "))
    };
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: "find.result".into(),
            message: format!(
                "FIND: {} match(es) for '{needle}' [ids: {}] first: {preview}{cells}",
                hits.len(),
                ids
            ),
        }],
        measurement: None,
        point_report: None,
    }
}

fn contains(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let hay = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.len() > hay.len() {
        return false;
    }
    (0..=hay.len() - needle.len())
        .any(|start| hay[start..start + needle.len()].eq_ignore_ascii_case(needle))
}

/// Replace every non-overlapping ASCII case-insensitive occurrence.
fn replace_all(haystack: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(haystack.len());
    let bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    let needle_len = needle_bytes.len();
    let mut index = 0;
    while index < bytes.len() {
        if index + needle_len <= bytes.len()
            && bytes[index..index + needle_len].eq_ignore_ascii_case(needle_bytes)
        {
            out.push_str(replacement);
            index += needle_len;
        } else {
            // Advance one **character**, not one byte, so a multi-byte character
            // is copied whole.
            let ch = haystack[index..]
                .chars()
                .next()
                .expect("index is inside the string");
            out.push(ch);
            index += ch.len_utf8();
        }
    }
    out
}

fn replace_in_entity(entity: &DbEntity, needle: &str, replacement: &str) -> CadResult<DbEntity> {
    let mut after = entity.clone();
    match &entity.geometry {
        SemanticGeometry::Text {
            text,
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            wrap_width,
            width_factor,
        } => {
            let mtext = entity.object.type_key.eq_ignore_ascii_case("AcDbMText");
            let stored = if mtext {
                newlines_to_paragraph_breaks(replacement)
            } else if has_line_break(replacement) {
                return Err(text_policy::single_line_newline_error("FIND"));
            } else {
                replacement.to_string()
            };
            after.geometry = SemanticGeometry::Text {
                text: replace_all(text, needle, &stored),
                position: *position,
                style: *style,
                height: *height,
                rotation: *rotation,
                font: font.clone(),
                h_align: *h_align,
                v_align: *v_align,
                wrap_width: *wrap_width,
                width_factor: *width_factor,
            };
        }
        SemanticGeometry::MultiLeader(leader) => {
            let stored = newlines_to_paragraph_breaks(replacement);
            let replaced = replace_all(&leader.text().string, needle, &stored);
            after.geometry = SemanticGeometry::MultiLeader(Box::new(leader.with_text(&replaced)?));
        }
        SemanticGeometry::Table(table) => {
            // Cell text is single-line; a multi-line replacement is refused by
            // the same policy the TEXT channel uses, before any transaction.
            if has_line_break(replacement) {
                return Err(text_policy::single_line_newline_error("FIND"));
            }
            let mut changes: Vec<(usize, usize, String)> = Vec::new();
            for (row_index, row) in table.rows().iter().enumerate() {
                for (column_index, cell) in row.cells.iter().enumerate() {
                    if cell.text.is_empty() || !contains(&cell.text, needle) {
                        continue;
                    }
                    changes.push((
                        row_index,
                        column_index,
                        replace_all(&cell.text, needle, replacement),
                    ));
                }
            }
            if !changes.is_empty() {
                // One bulk rebuild for every matched cell (and one undo step).
                after.geometry = SemanticGeometry::Table(Box::new(table.with_cell_texts(changes)?));
            }
        }
        other => {
            return Err(CadError::Unsupported(format!(
                "find.target_not_text: {} is not a text-bearing entity",
                geometry_kind(other)
            )))
        }
    }
    Ok(after)
}

fn preview(text: &str, needle: &str) -> String {
    let start = text
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
        .unwrap_or(0);
    let slice: String = text[start..].chars().take(PREVIEW_CHARS).collect();
    slice.replace(['\r', '\n'], " ")
}
