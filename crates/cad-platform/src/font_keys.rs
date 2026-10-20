//! Font-key discovery: which fonts a drawing references, and whether it holds
//! text that needs shaping even when it names no font.
//!
//! Split out of [`crate::fonts`] so that module stays under the code-file size
//! gate; both functions are re-exported there unchanged, so callers keep using
//! `cad_platform::fonts::requested_fonts` / `needs_text_shaping`.

use cad_db::DrawingDatabase;
use cad_domain::SemanticGeometry;

/// Font keys a drawing references, read from the public database API.
///
/// This only collects logical keys from `TEXT`/`MTEXT` geometry and the text
/// styles those entities point at (`Style::resource_keys`, which the importer
/// populates with the style's SHX/big-font/TTF names). It performs no catalog
/// lookup, no network access and no path resolution: a hostile drawing can
/// therefore only supply names, which `plan_fonts` later sanitises to catalog
/// entries. Names that resolve to nothing remain explicit in the loading report.
pub fn requested_fonts(database: &DrawingDatabase) -> Vec<String> {
    let mut out = Vec::new();
    for entity in database.entities() {
        collect_fonts(database, &entity.geometry, &mut out);
    }
    out
}

/// Whether the drawing contains text that needs shaping, even when it names no
/// font.
///
/// [`requested_fonts`] only reports *named* font keys. A hand-built or newly
/// created drawing can hold TEXT/MTEXT (or a dimension's measurement text)
/// while its style has no STYLE entry at all, so `requested_fonts` is empty and
/// the host would install no engine — leaving every glyph unshaped and not
/// drawn. This signal says "an engine is needed" without inventing a font name;
/// the missing font then resolves through the registered fallback chain.
///
/// Whitespace-only text needs no glyphs and does not count.
pub fn needs_text_shaping(database: &DrawingDatabase) -> bool {
    database
        .entities()
        .any(|entity| geometry_has_text(&entity.geometry))
}

fn geometry_has_text(geometry: &SemanticGeometry) -> bool {
    match geometry {
        SemanticGeometry::Text { text, .. } => text.chars().any(|c| !c.is_whitespace()),
        SemanticGeometry::Compound(parts) => parts.iter().any(geometry_has_text),
        // A dimension's measurement text is a display child (same traversal as
        // `collect_fonts`).
        SemanticGeometry::Dimension(dimension) => dimension.display().iter().any(geometry_has_text),
        // A hatch is traversed like a compound (ADR 0018 E).
        SemanticGeometry::Hatch(hatch) => hatch.display().iter().any(geometry_has_text),
        // A multileader's annotation text is a display child (ADR 0019 G).
        SemanticGeometry::MultiLeader(multileader) => {
            multileader.display().iter().any(geometry_has_text)
        }
        _ => false,
    }
}

pub(crate) fn collect_fonts(
    database: &DrawingDatabase,
    geometry: &SemanticGeometry,
    out: &mut Vec<String>,
) {
    match geometry {
        SemanticGeometry::Text { font, style, .. } => {
            if let Some(font) = font {
                out.push(font.clone());
            }
            if let Some(style) = database.style(*style) {
                out.extend(style.resource_keys.iter().cloned());
            }
        }
        SemanticGeometry::Compound(parts) => {
            for part in parts {
                collect_fonts(database, part, out);
            }
        }
        // A dimension's measurement text is a display child; without this arm
        // its font is never registered and the text is silently not drawn
        // (ADR 0017 A.8).
        SemanticGeometry::Dimension(dimension) => {
            for part in dimension.display() {
                collect_fonts(database, part, out);
            }
        }
        // A hatch is traversed like a compound (ADR 0018 E).
        SemanticGeometry::Hatch(hatch) => {
            for part in hatch.display() {
                collect_fonts(database, part, out);
            }
        }
        // A multileader's annotation text is a display child; without this arm its
        // font is never registered and the text is silently not drawn
        // (ADR 0019 decision G).
        SemanticGeometry::MultiLeader(multileader) => {
            for part in multileader.display() {
                collect_fonts(database, part, out);
            }
        }
        _ => {}
    }
}
