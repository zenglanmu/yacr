//! The document's **application domain**: which reference application's
//! interface the document is edited in (ADR-0053).
//!
//! The value is document metadata, not drawing content: switching it changes
//! the chrome's ribbon/panel/theme profile, never a geometry row. It is written
//! to `document.json` as `application_domain` only when the document carries the
//! key, so a minor-13 writer stamps the value verbatim and an older reader is
//! refused by the minor gate instead of guessing.
//!
//! The tokens are neutral and stable file-format identifiers; the commercial
//! application each profile mirrors appears only in UI copy. A token that this
//! build does not know is refused by the header reader (never normalized to
//! `cad2d`), so a future domain requires a schema step rather than a silent
//! reinterpretation.

use std::fmt;

/// The four domains this build models.
///
/// `Cad2d` is the only fully implemented profile; the other three are explicit
/// interface placeholders (ADR-0053 D8), which is why no token is "unknown" at
/// this level: absence of a value is a separate state (`Option`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApplicationDomain {
    /// Two-dimensional drafting (AutoCAD-style ribbon, command line).
    Cad2d,
    /// Building information modelling (Revit-style ribbon, properties/browser).
    Bim,
    /// Concept modelling (SketchUp-style tool set and tray).
    Concept,
    /// Mechanical design (SolidWorks-style CommandManager and feature tree).
    Mechanical,
}

impl ApplicationDomain {
    /// Every token, in the order the UI switcher lists them.
    pub const ALL: [Self; 4] = [Self::Cad2d, Self::Bim, Self::Concept, Self::Mechanical];

    /// The stable file/UI token (`document.json` and the `DOMAIN` command).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cad2d => "cad2d",
            Self::Bim => "bim",
            Self::Concept => "concept",
            Self::Mechanical => "mechanical",
        }
    }

    /// Parse a token; `None` for anything this build does not model.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "cad2d" => Some(Self::Cad2d),
            "bim" => Some(Self::Bim),
            "concept" => Some(Self::Concept),
            "mechanical" => Some(Self::Mechanical),
            _ => None,
        }
    }

    /// The domain an import connector's output opens in, or `None` when the
    /// connector says nothing about an application domain.
    ///
    /// Only connectors whose documents are edited in one of the modelled
    /// interfaces are mapped; mesh formats (STL/OBJ/PLY/OFF/glTF) and anything
    /// unknown stay `None`, so the caller keeps the conservative default instead
    /// of claiming a mechanical model from a triangle soup.
    pub fn from_connector(connector: &str) -> Option<Self> {
        match connector {
            "dwg" | "dxf" | "dgn" => Some(Self::Cad2d),
            "ifc" | "rvt" => Some(Self::Bim),
            "skp" => Some(Self::Concept),
            "step" | "stp" => Some(Self::Mechanical),
            _ => None,
        }
    }
}

impl fmt::Display for ApplicationDomain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_round_trips_and_nothing_else_parses() {
        for domain in ApplicationDomain::ALL {
            assert_eq!(ApplicationDomain::from_token(domain.as_str()), Some(domain));
        }
        for token in ["", "CAD2D", "autocad", "revit", "sketchup", "solidworks"] {
            assert_eq!(ApplicationDomain::from_token(token), None, "{token}");
        }
    }

    #[test]
    fn connector_mapping_is_conservative() {
        assert_eq!(
            ApplicationDomain::from_connector("dwg"),
            Some(ApplicationDomain::Cad2d)
        );
        assert_eq!(
            ApplicationDomain::from_connector("rvt"),
            Some(ApplicationDomain::Bim)
        );
        assert_eq!(
            ApplicationDomain::from_connector("skp"),
            Some(ApplicationDomain::Concept)
        );
        assert_eq!(
            ApplicationDomain::from_connector("step"),
            Some(ApplicationDomain::Mechanical)
        );
        for mesh in ["stl", "obj", "ply", "off", "glb", "gltf", "unknown"] {
            assert_eq!(ApplicationDomain::from_connector(mesh), None, "{mesh}");
        }
    }
}
