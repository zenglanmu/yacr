//! The dimension-editing batch's typed payloads (`DIMSTYLE`, `DIMTEDIT`,
//! `DIMEDIT`, `DIMOVERRIDE`, `DIMJOGLINE`, `QDIM`).
//!
//! Split out of `payload_extra.rs` to keep that file inside the code-size
//! limit; every contract lives in [`crate::command_id::dimension_edit`].

use super::*;

/// The 2026-11-07 dimension-editing batch. Grouped into one
/// [`crate::CommandPayload::Dimension`] variant so the payload enum stays inside
/// the file-size budget; every contract lives in
/// [`crate::command_id::dimension_edit`].
#[derive(Debug, Clone, PartialEq)]
pub enum DimensionBatchPayload {
    /// `DIMSTYLE LIST|CURRENT|SET <name>`.
    Style(DimStyleCommand),
    /// `DIMTEDIT MOVE|LEFT|RIGHT|CENTER|HOME|ANGLE|OBLIQUE`.
    TextEdit(DimTextEditCommand),
    /// `DIMEDIT TEXT <override>|RESET|HOME|ROTATE|OBLIQUE`.
    Edit(DimEditCommand),
    /// `DIMOVERRIDE <variable> <value>`.
    Override(DimOverrideCommand),
    /// `DIMJOGLINE ADD <x> <y>|REMOVE`.
    JogLine(DimJogLineCommand),
    /// `QDIM <mode> [DATUM x y] [EDIT i…] [SETTINGS ENDPOINTS|INTERSECTIONS]`.
    QDim(QDimCommand),
}

/// The typed option of a `DIMSTYLE` line.
///
/// `LIST`/`CURRENT` are the read-only reports. The remaining variants are the
/// post-import table writes (`cad-db`'s `drawing::style_edit`): `SET` makes the
/// row the session's current dimension style, and `NEW`/`RENAME`/`DELETE`/
/// `MODIFY` change the table in one transaction / one undo step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DimStyleCommand {
    /// Report every stored dimension style (read-only).
    List,
    /// Report the style dimension creation resolves (read-only).
    Current,
    /// Make the named row the session's current dimension style.
    Set(String),
    /// Create a row; `from` copies an existing row's variables.
    New { name: String, from: Option<String> },
    /// Rename a row (refused while a dimension still names it).
    Rename { old: String, new: String },
    /// Delete a row (`Standard` and in-use rows are refused).
    Delete(String),
    /// Write one or more stored `<field> <value>` pairs of a row.
    Modify {
        name: String,
        assignments: Vec<(String, String)>,
    },
}

/// The typed option of a `DIMTEDIT` line.
///
/// The documented AutoCAD 2026 set is `Left`/`Right`/`Center`/`Home`/`Angle`
/// plus the free text location; the former `ROTATE` spelling is the `Angle`
/// option (`DIMEDIT`'s Rotate is a different command). There is no `RESET`:
/// `Home` restores the default text position and `Angle 0` the default
/// orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DimTextEditCommand {
    /// Move the measurement text to this drawing-plane X/Y (Z stays on the
    /// dimension's own plane).
    Move { x: f64, y: f64 },
    /// Justify the text to the start of its dimension line: the first
    /// extension-line foot (linear/aligned), the centre-side leader end
    /// (radius/diameter), the dimension arc's start angle (angular) or the
    /// feature end of the leader (ordinate).
    Left,
    /// Justify the text to the end of its dimension line (the `Left` anchor's
    /// opposite end).
    Right,
    /// Justify the text to the midpoint of its dimension line.
    Center,
    /// Restore the derived text position (the group 11 sentinel); the rotation
    /// is untouched.
    Home,
    /// Set the text rotation to these degrees about the UCS X axis, keeping the
    /// text centre where it is; `0` restores the derived orientation.
    Angle { degrees: f64 },
    /// Oblique (slant) the glyphs by these degrees.
    ///
    /// Parsed so the refusal can name the real gap: this build's text model
    /// ([`cad_domain::SemanticGeometry::Text`]) stores no oblique angle for
    /// **any** text, so the command refuses by name and writes nothing.
    Oblique { degrees: f64 },
}

/// The typed option of a `DIMEDIT` line.
#[derive(Debug, Clone, PartialEq)]
pub enum DimEditCommand {
    /// Store a dimension text override (group 1); `<>` is the measurement
    /// placeholder.
    Text(String),
    /// Drop the text override and render the measurement again.
    Reset,
    /// Restore the derived text position (the group 11 sentinel); the rotation
    /// is untouched (use `Rotate 0` for the derived orientation).
    Home,
    /// Set the text rotation to these degrees about the UCS X axis, keeping the
    /// text centre where it is; `0` restores the derived orientation.
    Rotate { degrees: f64 },
    /// Oblique the **extension lines** by these degrees (the DIMSTYLE group-52
    /// angle). Parsed so the refusal can name the real gap: the domain's
    /// dimension entity stores no oblique angle, so the command refuses by name
    /// and writes nothing.
    Oblique { degrees: f64 },
}

/// The input of a `DIMOVERRIDE <variable> <value>` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DimOverrideCommand {
    pub variable: String,
    /// The trimmed remainder of the line, verbatim (no quote processing).
    pub value: String,
}

/// The typed option of a `DIMJOGLINE` line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DimJogLineCommand {
    Add { x: f64, y: f64 },
    Remove,
}
