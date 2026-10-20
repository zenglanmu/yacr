//! TEXTEDIT: replace the string of an existing text-bearing entity
//! (ADR 0020 amendment options A + B; ADR-0021 D4 generalises it to MTEXT and
//! MULTILEADER text).
//!
//! Editing is deliberately **not** a second creation path: it re-reads the
//! addressed entity inside the command, changes only its `text` field and stages
//! one `update_entity` in one transaction / one undo step, exactly like
//! TRIM/EXTEND/STRETCH/CHAMFER. Every refusal is named and happens before the
//! transaction is opened, so a stale or wrong target writes nothing.
//!
//! The target must be an **entity**, not a sub-object: a ref carrying an
//! instance path (a block-instance member) or a sub-element is refused instead
//! of being reached into. Compound children (which is where block/tolerance
//! MULTILEADER content stays) have no addressable identity here.
//!
//! The **newline policy** lives here, in the one command both entry paths
//! (`TEXTEDIT` and the panel's `SetSelectedText`) share, so the UI cannot
//! disagree with the stored string (ADR-0021 D4):
//!
//! * `AcDbMText` and MULTILEADER text treat a line break as a paragraph break:
//!   `\r\n`, `\r` and `\n` are stored as `\P`, the only form the viewport
//!   interprets (`cad-representation`'s MTEXT parser);
//! * `AcDbText` does **not** interpret `\P`, so a draft containing `\n`/`\r` is
//!   refused by name (`textedit.newline_text`) and nothing is written;
//! * every other code (`{`, `}`, `\f…;`, `\W…;`, …) passes through verbatim —
//!   this edits the raw string, it is not a rich-text editor.

use super::*;
use cad_domain::MLeaderContent;

impl Application {
    /// TEXTEDIT: replace the stored string of one directly addressable
    /// TEXT / MTEXT / MULTILEADER text (ADR 0020 amendment A/B, ADR-0021 D4).
    ///
    /// The payload's `target` is the ref resolved at pick time (interactive
    /// `TEXTEDIT`) or the current single selection (the panel path). The entity
    /// is re-read here, so a target that was deleted, replaced or retyped since
    /// the pick is refused by name and opens no transaction.
    ///
    /// Only the string changes: for `Text`, `style`/`height`/`rotation`/
    /// `position`/`font` and the two alignment fields are carried over
    /// byte-for-byte; for a MULTILEADER, `override_mask`, the style snapshot,
    /// `MLeaderText`'s width/location/rotation/height/attachment, the roots,
    /// lines, `reserved` bag and plane frame are preserved through
    /// [`cad_domain::MultiLeaderGeometry::with_text`], which also **rebuilds the
    /// derived display** so it can never disagree with the new string. The edit
    /// is one `update_entity` in one transaction / one undo step.
    pub(crate) fn edit_text(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::TextEdit { target, text } = &command.payload else {
            return Err(CadError::InvalidInput(
                "TEXTEDIT needs a TextEdit { target, text } payload".into(),
            ));
        };
        if target.document != command.document {
            return Err(CadError::InvalidInput(
                "TEXTEDIT target belongs to another document".into(),
            ));
        }
        // The same content refusals as the TEXT channel, re-validated here so a
        // programmatic caller cannot bypass the capture-time bound.
        if text.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "tool.text_empty: TEXTEDIT needs a non-empty, non-whitespace text".into(),
            ));
        }
        let characters = text.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(CadError::InvalidInput(format!(
                "tool.text_too_long: TEXTEDIT allows at most {MAX_TOOL_TEXT_CHARS} characters, \
                 got {characters}"
            )));
        }

        let document_id = command.document;
        let (drawing, before) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let before = resolve_text_edit_target(document.drawing.as_ref(), target)?;
            (document.drawing.clone(), before)
        };

        // The newline policy is applied once, here, before anything is staged.
        let stored = apply_newline_policy(&before, text)?;

        // Build the replacement from the stored entity, changing only the string.
        let mut after = before.clone();
        after.geometry = match &before.geometry {
            SemanticGeometry::Text {
                position,
                style,
                height,
                rotation,
                font,
                h_align,
                v_align,
                ..
            } => SemanticGeometry::Text {
                text: stored.clone(),
                position: *position,
                style: *style,
                height: *height,
                rotation: *rotation,
                font: font.clone(),
                h_align: *h_align,
                v_align: *v_align,
            },
            SemanticGeometry::MultiLeader(leader) => {
                SemanticGeometry::MultiLeader(Box::new(leader.with_text(&stored)?))
            }
            // `resolve_text_edit_target` guarantees this; keep an explicit
            // refusal instead of an unreachable panic.
            other => {
                return Err(CadError::Unsupported(format!(
                    "textedit.target_not_text: {} is not a TEXT entity",
                    geometry_kind(other)
                )))
            }
        };

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("edit text", transaction)?;
            staged.update_entity(after)?;
            staged.commit()?
        };
        // Read the committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let after = candidate
            .entity(before.id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("edited text vanished after commit".into()))?;
        let object = ObjectId(before.id.0);
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "edit text".into(),
                patches: vec![drawing_patch(before.id, Some(before), Some(after))],
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: vec![object],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(object),
                code: "drawing.textedit".into(),
                message: "TEXTEDIT: replaced the text of one text entity".into(),
            }],
            measurement: None,
        })
    }
}

/// Apply the ADR-0021 D4 newline policy to one draft, keyed by the target's
/// **source class** (never by guessing from the string).
///
/// MULTILEADER text and `AcDbMText` convert `\r\n`/`\r`/`\n` to `\P`; `AcDbText`
/// refuses them (`textedit.newline_text`) because AutoCAD's TEXT does not
/// interpret `\P`; any other control code passes through verbatim.
fn apply_newline_policy(entity: &DbEntity, text: &str) -> CadResult<String> {
    match &entity.geometry {
        SemanticGeometry::MultiLeader(_) => Ok(newlines_to_paragraph_breaks(text)),
        SemanticGeometry::Text { .. } if is_multiline_text_source(entity) => {
            Ok(newlines_to_paragraph_breaks(text))
        }
        SemanticGeometry::Text { .. } => {
            if text.contains(['\r', '\n']) {
                return Err(CadError::InvalidInput(
                    "textedit.newline_text: TEXT is single-line and does not interpret \\P; \
                     use MTEXT or a MULTILEADER for multiple lines"
                        .into(),
                ));
            }
            Ok(text.to_string())
        }
        other => Err(CadError::Unsupported(format!(
            "textedit.target_not_text: {} is not a text entity",
            geometry_kind(other)
        ))),
    }
}

/// `\r\n`, `\r` and `\n` → `\P`, in one pass.
///
/// `\P` is the only paragraph-break form the viewport's MTEXT parser
/// (`cad-representation`) interprets, so the stored string stays consistent
/// between our renderer and AutoCAD. No other code is inspected or rewritten.
fn newlines_to_paragraph_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                // A CRLF pair is one line break, not two.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\P");
            }
            '\n' => out.push_str("\\P"),
            other => out.push(other),
        }
    }
    out
}

/// Resolve the one `TEXTEDIT` target: an existing, **directly addressable**
/// TEXT / MTEXT entity or a MULTILEADER carrying text content.
///
/// Named refusals (the caller opens no transaction on any of them):
///
/// * `textedit.target_missing` — the ref no longer resolves in this drawing;
/// * `textedit.target_not_text` — an instance path/sub-element (a block member
///   or sub-object, which has no entity identity of its own), a `Compound`
///   (where block/tolerance leader content stays) or any other geometry kind;
/// * `textedit.mleader_content_not_text` — a first-class MULTILEADER whose
///   content is not [`MLeaderContent::Text`] (a content-less leader), which has
///   no string to replace.
///
/// It is shared by the interactive pick (which uses it to fill the prefill) and
/// the command (which re-validates at apply time), so the two can never drift.
pub(crate) fn resolve_text_edit_target(
    drawing: &DrawingDatabase,
    target: &SelectionRef,
) -> CadResult<DbEntity> {
    if !target.instance.0.is_empty() || target.sub_element.is_some() {
        return Err(CadError::Unsupported(
            "textedit.target_not_text: block-instance members and sub-elements are not \
             addressable text entities"
                .into(),
        ));
    }
    let entity = drawing.entity(target.entity).cloned().ok_or_else(|| {
        CadError::InvalidInput(format!(
            "textedit.target_missing: TEXTEDIT target {} does not exist",
            target.entity.0
        ))
    })?;
    match &entity.geometry {
        SemanticGeometry::Text { .. } => Ok(entity),
        SemanticGeometry::MultiLeader(leader) if leader.content() == MLeaderContent::Text => {
            Ok(entity)
        }
        SemanticGeometry::MultiLeader(leader) => Err(CadError::Unsupported(format!(
            "textedit.mleader_content_not_text: the MULTILEADER carries {:?} content, not text",
            leader.content()
        ))),
        other => Err(CadError::Unsupported(format!(
            "textedit.target_not_text: {} is not a TEXT entity",
            geometry_kind(other)
        ))),
    }
}
