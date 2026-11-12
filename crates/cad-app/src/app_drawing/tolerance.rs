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
//! concern. The height is the session `textHeight` typed channel, not a payload
//! field.
//!
//! ## Refusals (named, nothing written, no transaction)
//!
//! * `tolerance.payload` — a payload that is not `Tolerance { .. }`;
//! * `tolerance.origin` — a non-finite insertion point;
//! * `tolerance.text` — empty or whitespace-only frame text;
//! * `tolerance.text_too_long` — more than the shared
//!   [`MAX_TOOL_TEXT_CHARS`] characters;
//! * `tolerance.text_control` — a control character (including a line break);
//! * `tolerance.cell_empty` — an empty compartment between `|` separators;
//! * `tolerance.cells` — more than [`MAX_TOLERANCE_CELLS`] compartments;
//! * `tolerance.height` — the session text height is not finite/positive.

use super::*;
use crate::draw_tool::ToolParameter;
use cad_domain::{TextAlignH, TextAlignV};

/// The declared char-width estimate (the importer's `tolerance_frame` rule).
const TOLERANCE_CHAR_WIDTH_FACTOR: f64 = 0.62;

/// The declared row-height factor (the importer's `tolerance_frame` rule).
const TOLERANCE_ROW_HEIGHT_FACTOR: f64 = 1.6;

/// The horizontal padding on each side of a cell, as a fraction of the height.
const TOLERANCE_CELL_PADDING_FACTOR: f64 = 0.5;

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
fn frame_geometries(
    origin: Point3,
    cells: &[&str],
    height: f64,
    style: StyleId,
    font: Option<String>,
) -> CadResult<Vec<SemanticGeometry>> {
    let row_height = height * TOLERANCE_ROW_HEIGHT_FACTOR;
    let padding = height * TOLERANCE_CELL_PADDING_FACTOR;
    let widths: Vec<f64> = cells
        .iter()
        .map(|cell| {
            let chars = cell.chars().count().max(1) as f64;
            chars * height * TOLERANCE_CHAR_WIDTH_FACTOR + 2.0 * padding
        })
        .collect();
    let total_width: f64 = widths.iter().sum();
    if !total_width.is_finite() || total_width <= 0.0 || !row_height.is_finite() {
        return Err(invalid(
            "height",
            "the frame extent is not finite; check the session text height",
        ));
    }
    let z = origin.z;
    let corner = |x: f64, y: f64| Point3 { x, y, z };
    let mut geometries = Vec::with_capacity(1 + cells.len().saturating_sub(1) + cells.len());
    // The border: one closed rectangle.
    geometries.push(SemanticGeometry::Polyline {
        points: vec![
            corner(origin.x, origin.y),
            corner(origin.x + total_width, origin.y),
            corner(origin.x + total_width, origin.y + row_height),
            corner(origin.x, origin.y + row_height),
        ],
        bulges: vec![0.0; 4],
        closed: true,
        width: 0.0,
    });
    // One internal divider per compartment boundary.
    let mut x = origin.x;
    for width in widths.iter().take(cells.len().saturating_sub(1)) {
        x += *width;
        geometries.push(SemanticGeometry::Line {
            start: corner(x, origin.y),
            end: corner(x, origin.y + row_height),
        });
    }
    // One TEXT per compartment: left aligned, vertically centred in the cell.
    let mut cell_left = origin.x;
    for (cell, width) in cells.iter().zip(&widths) {
        geometries.push(SemanticGeometry::Text {
            text: (*cell).to_string(),
            position: corner(cell_left + padding, origin.y + row_height * 0.5),
            style,
            height,
            rotation: 0.0,
            font: font.clone(),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Middle,
            // The cell is already sized from the declared width estimate; an
            // explicit wrap boundary would only risk wrapping a cell that fits.
            wrap_width: None,
            width_factor: 1.0,
        });
        cell_left += *width;
    }
    Ok(geometries)
}
