//! Drawing-level `UNITS` display settings.
//!
//! The declared unit and the linear decimal precision live in
//! `cad_domain::UnitContext` (host state, written by `UNITS <unit> <precision>`).
//! The Units dialog also edits settings that are *not* part of the unit context:
//! the linear length **format** (Scientific/Decimal/Engineering/Architectural/
//! Fractional) and the angular format/precision/base-angle/clockwise direction.
//! Those are document-level scalars, so they belong in the drawing header next to
//! `$LTSCALE`/`$PDMODE`, not in a table and not in `cad-domain`.
//!
//! The block is written to `document.json` → `variables.units_display` **only
//! when it differs from [`UnitsDisplay::default`]**, exactly like the
//! `DWGPROPS` metadata block: a document that never edits these settings stays
//! byte-identical to a pre-settings archive, and an absent key decodes to the
//! documented defaults.
//!
//! # Remaining boundary
//!
//! The settings are stored and reported. The coordinate/measurement readouts
//! still format with fixed decimals (`cad-app` measurement records carry the
//! declared unit and `UnitContext::decimal_places`; the React status/log use
//! their own fixed counts), so the stored formats do not yet change every
//! on-screen number. That display wiring and AutoCAD's `$INSUNITS` insertion
//! scaling (`INSERT` across drawings) are not modelled here and are named by the
//! command's report instead of being silently implied.

use cad_domain::*;

use super::DrawingDatabase;
use crate::change::ChangeSet;
use crate::scope::ChangeScope;

/// The largest accepted precision for lengths and angles, in decimal places.
///
/// Eight is the same ceiling `UnitContext` and the Units dialog use: enough for
/// a millimetre length to distinguish a nanometre, and never silently clamped.
pub const UNITS_MAX_PRECISION: u8 = 8;

/// AutoCAD's linear length *format* (the Units dialog's "Type" list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LengthFormat {
    /// `1.2345E+01`
    Scientific,
    /// `12.3450`
    #[default]
    Decimal,
    /// `1'-0.3450"`
    Engineering,
    /// `1'-0 3/8"`
    Architectural,
    /// `12 3/8`
    Fractional,
}

impl LengthFormat {
    /// The closed set, in the Units dialog's order.
    pub const ALL: [LengthFormat; 5] = [
        LengthFormat::Scientific,
        LengthFormat::Decimal,
        LengthFormat::Engineering,
        LengthFormat::Architectural,
        LengthFormat::Fractional,
    ];

    /// The persisted token (part of the format).
    pub fn as_token(self) -> &'static str {
        match self {
            LengthFormat::Scientific => "scientific",
            LengthFormat::Decimal => "decimal",
            LengthFormat::Engineering => "engineering",
            LengthFormat::Architectural => "architectural",
            LengthFormat::Fractional => "fractional",
        }
    }

    /// Parse a persisted or typed token. Unknown tokens are refused by the
    /// caller, never defaulted.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token.trim().to_ascii_lowercase().as_str() {
            "scientific" => LengthFormat::Scientific,
            "decimal" => LengthFormat::Decimal,
            "engineering" => LengthFormat::Engineering,
            "architectural" => LengthFormat::Architectural,
            "fractional" => LengthFormat::Fractional,
            _ => return None,
        })
    }
}

/// AutoCAD's angular format (the Units dialog's angle "Type" list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AngleFormat {
    /// `45.0000`
    #[default]
    DecimalDegrees,
    /// `45d0'0"`
    DegreesMinutesSeconds,
    /// `50.0000g`
    Gradians,
    /// `0.7854r`
    Radians,
    /// `N 45d0'0" E`
    SurveyorsUnits,
}

impl AngleFormat {
    /// The closed set, in the Units dialog's order.
    pub const ALL: [AngleFormat; 5] = [
        AngleFormat::DecimalDegrees,
        AngleFormat::DegreesMinutesSeconds,
        AngleFormat::Gradians,
        AngleFormat::Radians,
        AngleFormat::SurveyorsUnits,
    ];

    /// The persisted token (part of the format).
    pub fn as_token(self) -> &'static str {
        match self {
            AngleFormat::DecimalDegrees => "decimal_degrees",
            AngleFormat::DegreesMinutesSeconds => "degrees_minutes_seconds",
            AngleFormat::Gradians => "gradians",
            AngleFormat::Radians => "radians",
            AngleFormat::SurveyorsUnits => "surveyors_units",
        }
    }

    /// Parse a persisted or typed token. Unknown tokens are refused by the
    /// caller, never defaulted.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token.trim().to_ascii_lowercase().as_str() {
            "decimal_degrees" | "decimal" | "degrees" => AngleFormat::DecimalDegrees,
            "degrees_minutes_seconds" | "dms" => AngleFormat::DegreesMinutesSeconds,
            "gradians" | "grad" => AngleFormat::Gradians,
            "radians" | "rad" => AngleFormat::Radians,
            "surveyors_units" | "surveyor" => AngleFormat::SurveyorsUnits,
            _ => return None,
        })
    }
}

/// The document-level `UNITS` settings that are not part of `UnitContext`.
///
/// The defaults are this build's declared initial values, not values read from a
/// file: decimal length, decimal-degree angles with zero decimals, a zero base
/// angle and counter-clockwise positive angles.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UnitsDisplay {
    pub length_format: LengthFormat,
    pub angle_format: AngleFormat,
    /// Angular decimal places, `0..=UNITS_MAX_PRECISION`.
    pub angle_precision: u8,
    /// `ANGBASE` in degrees: the direction of the zero angle. `0` is East,
    /// `90` North, `180` West, `270` South; any finite value is accepted
    /// (`Other` in the Units dialog).
    pub angle_base_degrees: f64,
    /// `ANGDIR`: `true` is clockwise, `false` counter-clockwise.
    pub angle_clockwise: bool,
}

impl UnitsDisplay {
    /// Refuse an out-of-range precision or a non-finite base angle.
    ///
    /// A refusal happens before the database is touched; nothing is clamped.
    pub fn validate(&self) -> CadResult<()> {
        if self.angle_precision > UNITS_MAX_PRECISION {
            return Err(CadError::InvalidInput(format!(
                "units.invalid_angle_precision: angular precision must be 0 to \
                 {UNITS_MAX_PRECISION} decimal places, got {}; nothing was written",
                self.angle_precision
            )));
        }
        if !self.angle_base_degrees.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "units.invalid_angle_base: the base angle must be a finite number of degrees, got \
                 {}; nothing was written",
                self.angle_base_degrees
            )));
        }
        Ok(())
    }

    /// The direction preset a base angle names, when it is exactly one of the
    /// four dialog presets.
    pub fn base_direction(&self) -> Option<&'static str> {
        if self.angle_base_degrees == 0.0 {
            Some("East")
        } else if self.angle_base_degrees == 90.0 {
            Some("North")
        } else if self.angle_base_degrees == 180.0 {
            Some("West")
        } else if self.angle_base_degrees == 270.0 {
            Some("South")
        } else {
            None
        }
    }

    /// The clockwise/counter-clockwise token.
    pub fn rotation_token(&self) -> &'static str {
        if self.angle_clockwise {
            "clockwise"
        } else {
            "counter-clockwise"
        }
    }
}

impl DrawingDatabase {
    /// The document's `UNITS` display settings.
    pub fn units_display(&self) -> &UnitsDisplay {
        &self.units_display
    }

    /// Replace the `UNITS` display settings in exactly one revision.
    ///
    /// Phase 1 validates the whole value before anything is touched. Phase 2
    /// treats an unchanged value as an authoritative no-op (`before == after`,
    /// no revision bump and no dirty header). Phase 3 writes it and raises the
    /// revision once with a header-only [`ChangeSet`]. The write records no undo
    /// step: `cad-history` has no header patch kind.
    pub fn set_units_display(
        &mut self,
        value: UnitsDisplay,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        value.validate()?;
        let before = self.revision;
        if self.units_display == value {
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
                scope: ChangeScope::default(),
            });
        }
        self.units_display = value;
        self.invalidate_bounds_cache();
        let after = Revision(before.0 + 1);
        self.revision = after;
        let mut scope = ChangeScope::default();
        scope.header();
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
            scope,
        })
    }
}
