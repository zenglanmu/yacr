//! The typed `TEXT3D` payload.

use cad_domain::Point3;

/// The input of a `TEXT3D <x> <y> <z> <height> <extrusion> <text>` line.
///
/// The command builds **extruded glyph geometry** from the host's outline font
/// engine (the same engine `TXTEXP` uses), as one `AcDbSubDMesh` row in one
/// transaction / one undo step. There is no font selector: the outline resolves
/// through the engine's default/fallback chain, exactly like an unstyled text.
#[derive(Debug, Clone, PartialEq)]
pub struct Text3dInput {
    /// Text baseline origin, in world coordinates.
    pub origin: Point3,
    /// Cap height in model units (strictly positive).
    pub height: f64,
    /// Extrusion along world `+Z`, in model units (strictly positive).
    pub extrusion: f64,
    /// The face to shape with, or `None` for the engine's default/fallback chain
    /// (the reference dialog's font choice, expressed as one token).
    pub font: Option<String>,
    /// The string to extrude (single line; trimmed; non-empty).
    pub text: String,
}
