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

/// One `UNITS`/`-UNITS` document-units write.
///
/// The command carries a **patch**: `None` leaves that setting alone. The
/// representable domain is the document's declared unit + linear decimal
/// precision (`cad_domain::UnitContext`) plus the document-level settings the
/// Units dialog also edits — the length *format* and the angular
/// format/precision/base-angle/clockwise direction ([`cad_db::UnitsDisplay`]).
/// No field is defaulted by this layer: an absent field is unchanged and an
/// empty patch is refused.
///
/// `$INSUNITS` insertion scaling (scaling an inserted drawing by the source and
/// target units) is not modelled; the command reports that boundary by name.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitsCommand {
    /// The declared unit and linear decimal precision, when the line writes
    /// them. The two are chosen together: one typed unit keeps the stored
    /// numbers unchanged (`display_per_source = 1.0`).
    pub declared: Option<DeclaredUnits>,
    /// The linear length format (`Scientific`/`Decimal`/…).
    pub length_format: Option<cad_db::LengthFormat>,
    /// The angular format (`DecimalDegrees`/`DegreesMinutesSeconds`/…).
    pub angle_format: Option<cad_db::AngleFormat>,
    /// Angular decimal places, `0..=8`.
    pub angle_precision: Option<u8>,
    /// `ANGBASE` in degrees (0 East, 90 North, 180 West, 270 South, anything
    /// else is the dialog's `Other`).
    pub angle_base_degrees: Option<f64>,
    /// `ANGDIR`: `true` clockwise, `false` counter-clockwise.
    pub angle_clockwise: Option<bool>,
}

/// One declared-unit write inside [`UnitsCommand`].
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredUnits {
    /// The unit the drawing's coordinates are declared in, and the unit they
    /// are displayed in.
    pub unit: cad_domain::Unit,
    /// Linear decimal places, `0..=8`.
    pub decimal_places: u8,
}

impl UnitsCommand {
    /// The declared-unit context this patch would write, or `None` when it does
    /// not touch the declared unit.
    pub fn declared_context(&self) -> Option<cad_domain::UnitContext> {
        self.declared
            .as_ref()
            .map(|declared| cad_domain::UnitContext {
                source: declared.unit.clone(),
                display: declared.unit.clone(),
                display_per_source: match declared.unit {
                    cad_domain::Unit::DrawingUnits => None,
                    _ => Some(1.0),
                },
                decimal_places: declared.decimal_places,
            })
    }

    /// Whether the patch names no setting at all.
    pub fn is_empty(&self) -> bool {
        self.declared.is_none()
            && self.length_format.is_none()
            && self.angle_format.is_none()
            && self.angle_precision.is_none()
            && self.angle_base_degrees.is_none()
            && self.angle_clockwise.is_none()
    }
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
