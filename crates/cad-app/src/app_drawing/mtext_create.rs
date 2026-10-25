//! MTEXT creation by command/payload (`CommandId::CreateMText`).
//!
//! This module is the **command core**: the payload carries a fully formed MTEXT
//! (insertion point, optional wrap width, text, glyph height, rotation and the two
//! alignment/attachment fields) and the handler writes one `AcDbMText` entity
//! through the shared create funnel in one transaction / one undo step. The
//! interactive side — `DrawToolKind::MText` (first corner → opposite corner →
//! typed text), `BeginDraw` routing, the ribbon entry and the prompts — lives in
//! `draw_tool`/`host`/`cad-ui-core` and reaches this handler through the ordinary
//! `CommandId::CreateMText` + payload path; there is exactly one creation path.
//!
//! What the payload **carries** (everything else is refused, never invented):
//!
//! * `text` — stored with `\r\n`/`\r`/`\n` converted to `\P` by the one shared
//!   policy in [`super::text_policy`], the same rule `TEXTEDIT` applies, so a
//!   created MTEXT is editable through `TEXTEDIT` afterwards;
//! * `position` — the insertion point (must be finite);
//! * `width` — the automatic wrap boundary (the DXF group-41 `rectangle_width`
//!   equivalent). `None` means "no wrap"; `Some(w)` must be finite and `> 0` and
//!   goes through [`wrap_width_from_dxf`] (the single sanitization rule), so the
//!   value is never clamped and never fabricated. This is the **two-corner width
//!   model**: the caller derives `width` from the second corner; the command does
//!   not do the capture itself;
//! * `height` — glyph height in world units (finite, `> 0`, the same bound as
//!   `TEXT`);
//! * `rotation` — radians (finite; `0.0` is the unrotated case);
//! * `h_align` / `v_align` — the horizontal/vertical attachment of the text
//!   relative to `position`.
//!
//! What is **NOT modelled** and therefore not accepted (the four stable reasons
//! the importer already names — `cad_import_acadrust::entity::convert`'s
//! `MTEXT_UNMODELLED`): line-spacing style (`text.line_spacing_style_unsupported`),
//! columns (`text.columns_unsupported`), background fill
//! (`text.background_unsupported`) and drawing direction
//! (`text.drawing_direction_unsupported`). There is no payload field for any of
//! them, and the create diagnostic repeats the list so the omission is stated
//! rather than implied.
//!
//! The entity's stored `type_key` is **`AcDbMText`**, not `class_name()`'s
//! `AcDbText`: the create funnel takes an explicit type-key override, which is
//! what makes the newline policy (`is_multiline_text_source`), the export path
//! (MTEXT re-export) and `TEXTEDIT` agree on the same entity. Nothing is
//! downgraded to TEXT.

use super::text_policy::{newlines_to_paragraph_breaks, MTEXT_TYPE_KEY};
use super::*;

impl Application {
    /// Insert one multiline text (`AcDbMText`) into model space on the session's
    /// active layer.
    ///
    /// Refusals (nothing written, no transaction, no undo entry):
    ///
    /// * a payload other than `MText { … }` (a bare dispatch; the interactive
    ///   `DrawToolKind::MText` tool always supplies the full payload);
    /// * a non-finite insertion point;
    /// * an empty or whitespace-only text (`tool.text_empty`), again without
    ///   `\P`/line-break conversion having happened first;
    /// * more than [`MAX_TOOL_TEXT_CHARS`] characters, counted in **characters**
    ///   (not bytes), matching the other text channels;
    /// * a non-finite or non-positive `height`;
    /// * a non-finite `rotation`;
    /// * a `width` that is present but not finite and positive
    ///   (`CreateMText width must be finite and positive …`), never silently
    ///   dropped to "no wrap";
    /// * a missing active layer (the shared create funnel).
    pub(crate) fn create_mtext(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::MText {
            position,
            width,
            text,
            height,
            rotation,
            h_align,
            v_align,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "MTEXT needs an MText { position, width, text, height, rotation, h_align, v_align } \
                 payload; the interactive two-corner MTEXT tool builds one, and a bare dispatch \
                 carries none"
                    .into(),
            ));
        };
        if !is_finite_point(*position) {
            return Err(CadError::InvalidInput(
                "CreateMText position must be finite".into(),
            ));
        }
        // Checked **before** the newline conversion, so a whitespace-only draft
        // reports the empty-text reason and not a paragraph-break result.
        if text.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "tool.text_empty: CreateMText needs a non-empty, non-whitespace text".into(),
            ));
        }
        let characters = text.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(CadError::InvalidInput(format!(
                "tool.text_too_long: CreateMText allows at most {MAX_TOOL_TEXT_CHARS} characters, \
                 got {characters}"
            )));
        }
        if !height.is_finite() || *height <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "CreateMText height must be finite and positive, got {height}"
            )));
        }
        if !rotation.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "CreateMText rotation must be finite, got {rotation}"
            )));
        }
        // `None` stays "no wrap". `Some(w)` must be a usable boundary: the DXF
        // sanitization rule maps an unusable value to `None`, and silently doing
        // that here would fabricate a different layout, so it is a refusal.
        let wrap_width = match width {
            None => None,
            Some(width) => Some(wrap_width_from_dxf(*width).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "CreateMText width must be finite and positive when given, got {width}; \
                     omit the width for an unwrapped text"
                ))
            })?),
        };
        // The one style path both text-creation commands use, resolved from the
        // authoritative drawing (never a fabricated style identity or face).
        let (style, font) = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            resolve_created_text_style(document.drawing.as_ref())
        };
        let geometry = SemanticGeometry::Text {
            text: newlines_to_paragraph_breaks(text),
            position: *position,
            style,
            height: *height,
            rotation: *rotation,
            font,
            h_align: *h_align,
            v_align: *v_align,
            wrap_width,
        };
        let mut outcome = self.commit_new_entity_typed(
            session,
            command,
            geometry,
            MTEXT_TYPE_KEY,
            "create mtext",
        )?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "drawing.mtext_created".into(),
            message: format!(
                "MTEXT: one {MTEXT_TYPE_KEY} entity; NOT MODELLED and not accepted by this \
                 command: line spacing style (text.line_spacing_style_unsupported), columns \
                 (text.columns_unsupported), background fill (text.background_unsupported), \
                 drawing direction (text.drawing_direction_unsupported)"
            ),
        });
        Ok(outcome)
    }
}
