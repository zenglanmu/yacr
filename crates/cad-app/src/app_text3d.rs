//! `TEXT3D`: one extruded glyph mesh from the host's outline font engine.
//!
//! The command is the **solid** counterpart of `TXTEXP` (which turns text into
//! outline polylines): it shapes the string with the same installed engine
//! ([`cad_representation::FontEngine`]), takes the filled glyph faces the display
//! already uses, and extrudes them into one `AcDbSubDMesh`
//! ([`crate::app_drawing::text3d`]).
//!
//! Contract: one transaction / one undo step through the shared create funnel;
//! Viewer is refused (this writes entity rows). Named refusals, nothing written:
//!
//! * `text3d.payload` — the wrong payload;
//! * `text3d.invalid_point` — a non-finite origin;
//! * `text3d.invalid_size` — height/extrusion non-finite or not strictly positive;
//! * `text3d.invalid_text` — empty, multi-line or over [`crate::MAX_TOOL_TEXT_CHARS`];
//! * `text3d.font_unavailable` — no engine installed, or the string's face cannot
//!   be shaped (no bundled fallback is invented);
//! * `text3d.no_outlines` / `text3d.not_solid` / `text3d.budget` — from the
//!   geometry builder.
//!
//! There is no font selector, rotation or alignment input in v1: the string is
//! placed at the given origin on the XY plane with the face the engine resolves
//! from its default/fallback chain, and the command says so instead of pretending
//! to support a dialog it does not have.

use super::*;
use crate::app_drawing::text3d::build_text3d;
use cad_domain::{TextAlignH, TextAlignV, TextLineSpacing};
use cad_representation::FontEngine;

/// The installed outline engine, if the carrier supplied one.
fn outline_fonts() -> Option<&'static std::sync::Arc<FontEngine>> {
    crate::app_extended_w9_reports::text_explode::outline_fonts()
}

fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}

impl Application {
    /// `TEXT3D <x> <y> <z> <height> <extrusion> <text>`.
    pub(crate) fn text3d(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Text3d(input) = &command.payload else {
            return Err(refusal(
                "text3d.payload",
                "TEXT3D needs <x> <y> <z> <height> <extrusion> <text>",
            ));
        };
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let origin = input.origin;
        if !(origin.x.is_finite() && origin.y.is_finite() && origin.z.is_finite()) {
            return Err(refusal(
                "text3d.invalid_point",
                "TEXT3D needs a finite origin point",
            ));
        }
        if !(input.height.is_finite() && input.height > 0.0) {
            return Err(refusal(
                "text3d.invalid_size",
                format!(
                    "the glyph height must be finite and strictly positive, got {}",
                    input.height
                ),
            ));
        }
        if !(input.extrusion.is_finite() && input.extrusion > 0.0) {
            return Err(refusal(
                "text3d.invalid_size",
                format!(
                    "the extrusion must be finite and strictly positive, got {}",
                    input.extrusion
                ),
            ));
        }
        let text = input.text.trim();
        if text.is_empty() {
            return Err(refusal(
                "text3d.invalid_text",
                "TEXT3D needs a non-empty string",
            ));
        }
        if text.contains(['\n', '\r']) {
            return Err(refusal(
                "text3d.invalid_text",
                "TEXT3D is a single line; newlines are not part of the string",
            ));
        }
        if text.chars().count() > crate::MAX_TOOL_TEXT_CHARS {
            return Err(refusal(
                "text3d.invalid_text",
                format!(
                    "the string holds {} characters, over the {} limit",
                    text.chars().count(),
                    crate::MAX_TOOL_TEXT_CHARS
                ),
            ));
        }
        let fonts = outline_fonts().ok_or_else(|| {
            unsupported(
                "text3d.font_unavailable",
                "no outline font engine is installed; the carrier supplies the same engine the \
                 display uses and this build bundles no face",
            )
        })?;
        let shaped = fonts
            .shape(
                input.font.as_deref().unwrap_or(""),
                text,
                // The extruder places the caps at the origin's own z; the shaping
                // z is irrelevant to the XY contours.
                cad_domain::Point3 {
                    x: origin.x,
                    y: origin.y,
                    z: 0.0,
                },
                input.height,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
                None,
                1.0,
                TextLineSpacing::default(),
            )
            .map_err(|error| {
                unsupported(
                    "text3d.font_unavailable",
                    format!("the string's face cannot be shaped: {error}"),
                )
            })?;
        if shaped.meshes.is_empty() && shaped.outlines.is_empty() {
            return Err(unsupported(
                "text3d.no_outlines",
                "the font engine produced no contour for this string",
            ));
        }
        let built = build_text3d(&shaped.meshes, origin.z, input.extrusion)?;
        let mut outcome = self.commit_new_entity_typed(
            session,
            command,
            SemanticGeometry::Mesh(built.mesh.clone()),
            crate::app_drawing::MESH_TYPE_KEY,
            "create 3D text",
        )?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "text3d.created".into(),
            message: format!(
                "TEXT3D: `{text}` as {} filled glyph(s), {} vertices, {} triangles, extruded by {}",
                built.glyphs,
                built.mesh.vertices.len(),
                built.mesh.triangles.len(),
                input.extrusion
            ),
        });
        Ok(outcome)
    }
}
