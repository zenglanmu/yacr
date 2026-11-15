//! `-FIND` / `FIND` (2026-11-07): report or replace text across model-space
//! TEXT, MTEXT and MULTILEADER text.
//!
//! Search is a plain **ASCII case-insensitive substring** of what the viewport
//! draws. Every `Text` entity is matched on the **plain-text projection** of its
//! stored value — MTEXT, a text-bearing MULTILEADER (which stores the same
//! `\P`/`\f…;` markup) and single-line `AcDbText` alike, because
//! `cad-representation`'s provider shapes all of them through the same
//! `parse_mtext` (see [`mtext_preview_text`]). The projection is produced by that
//! *same* tokenizer ([`cad_representation::mtext_projection`]): formatting runs
//! contribute no characters, `\P`/`\N`/`^J`/literal newlines are one `'\n'`
//! each, and escapes (`\~`, `%%d`, `\U+XXXX`, `\S`) contribute the character
//! they draw. A needle that only exists in the markup (`\fArial|b1;`, `|b1;`)
//! is therefore **not** a match, and a hit is reported with its plain-text
//! context.
//!
//! Each projected character carries the byte range of the markup that produced
//! it, so a replace can rebuild the markup instead of rewriting the whole
//! value: the replacement is inserted at the **first** matched character's byte
//! range, every other matched character's own bytes are deleted, and every byte
//! in between — `\f…;`, `\H…;`, `{`/`}` — is copied verbatim. A hit that spans
//! two runs therefore lands where it starts (that run's codes are before the
//! match, so they still apply) and deletes the matched characters of the later
//! run *without touching that run's codes*, which keep styling what follows. A
//! single inserted string cannot honestly carry two formats, so the format at
//! the match start wins and no run's codes are rewritten or dropped. The inserted
//! text is escaped (`\`, `{`, `}` and line breaks), so a replacement cannot
//! inject control content.
//!
//! Two things are refused by name instead of being written approximately:
//! a match that covers only **part** of a structured code (`\S…;` stacked
//! fraction, `^I` tab) — `find.mtext_partial_code` — because neither grammar has
//! a partial form, and a replacement the control-content grammar would draw as
//! something other than the typed text (`%%d`, `^I`, …) —
//! `find.mtext_replacement_not_literal`. Replacing **all** of a structure's
//! drawn text is allowed and stored as literal text.
//!
//! A MULTILEADER's string is reached through
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
use cad_representation::mtext_projection;
use cad_representation::text::ProjectedOrigin;
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
            // Both MTEXT and single-line TEXT are matched against what they
            // draw: the viewport parses the same control content for each.
            let shown = mtext_preview_text(text);
            if contains(&shown, needle) {
                vec![FindHit::entity_hit(entity, shown)]
            } else {
                Vec::new()
            }
        }
        SemanticGeometry::MultiLeader(leader) if leader.content() == MLeaderContent::Text => {
            let shown = mtext_preview_text(&leader.text().string);
            if contains(&shown, needle) {
                vec![FindHit::entity_hit(entity, shown)]
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
    !occurrences(haystack, needle).is_empty()
}

/// Every non-overlapping ASCII case-insensitive occurrence, as byte ranges.
fn occurrences(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let hay = haystack.as_bytes();
    let needle = needle.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index + needle.len() <= hay.len() {
        if hay[index..index + needle.len()].eq_ignore_ascii_case(needle) {
            found.push((index, index + needle.len()));
            index += needle.len();
        } else {
            index += 1;
        }
    }
    found
}

/// Whether the entity is a **multiline** text (`AcDbMText`) rather than a
/// single-line `AcDbText`.
///
/// Both are parsed by the same engine in the viewport, so both are searched on
/// their **drawn** text (see [`mtext_preview_text`]); the type key only decides
/// whether a replacement may store a line break (`text_policy`).
fn stores_mtext_markup(entity: &DbEntity) -> bool {
    entity.object.type_key.eq_ignore_ascii_case("AcDbMText")
}

/// The text a stored `Text` value draws, for the report and for matching.
///
/// The viewport parses control content for **every** `Text` geometry, not just
/// MTEXT: `crates/cad-representation/src/provider/default.rs` shapes each one
/// through `FontEngine::shape`/`shape_unstyled`, and `engine.rs` parses the same
/// `parse_mtext` for both. An `AcDbText` can therefore store `%%d`, `\~`, `\{`
/// or even `\P` and draw something other than its raw string, so it is matched
/// on the projection too.
fn mtext_preview_text(raw: &str) -> String {
    mtext_projection(raw).text()
}

/// The replacement as it must be **drawn**: every line-break form is one
/// paragraph break (the same CRLF-as-one-break rule as
/// [`super::text_policy::newlines_to_paragraph_breaks`]), everything else is
/// literal.
fn drawn_replacement(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' => out.push('\n'),
            other => out.push(other),
        }
    }
    out
}

/// A short, single-line clip of `text` for a diagnostic message.
fn clip(text: &str) -> String {
    let mut clipped: String = text.chars().take(PREVIEW_CHARS).collect();
    if clipped.chars().count() < text.chars().count() {
        clipped.push('…');
    }
    clipped.replace(['\r', '\n'], " ")
}

/// Stable name of the refusal for an edit that would cover only part of a
/// structured MTEXT code (a `\S…;` stacked fraction or a `^I` tab).
const PARTIAL_CODE: &str = "find.mtext_partial_code";

/// Replace every occurrence of `needle` in the text a stored value draws, and
/// splice the replacement back into the markup.
///
/// The rule: the replacement goes at the **first** matched character's byte
/// range; the other matched characters lose only their own bytes; every byte
/// between them (formatting codes, quoting braces) is copied verbatim. A hit
/// that starts inside `{\fArial|b1;Bold}` therefore keeps that `\f…;` in front
/// of the inserted text, and a hit that continues into the next run deletes the
/// matched characters there while leaving that run's own codes in place. One
/// inserted string cannot honestly carry two formats, so the match start wins;
/// no run's codes are rewritten, reordered or dropped.
///
/// Replacing **all** the characters one structured code draws (`\S1/2;`'s
/// `1/2`, `^I`'s four spaces) writes the replacement as literal text: the user
/// replaced everything that code stood for. Replacing only **part** of them is
/// refused by name.
///
/// # Errors
///
/// * `find.mtext_partial_code` ([`PARTIAL_CODE`]) — a match covers only part of
///   a `\S…;` stacked fraction or a `^I` tab. Neither grammar has a partial form
///   (`\S` needs one separator between non-empty parts and cannot carry `/`,
///   `#`, `^` or `;` inside a part; `^I` is exactly four spaces), so rebuilding
///   one would drop the structure the user did not ask about. Nothing is
///   written.
/// * `find.mtext_replacement_not_literal` — the rebuilt value would not draw the
///   replacement as typed (control content has no literal form for text such as
///   `%%d`, `^I` or a stray trailing `\`), so the drawn result would differ from
///   the request. Nothing is written.
fn replace_mtext_markup(raw: &str, needle: &str, replacement: &str) -> CadResult<String> {
    let projection = mtext_projection(raw);
    let text = projection.text();
    let matches = occurrences(&text, needle);
    if matches.is_empty() {
        return Ok(raw.to_string());
    }
    // Byte offset of every projected character, plus the total length.
    let mut offsets = Vec::with_capacity(projection.chars.len() + 1);
    let mut at = 0;
    for projected in &projection.chars {
        offsets.push(at);
        at += projected.ch.len_utf8();
    }
    offsets.push(at);
    let char_index = |byte: usize| offsets.partition_point(|offset| *offset < byte);
    let mut matched = vec![false; projection.chars.len()];
    let mut insertion = vec![false; projection.chars.len()];
    for (start, end) in &matches {
        let first = char_index(*start);
        let last = char_index(*end);
        insertion[first] = true;
        for flag in &mut matched[first..last] {
            *flag = true;
        }
    }

    let inserted = escape_mtext_insertion(replacement);
    let mut out = String::with_capacity(raw.len());
    let mut copied = 0;
    let mut index = 0;
    while index < projection.chars.len() {
        let (start, end) = (projection.chars[index].start, projection.chars[index].end);
        // Characters a single code produced share one byte range and can only
        // be edited as a unit (`^I` draws four spaces from two bytes).
        let mut group_end = index + 1;
        while group_end < projection.chars.len()
            && projection.chars[group_end].start == start
            && projection.chars[group_end].end == end
        {
            group_end += 1;
        }
        let covered = matched[index..group_end]
            .iter()
            .filter(|flag| **flag)
            .count();
        if let Some(structure) = structure_of(projection.chars[index].origin) {
            if covered != 0 && covered != group_end - index {
                // Part of a token whose grammar has no partial form: refuse by
                // name rather than silently dropping the structure it encodes.
                return Err(CadError::Unsupported(format!(
                    "{PARTIAL_CODE}: the match covers only part of the control-content code '{}' \
                     ({structure}) at bytes {start}..{end}; rewriting it would drop that \
                     structure, so nothing was written — match all of its drawn text, or edit \
                     the value with TEXTEDIT",
                    &raw[start..end]
                )));
            }
        }
        let touched = covered > 0 || insertion[index..group_end].iter().any(|flag| *flag);
        if touched {
            out.push_str(&raw[copied..start]);
            for position in index..group_end {
                if insertion[position] {
                    out.push_str(&inserted);
                }
                if !matched[position] {
                    push_mtext_escaped(&mut out, projection.chars[position].ch);
                }
            }
            copied = end;
        }
        index = group_end;
    }
    out.push_str(&raw[copied..]);

    // The rebuilt markup must draw exactly the text that was asked for: the
    // value's grammar has no literal form for everything a replacement can
    // contain (`%%d`, `^I`, a stray `\`), and drawing something else silently
    // would be a lie about what was written.
    let mut expected = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in &matches {
        expected.push_str(&text[cursor..*start]);
        expected.push_str(&drawn_replacement(replacement));
        cursor = *end;
    }
    expected.push_str(&text[cursor..]);
    let drawn = mtext_projection(&out).text();
    if drawn != expected {
        return Err(CadError::InvalidInput(format!(
            "find.mtext_replacement_not_literal: the rebuilt MTEXT markup would draw '{}' where \
             '{}' was requested; the value's grammar has no literal form for that text, so nothing \
             was written",
            clip(&drawn),
            clip(&expected)
        )));
    }
    Ok(out)
}

/// The structure a code encodes, when a partial rewrite of its drawn characters
/// cannot keep it; `None` for text that can be rewritten literally.
fn structure_of(origin: ProjectedOrigin) -> Option<&'static str> {
    match origin {
        ProjectedOrigin::Literal => None,
        ProjectedOrigin::Tab => Some("a stored tab, drawn as spaces"),
        ProjectedOrigin::Fraction => Some("a stacked fraction, drawn inline"),
    }
}

/// Escape typed text so it is drawn literally: backslash, grouping braces and
/// line breaks (as `\P`) cannot reach the parser as control content.
fn escape_mtext_insertion(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '{' => escaped.push_str("\\{"),
            '}' => escaped.push_str("\\}"),
            other => escaped.push(other),
        }
    }
    newlines_to_paragraph_breaks(&escaped)
}

/// Write a projected character back as literal markup. A paragraph break is
/// already a single `'\n'` here, so it becomes one `\P`.
fn push_mtext_escaped(out: &mut String, ch: char) {
    match ch {
        '\\' => out.push_str("\\\\"),
        '{' => out.push_str("\\{"),
        '}' => out.push_str("\\}"),
        '\r' | '\n' => out.push_str("\\P"),
        other => out.push(other),
    }
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
            line_spacing,
        } => {
            // A single-line TEXT still refuses a stored line break
            // (`text_policy`), but the drawn text is the same projection either
            // way: the viewport parses `%%d`, `\~`, `\{` and `\P` for every
            // `Text` geometry, so the replacement is spliced and escaped the
            // same way it is for MTEXT.
            if !stores_mtext_markup(entity) && has_line_break(replacement) {
                return Err(text_policy::single_line_newline_error("FIND"));
            }
            after.geometry = SemanticGeometry::Text {
                text: replace_mtext_markup(text, needle, replacement)?,
                position: *position,
                style: *style,
                height: *height,
                rotation: *rotation,
                font: font.clone(),
                h_align: *h_align,
                v_align: *v_align,
                wrap_width: *wrap_width,
                width_factor: *width_factor,
                // A text replacement never touches the stored spacing.
                line_spacing: *line_spacing,
            };
        }
        SemanticGeometry::MultiLeader(leader) => {
            // A text-bearing multileader stores the same MTEXT markup.
            let replaced = replace_mtext_markup(&leader.text().string, needle, replacement)?;
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
