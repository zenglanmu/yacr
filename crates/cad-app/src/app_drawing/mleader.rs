//! The `MLEADER` creation command (ADR 0019 decision F.2, P7c).
//!
//! Split out of `drawing_create.rs` to keep that module inside the code-size
//! limit. This is an inherent method on [`Application`], so the dispatch in
//! `drawing_create.rs` reads exactly like the other create commands.
//!
//! The command is the single construction point: it resolves the drawing's
//! `MLeaderStyle` table (decision D), asks [`MultiLeaderGeometry::created`] for
//! the definition (one root, one line, two straight vertices) and inserts one
//! `AcDbMultiLeader` through the shared create funnel — one transaction, one undo
//! step, the active layer. A refusal opens no transaction, so the drawing and the
//! history are untouched.
//!
//! Deliberate subset (see [`crate::CommandId::MLeader`]): **no `Options`
//! sub-prompt** (leader type / content type / maximum points / angular
//! constraint), no style selector, no dogleg or attachment override, and **no
//! content-less creation** — a MULTILEADER without text is import/display/export
//! only (decision F.2).

use super::*;
use cad_db::MLeaderStyle;

/// The multileader style `MLEADER` creation resolves by name (ADR 0019 D).
///
/// There is no style selector in the capture, so `Standard` is the one name the
/// command reads; the table always carries it (the builder pins it).
const MLEADER_STYLE_NAME: &str = "Standard";

impl Application {
    /// Insert one straight-line multileader into model space on the active layer.
    ///
    /// Everything but the two points and the text comes from the drawing's
    /// `MLeaderStyle` row: dogleg length, landing gap, arrowhead size, text
    /// height, text style, vertical attachment, path type and content kind. The
    /// resolved snapshot is stored on the entity (P7b-1) — there is no second
    /// source, because the same resolution feeds both the definition and the
    /// export's style record.
    ///
    /// Refusals (named, nothing written, no transaction):
    ///
    /// * `mleader.payload` — a payload that is not `MLeader { .. }`;
    /// * `tool.text_empty` / `tool.text_too_long` — the text rules shared with
    ///   `TEXT` (empty/whitespace-only or more than
    ///   [`MAX_TOOL_TEXT_CHARS`] characters);
    /// * `mleader.style_missing` — the drawing has no `Standard` MLEADERSTYLE
    ///   (the builder pins one, so this is a corrupt-database case);
    /// * `mleader.text_required` / `mleader.content_unsupported` — the resolved
    ///   style declares no content (content-less leaders are not created) or
    ///   block/tolerance content (stays on the import path);
    /// * every refusal [`MultiLeaderGeometry::created`] raises: non-finite or
    ///   non-`+Z`-coplanar points, a landing point no farther than the style's
    ///   dogleg length, a spline/invisible path type, or an unusable style size;
    /// * a missing active layer (the shared create funnel).
    pub(crate) fn create_mleader(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::MLeader {
            arrow,
            landing,
            text,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "mleader.payload: CreateMLeader needs a MLeader { arrow, landing, text } payload"
                    .into(),
            ));
        };
        if text.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "tool.text_empty: a multileader needs a non-empty, non-whitespace text; \
                 content-less leaders are not created"
                    .into(),
            ));
        }
        let characters = text.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(CadError::InvalidInput(format!(
                "tool.text_too_long: MLEADER allows at most {MAX_TOOL_TEXT_CHARS} characters, \
                 got {characters}"
            )));
        }
        let (resolution, content_type, style_name) = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            let row = drawing
                .mleader_style_by_name(MLEADER_STYLE_NAME)
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "mleader.style_missing: the drawing has no MLEADERSTYLE named \
                         '{MLEADER_STYLE_NAME}'"
                    ))
                })?;
            let content_type = row.content_type;
            // Store the row's own name, not the constant, so a source row with a
            // different case is not silently renamed on the entity (and in the
            // written MLEADERSTYLE record).
            let style_name = row.name.clone();
            let resolution = drawing
                .resolve_mleader_style(MLEADER_STYLE_NAME)
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "mleader.style_missing: the MLEADERSTYLE '{MLEADER_STYLE_NAME}' did not \
                         resolve"
                    ))
                })?;
            (resolution, content_type, style_name)
        };
        match content_type {
            MLeaderStyle::CONTENT_MTEXT => {}
            MLeaderStyle::CONTENT_NONE => {
                return Err(CadError::InvalidInput(
                    "mleader.text_required: the resolved MLEADERSTYLE declares no content; a \
                     content-less MULTILEADER is only imported, displayed and exported"
                        .into(),
                ));
            }
            MLeaderStyle::CONTENT_BLOCK | MLeaderStyle::CONTENT_TOLERANCE => {
                return Err(CadError::Unsupported(format!(
                    "mleader.content_unsupported: the resolved MLEADERSTYLE declares content kind \
                     {content_type} (block/tolerance); only MText content is created"
                )));
            }
            other => {
                return Err(CadError::Unsupported(format!(
                    "mleader.content_unsupported: the resolved MLEADERSTYLE declares unknown \
                     content kind {other}; only MText content is created"
                )));
            }
        }
        let geometry = MultiLeaderGeometry::created(
            resolution.values,
            style_name,
            resolution.left_attachment,
            *arrow,
            *landing,
            text.clone(),
        )?;
        self.commit_new_entity(
            session,
            command,
            SemanticGeometry::MultiLeader(Box::new(geometry)),
            "create mleader",
        )
    }
}
