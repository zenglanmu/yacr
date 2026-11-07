//! The three selection-based text commands (2026-11-07):
//! [`crate::CommandId::Torient`] (`TORIENT`), [`crate::CommandId::TCase`]
//! (`TCASE`) and [`crate::CommandId::Txt2MText`] (`TXT2MTXT`).
//!
//! All three read the **current session selection** at apply time
//! ([`SessionState::selection`]); there is no interactive "Select objects"
//! prompt (the declared divergence in [`crate::command_id::text_commands`]).
//! The bare selectors travel as [`CommandPayload::None`]; only the two typed
//! argument forms (`TORIENT <angle>`, `TCASE <mode>`) carry a payload, and both
//! are parsed by `cad_ui_core::session`.
//!
//! ## Shared rules
//!
//! * Every refusal happens before a transaction exists, so a refused command
//!   writes nothing: no revision bump, no `ChangeSet`, no undo entry.
//! * A successful rewrite is **one** transaction / one undo step, with the
//!   committed rows read back so the undo patch carries the exact stored value.
//! * An update keeps the entity id, layer, space, draw order and `type_key`;
//!   only the documented field changes.
//! * No change at all is a successful `*.unchanged` diagnostic and opens no
//!   transaction (never an empty undo entry).
//!
//! Text geometry is [`SemanticGeometry::Text`]; MTEXT is the same variant with
//! `object.type_key == "AcDbMText"` (the authority in
//! [`super::text_policy`]). MTEXT stores paragraph breaks as the raw `\P`
//! sequence, which the case transformer copies verbatim.

use super::*;
use crate::TextCaseMode;
use cad_domain::MLeaderContent;

/// The half-open rotation interval `(-π/2, π/2]` used by Most Readable.
///
/// A text rotated by exactly π/2 (baseline vertical, read bottom-to-top) is
/// upright and stays put; anything beyond it is folded back by π.
const MOST_READABLE_LIMIT: f64 = std::f64::consts::FRAC_PI_2;

/// A `torient.<code>` / `tcase.<code>` / `txt2mtxt.<code>` refusal.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// The ASCII name a selected entity's geometry is reported under.
fn kind_name(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "LINE",
        SemanticGeometry::Arc { .. } => "ARC",
        SemanticGeometry::Circle { .. } => "CIRCLE",
        SemanticGeometry::Polyline { .. } => "LWPOLYLINE",
        SemanticGeometry::Ellipse { .. } => "ELLIPSE",
        SemanticGeometry::Spline { .. } => "SPLINE",
        SemanticGeometry::Point(_) => "POINT",
        SemanticGeometry::Shape { .. } => "SHAPE",
        SemanticGeometry::Insert { .. } => "INSERT",
        SemanticGeometry::Hatch(_) => "HATCH",
        SemanticGeometry::MultiLeader(_) => "MULTILEADER",
        SemanticGeometry::Text { .. } => "TEXT",
        _ => "that entity",
    }
}

/// Fold one text rotation into `(-π/2, π/2]` by 180° steps.
///
/// This is the pure Most Readable rule: it keeps the baseline within a
/// readable quadrant and never changes the insertion point. A non-finite value
/// is returned unchanged (the caller reports it as `torient.not_reset` rather
/// than inventing a rotation).
pub(crate) fn most_readable_rotation(rotation: f64) -> f64 {
    if !rotation.is_finite() {
        return rotation;
    }
    let reduced = rotation.rem_euclid(std::f64::consts::PI);
    if reduced > MOST_READABLE_LIMIT {
        reduced - std::f64::consts::PI
    } else {
        reduced
    }
}

/// Whether one MTEXT escape code takes a `;`-terminated parameter payload.
///
/// MTEXT's parameterised codes are `\f`, `\F`, `\H`, `\W`, `\Q`, `\T`, `\A`,
/// `\C`, `\c`, `\p` and `\S`; the remaining letter codes are toggles
/// (`\L`/`\l`, `\O`/`\o`, `\K`/`\k`) or fixed pairs (`\P`, `\~`), so a generic
/// "consume to the next `;`" rule would swallow visible text after a toggle.
fn is_parameterized_escape(code: char) -> bool {
    matches!(
        code,
        'f' | 'F' | 'H' | 'W' | 'Q' | 'T' | 'A' | 'C' | 'c' | 'p' | 'S'
    )
}

/// The one pure case transformer shared by every `TCASE` target.
///
/// Only **visible** characters change. MTEXT formatting is copied verbatim:
///
/// * `\P` (paragraph break), `\~` (non-breaking space), `\\` (literal
///   backslash), `\{`/`\}` and every other two-character escape are copied
///   as-is;
/// * parameterised codes (`\f…;`, `\H…;`, `\C…;`, …) are copied through their
///   terminating `;`;
/// * `{`/`}` grouping braces are copied and never treated as visible text.
///
/// Word rules (documented, used by Title):
///
/// * a **word** is a whitespace-separated run; punctuation is part of the run,
///   so only its first alphabetic character is upper-cased (`(hello)` →
///   `(Hello)`, `o'neill` → `O'neill`);
/// * `\P` and `\~` also start a new word (they are a line break and a space),
///   while a font/height/colour code does not.
///
/// Sentence case upper-cases the first alphabetic character of the whole
/// string and lower-cases the rest; Toggle swaps each alphabetic character's
/// case. Unicode case mappings are used, so an expansion such as `ß` → `SS`
/// in Upper mode is preserved rather than truncated to ASCII.
pub(crate) fn apply_text_case(text: &str, mode: TextCaseMode) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    let mut word_start = true;
    let mut seen_alpha = false;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\\' {
            out.push('\\');
            index += 1;
            let Some(code) = chars.get(index).copied() else {
                break;
            };
            out.push(code);
            index += 1;
            if matches!(code, 'P' | '~') {
                // A paragraph break / non-breaking space separates words.
                word_start = true;
            } else if is_parameterized_escape(code) {
                if let Some(offset) = chars[index..]
                    .iter()
                    .position(|candidate| *candidate == ';')
                {
                    for candidate in &chars[index..=index + offset] {
                        out.push(*candidate);
                    }
                    index += offset + 1;
                }
            }
            continue;
        }
        if ch == '{' || ch == '}' {
            // Grouping braces are not visible text and do not change words.
            out.push(ch);
            index += 1;
            continue;
        }
        if ch.is_whitespace() {
            out.push(ch);
            word_start = true;
            index += 1;
            continue;
        }
        if ch.is_alphabetic() {
            let converted = match mode {
                TextCaseMode::Upper => ch.to_uppercase().collect::<String>(),
                TextCaseMode::Lower => ch.to_lowercase().collect::<String>(),
                TextCaseMode::Toggle => {
                    if ch.is_uppercase() {
                        ch.to_lowercase().collect()
                    } else {
                        ch.to_uppercase().collect()
                    }
                }
                TextCaseMode::Sentence => {
                    if seen_alpha {
                        ch.to_lowercase().collect()
                    } else {
                        ch.to_uppercase().collect()
                    }
                }
                TextCaseMode::Title => {
                    if word_start {
                        ch.to_uppercase().collect()
                    } else {
                        ch.to_lowercase().collect()
                    }
                }
            };
            out.push_str(&converted);
            seen_alpha = true;
            word_start = false;
            index += 1;
            continue;
        }
        out.push(ch);
        index += 1;
    }
    out
}

/// The stable name of a `TCASE` mode (used in the success diagnostic only).
fn case_mode_name(mode: TextCaseMode) -> &'static str {
    match mode {
        TextCaseMode::Upper => "UPPER",
        TextCaseMode::Lower => "LOWER",
        TextCaseMode::Title => "TITLE",
        TextCaseMode::Sentence => "SENTENCE",
        TextCaseMode::Toggle => "TOGGLE",
    }
}

/// A `torient.not_reset` / `tcase.not_reset` informational diagnostic.
fn not_reset(code: &str, entity: EntityId, kind: &str, reason: &str) -> Diagnostic {
    Diagnostic {
        object: None,
        code: code.into(),
        message: format!(
            "{code}: entity {} ({kind}) was left unchanged: {reason}",
            entity.0
        ),
    }
}

/// The named no-op outcome shared by TORIENT and TCASE.
fn unchanged(code: &str, message: String, diagnostics: Vec<Diagnostic>) -> CommandOutcome {
    let mut diagnostics = diagnostics;
    diagnostics.push(Diagnostic {
        object: None,
        code: code.into(),
        message,
    });
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics,
        measurement: None,
        point_report: None,
    }
}

impl Application {
    /// `TORIENT`: Most Readable (bare) or one absolute rotation.
    ///
    /// One transaction / one undo step for every changed Text entity; a
    /// non-Text selection is reported, never fatal. A selection with no Text
    /// change is `torient.unchanged` and writes nothing.
    pub(crate) fn torient(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let angle_degrees = match &command.payload {
            CommandPayload::None => None,
            CommandPayload::Torient { angle_degrees } => {
                if let Some(degrees) = angle_degrees {
                    if !degrees.is_finite() {
                        return Err(refusal(
                            "torient.invalid_angle",
                            format!("TORIENT angle must be finite, got {degrees}"),
                        ));
                    }
                }
                *angle_degrees
            }
            _ => {
                return Err(refusal(
                    "torient.invalid_payload",
                    "TORIENT takes no payload, or one Torient { angle_degrees } payload",
                ));
            }
        };
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                "torient.needs_selection",
                "TORIENT needs at least one selected Text/MText entity",
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();
        let mut diagnostics = Vec::new();
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::new();
        for reference in &references {
            if reference.document != document_id {
                diagnostics.push(Diagnostic {
                    object: None,
                    code: "torient.not_reset".into(),
                    message: format!(
                        "torient.not_reset: entity {} belongs to another document",
                        reference.entity.0
                    ),
                });
                continue;
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                diagnostics.push(not_reset(
                    "torient.not_reset",
                    reference.entity,
                    "block-instance member or sub-element",
                    "only a directly addressable model-space Text/MText has a writable rotation",
                ));
                continue;
            }
            let Some(before) = drawing.entity(reference.entity).cloned() else {
                diagnostics.push(not_reset(
                    "torient.not_reset",
                    reference.entity,
                    "missing",
                    "the selected entity does not resolve",
                ));
                continue;
            };
            let SemanticGeometry::Text { rotation, .. } = &before.geometry else {
                diagnostics.push(not_reset(
                    "torient.not_reset",
                    before.id,
                    kind_name(&before.geometry),
                    "TORIENT resets Text/MText rotations only in this build",
                ));
                continue;
            };
            let target = match angle_degrees {
                Some(degrees) => degrees.to_radians(),
                None => most_readable_rotation(*rotation),
            };
            if !target.is_finite() {
                diagnostics.push(not_reset(
                    "torient.not_reset",
                    before.id,
                    "TEXT",
                    "the stored rotation is not finite",
                ));
                continue;
            }
            if target == *rotation {
                continue;
            }
            let mut after = before.clone();
            if let SemanticGeometry::Text { rotation, .. } = &mut after.geometry {
                *rotation = target;
            }
            planned.push((before, after));
        }
        let selected = references.len();
        if planned.is_empty() {
            let mode = angle_degrees.map_or_else(
                || "most readable".to_string(),
                |degrees| format!("absolute {degrees} degrees"),
            );
            return Ok(unchanged(
                "torient.unchanged",
                format!(
                    "TORIENT ({mode}): none of the {selected} selected entities changed; \
                     nothing was written"
                ),
                diagnostics,
            ));
        }

        let mut candidate = drawing.clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged =
                candidate.begin_drawing_transaction("text orientation", transaction)?;
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
                .ok_or_else(|| CadError::Invariant("oriented text vanished after commit".into()))?;
            patches.push(drawing_patch(before.id, Some(before.clone()), Some(after)));
            objects.push(ObjectId(before.id.0));
        }
        let changed = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "text orientation".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        let mut all_diagnostics = diagnostics;
        all_diagnostics.push(Diagnostic {
            object: None,
            code: "drawing.torient".into(),
            message: format!(
                "TORIENT: reset {changed} of {selected} selected text entities to {}",
                angle_degrees.map_or_else(
                    || "the most readable rotation".to_string(),
                    |degrees| format!("the absolute rotation {degrees} degrees")
                )
            ),
        });
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: all_diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `TCASE`: change the case of the selected text (Title by default).
    ///
    /// One transaction / one undo step; non-text targets are `tcase.not_reset`
    /// diagnostics and never fail the command when at least one target changed.
    pub(crate) fn tcase(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let mode = match &command.payload {
            CommandPayload::None => TextCaseMode::Title,
            CommandPayload::TCase { mode } => *mode,
            _ => {
                return Err(refusal(
                    "tcase.invalid_payload",
                    "TCASE takes no payload, or one TCase { mode } payload",
                ));
            }
        };
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                "tcase.needs_selection",
                "TCASE needs at least one selected text entity",
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();
        let mut diagnostics = Vec::new();
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::new();
        for reference in &references {
            if reference.document != document_id {
                diagnostics.push(Diagnostic {
                    object: None,
                    code: "tcase.not_reset".into(),
                    message: format!(
                        "tcase.not_reset: entity {} belongs to another document",
                        reference.entity.0
                    ),
                });
                continue;
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                diagnostics.push(not_reset(
                    "tcase.not_reset",
                    reference.entity,
                    "block-instance member or sub-element",
                    "only a directly addressable text entity has a writable string",
                ));
                continue;
            }
            let Some(before) = drawing.entity(reference.entity).cloned() else {
                diagnostics.push(not_reset(
                    "tcase.not_reset",
                    reference.entity,
                    "missing",
                    "the selected entity does not resolve",
                ));
                continue;
            };
            match &before.geometry {
                SemanticGeometry::Text { text, .. } => {
                    let changed = apply_text_case(text, mode);
                    if changed == *text {
                        continue;
                    }
                    let mut after = before.clone();
                    if let SemanticGeometry::Text { text, .. } = &mut after.geometry {
                        *text = changed;
                    }
                    planned.push((before, after));
                }
                SemanticGeometry::MultiLeader(leader)
                    if leader.content() == MLeaderContent::Text =>
                {
                    let changed = apply_text_case(&leader.text().string, mode);
                    if changed == leader.text().string {
                        continue;
                    }
                    match leader.with_text(&changed) {
                        Ok(geometry) => {
                            let mut after = before.clone();
                            after.geometry = SemanticGeometry::MultiLeader(Box::new(geometry));
                            planned.push((before, after));
                        }
                        Err(error) => diagnostics.push(not_reset(
                            "tcase.not_reset",
                            before.id,
                            "MULTILEADER",
                            &format!("its text could not be rewritten: {error:?}"),
                        )),
                    }
                }
                other => diagnostics.push(not_reset(
                    "tcase.not_reset",
                    before.id,
                    kind_name(other),
                    "TCASE changes Text/MText and MULTILEADER text only in this build \
                     (attribute and dimension text are not modelled)",
                )),
            }
        }
        let selected = references.len();
        if planned.is_empty() {
            return Ok(unchanged(
                "tcase.unchanged",
                format!(
                    "TCASE {}: none of the {selected} selected entities changed; nothing was written",
                    case_mode_name(mode)
                ),
                diagnostics,
            ));
        }
        let mut candidate = drawing.clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("text case", transaction)?;
            for (_, after) in &planned {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(planned.len());
        let mut objects = Vec::with_capacity(planned.len());
        for (before, _) in &planned {
            let after = candidate.entity(before.id).cloned().ok_or_else(|| {
                CadError::Invariant("case-changed text vanished after commit".into())
            })?;
            patches.push(drawing_patch(before.id, Some(before.clone()), Some(after)));
            objects.push(ObjectId(before.id.0));
        }
        let changed = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "text case".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        let mut all_diagnostics = diagnostics;
        all_diagnostics.push(Diagnostic {
            object: None,
            code: "drawing.tcase".into(),
            message: format!(
                "TCASE {}: changed {changed} of {selected} selected text entities",
                case_mode_name(mode)
            ),
        });
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: all_diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `TXT2MTXT`: combine the selected Text/MText entities into one `AcDbMText`.
    ///
    /// The selection is the whole input (`CommandPayload::None`). Sources are
    /// sorted top-to-bottom then left-to-right (stable), joined with `\P`
    /// verbatim, and deleted in the same transaction as the new entity. The
    /// settings dialog and uniform line spacing are NOT IMPLEMENTED and are
    /// reported as diagnostics.
    pub(crate) fn txt2mtext(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(refusal(
                "txt2mtxt.invalid_payload",
                "TXT2MTXT takes no payload; the selection is the input",
            ));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(refusal(
                "txt2mtxt.needs_selection",
                "TXT2MTXT needs at least one selected Text/MText entity",
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();
        let mut sources: Vec<DbEntity> = Vec::with_capacity(references.len());
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            if reference.document != document_id {
                return Err(refusal(
                    "txt2mtxt.not_addressable",
                    format!(
                        "TXT2MTXT target {} belongs to another document",
                        reference.entity.0
                    ),
                ));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(refusal(
                    "txt2mtxt.not_addressable",
                    format!(
                        "TXT2MTXT target {} is a block-instance member or sub-element; only \
                         directly addressable model-space text entities can be combined",
                        reference.entity.0
                    ),
                ));
            }
            let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                refusal(
                    "txt2mtxt.target_missing",
                    format!("TXT2MTXT target {} does not exist", reference.entity.0),
                )
            })?;
            if entity.space != SpaceId::Model {
                return Err(CadError::Unsupported(format!(
                    "txt2mtxt.unsupported: entity {} is not model space; v1 combines model-space \
                     text only",
                    entity.id.0
                )));
            }
            if !matches!(entity.geometry, SemanticGeometry::Text { .. }) {
                return Err(CadError::Unsupported(format!(
                    "txt2mtxt.unsupported: {} is not a Text/MText entity; the whole command is \
                     refused and nothing is converted",
                    kind_name(&entity.geometry)
                )));
            }
            if seen.insert(entity.id) {
                sources.push(entity);
            }
        }
        // Top-to-bottom (descending Y), then left-to-right (ascending X). The
        // sort is stable, so equal anchors keep the selection order.
        sources.sort_by(|first, second| {
            let (
                SemanticGeometry::Text { position: a, .. },
                SemanticGeometry::Text { position: b, .. },
            ) = (&first.geometry, &second.geometry)
            else {
                return std::cmp::Ordering::Equal;
            };
            b.y.total_cmp(&a.y).then_with(|| a.x.total_cmp(&b.x))
        });
        let first = sources
            .first()
            .ok_or_else(|| CadError::Invariant("TXT2MTXT lost its source set".into()))?;
        let SemanticGeometry::Text {
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            wrap_width,
            ..
        } = &first.geometry
        else {
            return Err(CadError::Invariant(
                "TXT2MTXT first source is not Text".into(),
            ));
        };
        let mut joined = String::new();
        for (index, source) in sources.iter().enumerate() {
            let SemanticGeometry::Text { text, .. } = &source.geometry else {
                return Err(CadError::Invariant("TXT2MTXT source is not Text".into()));
            };
            if index > 0 {
                joined.push_str("\\P");
            }
            joined.push_str(text);
        }
        let geometry = SemanticGeometry::Text {
            text: joined,
            position: *position,
            style: *style,
            height: *height,
            rotation: *rotation,
            font: font.clone(),
            h_align: *h_align,
            v_align: *v_align,
            wrap_width: *wrap_width,
        };
        let mut candidate = drawing.clone();
        let new_id = candidate.allocate_entity_id();
        // Preserve the first sorted source's draw order (the new MTEXT replaces
        // that source in the draw stack instead of jumping above every entity).
        let entity = DbEntity {
            object: cad_db::DbObject {
                id: ObjectId(new_id.0),
                type_key: text_policy::MTEXT_TYPE_KEY.into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: new_id,
            layer: first.layer,
            space: first.space.clone(),
            geometry,
            draw_order: first.draw_order,
        };
        let display_states: Vec<_> = sources
            .iter()
            .map(|source| candidate.entity_display_state(source.id))
            .collect();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("combine text", transaction)?;
            for source in &sources {
                staged.delete_entity(source.id)?;
            }
            staged.insert_entity(entity.clone())?;
            staged.commit()?
        };
        let mut patches: Vec<DrawingPatch> = sources
            .iter()
            .zip(display_states)
            .map(|(source, state)| drawing_patch_deleted(source.id, source.clone(), state))
            .collect();
        let committed = candidate
            .entity(new_id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("combined MTEXT vanished after commit".into()))?;
        patches.push(drawing_patch(new_id, None, Some(committed)));
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "combine text".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: vec![ObjectId(new_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![
                Diagnostic {
                    object: Some(ObjectId(new_id.0)),
                    code: "drawing.txt2mtxt".into(),
                    message: format!(
                        "TXT2MTXT: combined {} text entities into one AcDbMText at the first \
                         source's position",
                        sources.len()
                    ),
                },
                Diagnostic {
                    object: None,
                    code: "txt2mtxt.settings_dialog".into(),
                    message: "TXT2MTXT: the AutoCAD settings dialog (single mtext, sort top-down, \
                              word-wrap, force uniform line spacing) is NOT IMPLEMENTED in v1; \
                              this build always joins every source into one MTEXT"
                        .into(),
                },
                Diagnostic {
                    object: None,
                    code: "txt2mtxt.uniform_line_spacing".into(),
                    message: "TXT2MTXT: forcing uniform line spacing is NOT IMPLEMENTED; the \
                              stored string and the first source's text style are preserved"
                        .into(),
                },
            ],
            measurement: None,
            point_report: None,
        })
    }
}
