//! Long-form contracts of the two text-creation payloads.
//!
//! [`crate::CommandPayload::Text`] and [`crate::CommandPayload::MText`] are
//! declared in the parent module (an enum cannot be split across files); their
//! documentation lives here so `command_payload.rs` stays inside the 1000-line
//! code gate. Nothing in this file is behaviour: each section is the full
//! contract the command layer implements, moved verbatim, and the enum's own
//! doc comments point here.
//!
//! # `Text`: one single-line text entity (ADR 0019 F.1, P7a)
//!
//! `position` is the captured insertion point, `text` the typed string
//! (non-empty, at most [`crate::draw_tool::MAX_TOOL_TEXT_CHARS`] characters —
//! re-validated by the command so a programmatic caller cannot bypass the
//! capture), and `height` its glyph height in world units (finite, `> 0`). The
//! capture reads the height from [`crate::draw_tool::ToolParameter::TextHeight`];
//! it is a payload field because the command is the single construction point and
//! the value must travel with it (never re-read from a session the caller does not
//! own). The **rotation** deliberately has no payload field: the command reads it
//! from the session it is handed through the typed channel
//! [`crate::draw_tool::ToolParameter::TextRotation`] (**degrees**, any finite
//! value) and stores the radians, so a session at its default stores `0.0` exactly
//! like the previously fixed unrotated text. The style is the drawing's `Standard`
//! text style with the style default alignment (left/baseline).
//!
//! Single-line means single-line: a `text` containing `\r`/`\n` is **refused**
//! (`text.single_line_newline`), because an `AcDbText` does not interpret `\P` —
//! storing one would render over several lines and could never be re-edited
//! through `TEXTEDIT`. Use [`crate::CommandPayload::MText`] for multi-line text.
//!
//! # `MText`: one multiline text entity stored as `AcDbMText`
//!
//! This is the command core: the interactive `DrawToolKind::MText` tool (first
//! corner → opposite corner → typed text) builds exactly this payload, and a
//! direct caller (CLI, host IPC) supplies the whole construction:
//!
//! * `text` — the draft string. `\r\n`, `\r` and `\n` are stored as `\P`, the
//!   form the viewport's MTEXT parser and AutoCAD both interpret, by the one
//!   newline policy `TEXTEDIT` shares. A later `TEXTEDIT` of this entity accepts
//!   line breaks again because the entity is `AcDbMText`;
//! * `position` — insertion point (world, must be finite);
//! * `width` — the automatic wrap boundary (DXF group `41` `rectangle_width`;
//!   the **two-corner** model: the caller passes the width it captured from the
//!   second corner). `None` = no wrap and is stored as `None`; `Some(w)` must be
//!   finite and `> 0` and is refused otherwise (never clamped, never dropped) and
//!   sanitized through [`cad_domain::wrap_width_from_dxf`] like every other wrap
//!   boundary;
//! * `height` — glyph height in world units (finite, `> 0`);
//! * `rotation` — rotation in radians (finite; `0.0` = unrotated). The
//!   interactive capture fills it from
//!   [`crate::draw_tool::ToolParameter::TextRotation`] (**degrees**, any finite
//!   value) and converts once; a programmatic caller supplies the radians
//!   directly, exactly as before this parameter existed;
//! * `h_align` / `v_align` — horizontal/vertical attachment relative to
//!   `position`. The interactive capture fills them from
//!   [`crate::draw_tool::ToolParameter::MTextAttachment`] (AutoCAD's `1..=9`
//!   codes; `1` = Left/Top, `5` = Center/Middle, `9` = Right/Bottom), so
//!   `TextAlignV::Baseline` is never produced by the interactive path.
//!
//! **Not carried, because not modelled**: columns, background fill and drawing
//! direction (the importer's `MTEXT_UNMODELLED` facets). No field exists for any
//! of them, so none can be believed to be honoured. Line spacing is modelled
//! (`SemanticGeometry::Text::line_spacing`) but this payload has no field for it:
//! a created MTEXT stores the neutral `1.0 / at least`, the value the layout has
//! always used.
//!
//! The style/font resolution is the one `Text` creation uses, and the stored
//! `type_key` is `AcDbMText` (not `Text`'s `AcDbText`), so this command can never
//! produce a single-line entity that claims to be multiline.
