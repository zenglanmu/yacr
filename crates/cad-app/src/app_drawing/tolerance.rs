//! `TOLERANCE`: build one feature-control frame as real geometry.
//!
//! ## Representation (deliberate divergence)
//!
//! This build has no single `AcDbFcf` (feature-control-frame) object: the
//! `cad-domain` geometry set has no tolerance variant and none is added here.
//! The command therefore writes the frame as **separate ordinary entities** on
//! the active layer, in one transaction / one undo step:
//!
//! * one **closed `LWPOLYLINE`** border rectangle (4 vertices, no bulge);
//! * one **`LINE` divider** per internal compartment boundary;
//! * one **`TEXT`** per compartment, vertically centred and left-padded inside
//!   its cell (left/baseline would sit on the bottom edge).
//!
//! That is a faithful *drawing* of a feature-control frame, but it is not an
//! associative tolerance object: `TEXTEDIT` edits the individual cell text and
//! moving the frame means selecting all of its parts. The residual is reported by
//! `tolerance.representation` on every success.
//!
//! ## Input
//!
//! `TOLERANCE <x> <y> <text>`: `(x, y)` is the frame's **lower-left corner** (the
//! same insertion convention the importer's `tolerance_frame` uses) and `<text>`
//! is the typed frame text. The text splits on `|` into compartments, e.g.
//! `TOLERANCE 0 0 "|0.05|A|B"` produces four cells. There is no
//! orientation/rotation input, so the frame is axis-aligned on the world XY plane.
//!
//! The cell width and row height use the same declared estimate the importer's
//! frame uses — width `chars · height · 0.62 + height`, row height
//! `height · 1.6` — because exact glyph metrics are a representation-layer
//! concern. The rule itself lives in `cad_domain`
//! (`tolerance_frame_geometries`), which is also what a MULTILEADER's tolerance
//! content draws, so the two frames cannot drift apart. The height is the session
//! `textHeight` typed channel, not a payload field.
//!
//! ## Refusals (named, nothing written, no transaction)
//!
//! * `tolerance.payload` — a payload that is not `Tolerance { .. }`;
//! * `tolerance.origin` — a non-finite insertion point;
//! * `tolerance.text` — empty or whitespace-only frame text;
//! * `tolerance.text_too_long` — more than the shared
//!   [`MAX_TOOL_TEXT_CHARS`] characters;
//! * `tolerance.text_control` — a control character (including a line break);
//! * `tolerance.gdt_symbol` — the DXF GDT-font notation (`{\Fgdt;…}`); this
//!   build has no GDT symbol-font interpreter, so it refuses instead of drawing
//!   the escape literally;
//! * `tolerance.multiline` — the DXF `^J` multi-row separator; this command
//!   builds one single-row frame;
//! * `tolerance.dxf_separator` — the DXF `%%v` cell separator; the typed command
//!   uses `|`, and treating a DXF string as one literal cell would be wrong;
//! * `tolerance.cell_empty` — an empty compartment between `|` separators;
//! * `tolerance.cells` — more than [`MAX_TOLERANCE_CELLS`] compartments;
//! * `tolerance.height` — the session text height is not finite/positive.
//!
//! ## Why this stays a declared partial (not a promotion)
//!
//! The decomposition itself is a faithful drawing, but the **cell grammar** is
//! not the full feature-control-frame grammar: AutoCAD's dialog has two
//! tolerance slots, three datum slots, a projected-zone height and a datum
//! identifier, and the source text uses GDT symbol font escapes and `%%v`
//! separators. This command supports one single-row frame of up to six plain-text
//! `|`-separated cells and refuses the DXF notation by name. The composite
//! `AcDbFcf` entity class is also not modelled (declared representation choice),
//! but the grammar/limit gap above is real, so the command remains **partial**
//! until a GDT symbol grammar (or a documented plain-text equivalent) is
//! implemented.

use super::*;
use crate::draw_tool::ToolParameter;
use cad_domain::{Point3, SemanticGeometry, StyleId};

/// The compartment cap of one frame. A feature-control frame has at most the
/// symbol, the tolerance, three datums and one optional extra cell; a longer
/// string is a typed mistake, not a wider frame.
const MAX_TOLERANCE_CELLS: usize = 6;

/// A named invalid-input refusal (`tolerance.<code>: …`).
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("tolerance.{code}: {detail}"))
}

impl Application {
    /// `TOLERANCE`: insert one feature-control frame as border + dividers + cell
    /// text on the active layer.
    ///
    /// Refusals are listed in the module docs; every one happens before the
    /// transaction, so a refusal writes nothing and records no history.
    pub(crate) fn tolerance_frame(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Tolerance(payload) = &command.payload else {
            return Err(invalid(
                "payload",
                "TOLERANCE needs a Tolerance { origin, text } payload",
            ));
        };
        if !is_finite_point(payload.origin) {
            return Err(invalid(
                "origin",
                "the frame insertion point (lower-left corner) must be finite",
            ));
        }
        if payload.text.trim().is_empty() {
            return Err(invalid(
                "text",
                "a feature-control frame needs non-empty, non-whitespace text",
            ));
        }
        let characters = payload.text.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(invalid(
                "text_too_long",
                format!(
                    "TOLERANCE allows at most {MAX_TOOL_TEXT_CHARS} characters, got {characters}"
                ),
            ));
        }
        if payload.text.chars().any(char::is_control) {
            return Err(invalid(
                "text_control",
                "the frame text must not contain control characters (including line breaks); \
                 compartments are separated by `|`",
            ));
        }
        if payload.text.contains("{\\Fgdt;") {
            return Err(invalid(
                "gdt_symbol",
                "the DXF GDT-font notation (`{\\Fgdt;…}`) is not rendered by this build; type the \
                 symbol as plain text instead",
            ));
        }
        if payload.text.contains("^J") {
            return Err(invalid(
                "multiline",
                "the DXF `^J` multi-row separator is not supported: TOLERANCE builds one \
                 single-row feature-control frame",
            ));
        }
        if payload.text.contains("%%v") {
            return Err(invalid(
                "dxf_separator",
                "the DXF `%%v` cell separator is not accepted: the typed frame uses `|` between \
                 compartments",
            ));
        }
        let cells: Vec<&str> = payload.text.split('|').collect();
        if cells.len() > MAX_TOLERANCE_CELLS {
            return Err(invalid(
                "cells",
                format!(
                    "TOLERANCE allows at most {MAX_TOLERANCE_CELLS} compartments, got {}",
                    cells.len()
                ),
            ));
        }
        for cell in &cells {
            if cell.trim().is_empty() {
                return Err(invalid(
                    "cell_empty",
                    "every compartment between `|` separators needs text; an empty cell has no \
                     content to frame",
                ));
            }
        }
        let height = session
            .tool_parameter(ToolParameter::TextHeight)
            .number()
            .unwrap_or(2.5);
        if !height.is_finite() || height <= 0.0 {
            return Err(invalid(
                "height",
                format!(
                    "the session text height must be finite and positive for the frame, got \
                     {height}"
                ),
            ));
        }
        let (style, font) = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            resolve_created_text_style(document.drawing.as_ref())
        };
        let geometries = frame_geometries(payload.origin, &cells, height, style, font)?;
        let mut outcome =
            self.commit_new_entities(session, command, geometries, "create tolerance")?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "tolerance.representation".into(),
            message: format!(
                "TOLERANCE: built a feature-control frame from separate entities (1 border \
                 polyline + {} divider line(s) + {} TEXT row(s)) on the active layer; this build \
                 stores no single AcDbFcf tolerance object",
                cells.len().saturating_sub(1),
                cells.len()
            ),
        });
        Ok(outcome)
    }
}

/// Build border + divider + cell-text geometry for one frame.
///
/// The frame's **single rule** lives in `cad_domain` (`tolerance_frame_geometries`):
/// the same estimate sizes the `TOLERANCE` command's entities and a MULTILEADER's
/// tolerance content, so the two can never draw different frames.
fn frame_geometries(
    origin: Point3,
    cells: &[&str],
    height: f64,
    style: StyleId,
    font: Option<String>,
) -> CadResult<Vec<SemanticGeometry>> {
    let cells: Vec<String> = cells.iter().map(|cell| (*cell).to_string()).collect();
    cad_domain::tolerance_frame_geometries(origin, &cells, height, style, font)
}
