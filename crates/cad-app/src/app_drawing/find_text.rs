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

        let matches: Vec<DbEntity> = drawing
            .model_space()
            .into_iter()
            .filter(|entity| text_of(entity).is_some_and(|text| contains(text, needle)))
            .cloned()
            .collect();

        let Some(replacement) = replacement else {
            return Ok(report_outcome(&matches, needle));
        };
        if matches.is_empty() {
            return Ok(report_outcome(&matches, needle));
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

fn report_outcome(matches: &[DbEntity], needle: &str) -> CommandOutcome {
    let ids: Vec<String> = matches
        .iter()
        .take(10)
        .map(|entity| entity.id.0.to_string())
        .collect();
    let preview = matches
        .first()
        .and_then(|entity| text_of(entity))
        .map(|text| preview(text, needle))
        .unwrap_or_default();
    let more = matches.len().saturating_sub(ids.len());
    let ids = if more > 0 {
        format!("{}, +{more} more", ids.join(", "))
    } else {
        ids.join(", ")
    };
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: "find.result".into(),
            message: format!(
                "FIND: {} match(es) for '{needle}' [ids: {}] first: {preview}",
                matches.len(),
                ids
            ),
        }],
        measurement: None,
        point_report: None,
    }
}

/// The stored raw text of one text-bearing entity.
fn text_of(entity: &DbEntity) -> Option<&str> {
    match &entity.geometry {
        SemanticGeometry::Text { text, .. } => Some(text),
        SemanticGeometry::MultiLeader(leader) if leader.content() == MLeaderContent::Text => {
            Some(&leader.text().string)
        }
        _ => None,
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
            };
        }
        SemanticGeometry::MultiLeader(leader) => {
            let stored = newlines_to_paragraph_breaks(replacement);
            let replaced = replace_all(&leader.text().string, needle, &stored);
            after.geometry = SemanticGeometry::MultiLeader(Box::new(leader.with_text(&replaced)?));
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
