//! Payloads for document-level administrative commands.
//!
//! Split out of `payload_extra.rs` for the file-size gate. These are input
//! values only: `DWGPROPS` writes summary metadata, `UNITS` writes the declared
//! unit/precision, and `SYSVDLG` reads/applies modelled system variables.

/// One `DWGPROPS` summary-metadata write.
///
/// The five named fields are AutoCAD's Drawing Properties ▸ Summary fields; a
/// custom property is a free-form name/value pair. An **empty value** clears
/// the named field or removes the custom property (the command layer maps it to
/// `None`), so there is no separate "remove" grammar for named fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DwgPropsCommand {
    /// Set (or clear, with an empty value) one named summary field.
    SetField {
        field: cad_db::DocumentMetadataField,
        value: String,
    },
    /// Set one custom property; an empty value removes it.
    SetCustom { name: String, value: String },
}

/// One `UNITS`/`-UNITS` drawing-unit write.
///
/// The command's representable domain is the document's declared source and
/// display unit plus the linear decimal precision. Angle format/precision/
/// direction and the AutoCAD length *format* table are not modeled, so the
/// command reports that gap instead of accepting values it cannot store.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitsCommand {
    /// The unit the drawing's coordinates are declared in, and the unit they
    /// are displayed in. The two are chosen together: one typed unit keeps the
    /// stored numbers unchanged (`display_per_source = 1.0`), which is exactly
    /// what "declare the drawing in mm" means.
    pub unit: cad_domain::Unit,
    /// Linear decimal places, `0..=8`.
    pub decimal_places: u8,
}

/// One `SYSVDLG` action that is not the bare report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SysVarDialogCommand {
    /// `SYSVDLG <name>`: report one modelled variable by name.
    Read(String),
    /// `SYSVDLG <name> <value>` (one entry) or `SYSVDLG RESTORE <file>` (many):
    /// apply value strings through the one existing setter per variable. Every
    /// name and value is validated before anything is written.
    Apply(Vec<SysVarAssignment>),
}

/// One name/value pair for [`SysVarDialogCommand::Apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysVarAssignment {
    pub name: String,
    pub value: String,
}
