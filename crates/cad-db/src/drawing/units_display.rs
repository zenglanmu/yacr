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
//! The settings are stored, reported and consumed by the readouts: the
//! status-bar coordinate, the command-window echo/log and the measurement
//! diagnostics all format through [`UnitsDisplay::format_length`] /
//! [`UnitsDisplay::format_angle`] (the React readouts receive the same values on
//! `cad_status.units` and use one mirrored helper). AutoCAD's `$INSUNITS`
//! insertion scaling (`INSERT` across drawings) and the host Units dialog are
//! not modelled here and are named by the command's report instead of being
//! silently implied.

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
    /// Format one length in the document's stored format and precision.
    ///
    /// `precision` is the document's linear decimal-place count
    /// (`UnitContext::decimal_places`); the Units dialog stores it with the
    /// declared unit rather than here. Engineering and architectural formats
    /// read the value as **inches** (AutoCAD's convention for those formats); a
    /// non-finite value is printed as itself rather than fabricated as zero.
    ///
    /// The precision is capped at [`UNITS_MAX_PRECISION`] for allocation safety;
    /// the write path already refuses anything larger, so no stored value is
    /// silently narrowed.
    pub fn format_length(&self, value: f64, precision: u8) -> String {
        if !value.is_finite() {
            return format!("{value}");
        }
        let value = if value == 0.0 { 0.0 } else { value };
        let precision = usize::from(precision.min(UNITS_MAX_PRECISION));
        match self.length_format {
            LengthFormat::Scientific => scientific(value, precision),
            LengthFormat::Decimal => format!("{value:.precision$}"),
            LengthFormat::Engineering => engineering(value, precision),
            LengthFormat::Architectural => architectural(value, precision),
            LengthFormat::Fractional => fractional(value, precision),
        }
    }

    /// Format one angle in degrees.
    ///
    /// `absolute` applies `ANGBASE`/`ANGDIR` first (an absolute bearing) and
    /// normalises it into `[0, 360)`. A **relative** angle (a measured angle
    /// between two vectors) keeps its sign and magnitude: the base direction and
    /// rotation sense describe where zero points, not how large a turn is.
    pub fn format_angle(&self, degrees: f64, absolute: bool) -> String {
        if !degrees.is_finite() {
            return format!("{degrees}");
        }
        let precision = usize::from(self.angle_precision.min(UNITS_MAX_PRECISION));
        let value = if absolute {
            let mut value = degrees - self.angle_base_degrees;
            if self.angle_clockwise {
                value = -value;
            }
            value.rem_euclid(360.0)
        } else {
            degrees
        };
        match self.angle_format {
            AngleFormat::DecimalDegrees => format!("{value:.precision$}°"),
            AngleFormat::DegreesMinutesSeconds => dms(value, precision),
            AngleFormat::Gradians => format!("{:.precision$}g", value * 10.0 / 9.0),
            AngleFormat::Radians => format!("{:.precision$}r", value.to_radians()),
            AngleFormat::SurveyorsUnits => surveyors(value, precision),
        }
    }

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

/// One scientific-notation length (`1.2345E+01`).
fn scientific(value: f64, precision: usize) -> String {
    if value == 0.0 {
        return format!("0.{}E+00", "0".repeat(precision));
    }
    let text = format!("{value:.*e}", precision);
    let (mantissa, exponent) = text.split_once('e').unwrap_or((text.as_str(), "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}E{sign}{:02}", exponent.unsigned_abs())
}

/// Feet-and-decimal-inches (`1'-0.3450"`), reading the value as inches.
fn engineering(value: f64, precision: usize) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let scale = 10u128.pow(precision as u32);
    let total = (value.abs() * scale as f64).round() as u128;
    let whole_inches = total / scale;
    let fraction = total % scale;
    let feet = whole_inches / 12;
    let inches = whole_inches % 12;
    if precision == 0 {
        format!("{sign}{feet}'-{inches}\"")
    } else {
        format!(
            "{sign}{feet}'-{inches}.{fraction:0precision$}\"",
            precision = precision
        )
    }
}

/// Feet-and-fractional-inches (`1'-0 3/8"`), reading the value as inches.
fn architectural(value: f64, precision: usize) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let denominator = 1u128 << precision;
    let per_foot = 12 * denominator;
    let total = (value.abs() * denominator as f64).round() as u128;
    let feet = total / per_foot;
    let remainder = total % per_foot;
    let inches = remainder / denominator;
    let numerator = remainder % denominator;
    if numerator == 0 {
        return format!("{sign}{feet}'-{inches}\"");
    }
    let divisor = gcd(numerator, denominator);
    format!(
        "{sign}{feet}'-{inches} {}/{}\"",
        numerator / divisor,
        denominator / divisor
    )
}

/// A fractional length without feet (`12 3/8`).
fn fractional(value: f64, precision: usize) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let denominator = 1u128 << precision;
    let total = (value.abs() * denominator as f64).round() as u128;
    let whole = total / denominator;
    let numerator = total % denominator;
    if numerator == 0 {
        return format!("{sign}{whole}");
    }
    let divisor = gcd(numerator, denominator);
    format!(
        "{sign}{whole} {}/{}",
        numerator / divisor,
        denominator / divisor
    )
}

/// An angle as degrees/minutes/seconds (`45d0'0"`).
fn dms(value: f64, precision: usize) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let (degrees, minutes, seconds) = dms_parts(value.abs(), precision);
    format!("{sign}{degrees}d{minutes}'{seconds}\"")
}

/// The surveyor's bearing form (`N 45d0'0" E`).
fn surveyors(value: f64, precision: usize) -> String {
    let angle = value.rem_euclid(360.0);
    let (prefix, inner, suffix) = if angle <= 90.0 {
        ("N", angle, "E")
    } else if angle <= 180.0 {
        ("S", 180.0 - angle, "E")
    } else if angle <= 270.0 {
        ("S", angle - 180.0, "W")
    } else {
        ("N", 360.0 - angle, "W")
    };
    let (degrees, minutes, seconds) = dms_parts(inner, precision);
    format!("{prefix} {degrees}d{minutes}'{seconds}\" {suffix}")
}

/// Split an absolute angle into rounded degree/minute/second components.
fn dms_parts(value: f64, precision: usize) -> (u128, u128, String) {
    let scale = 10u128.pow(precision as u32);
    let total = (value * 3600.0 * scale as f64).round() as u128;
    let seconds_per_minute = 60 * scale;
    let degree = total / (60 * seconds_per_minute);
    let remainder = total % (60 * seconds_per_minute);
    let minutes = remainder / seconds_per_minute;
    let seconds_units = remainder % seconds_per_minute;
    let seconds_whole = seconds_units / scale;
    let seconds = if precision == 0 {
        format!("{seconds_whole}")
    } else {
        format!(
            "{seconds_whole}.{fraction:0precision$}",
            fraction = seconds_units % scale,
            precision = precision
        )
    };
    (degree, minutes, seconds)
}

/// Greatest common divisor for reducing a fraction.
fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.max(1)
}

#[cfg(test)]
mod format_tests {
    use super::*;

    fn display(length: LengthFormat, angle: AngleFormat, precision: u8) -> UnitsDisplay {
        UnitsDisplay {
            length_format: length,
            angle_format: angle,
            angle_precision: precision,
            angle_base_degrees: 0.0,
            angle_clockwise: false,
        }
    }

    #[test]
    fn every_length_format_uses_the_stored_form() {
        let decimal = display(LengthFormat::Decimal, AngleFormat::DecimalDegrees, 4);
        assert_eq!(decimal.format_length(12.345, 2), "12.35");
        let scientific = display(LengthFormat::Scientific, AngleFormat::DecimalDegrees, 4);
        assert_eq!(scientific.format_length(12.345, 4), "1.2345E+01");
        let engineering = display(LengthFormat::Engineering, AngleFormat::DecimalDegrees, 4);
        assert_eq!(engineering.format_length(12.345, 4), "1'-0.3450\"");
        let architectural = display(LengthFormat::Architectural, AngleFormat::DecimalDegrees, 4);
        assert_eq!(architectural.format_length(12.375, 3), "1'-0 3/8\"");
        let fractional = display(LengthFormat::Fractional, AngleFormat::DecimalDegrees, 4);
        assert_eq!(fractional.format_length(12.375, 3), "12 3/8");
    }

    #[test]
    fn length_precision_and_negative_zero_are_exact() {
        let display = display(LengthFormat::Decimal, AngleFormat::DecimalDegrees, 4);
        assert_eq!(display.format_length(-0.0, 4), "0.0000");
        assert_eq!(display.format_length(-1.5, 2), "-1.50");
    }

    #[test]
    fn every_angle_format_uses_the_stored_form_and_precision() {
        let decimal = display(LengthFormat::Decimal, AngleFormat::DecimalDegrees, 2);
        assert_eq!(decimal.format_angle(45.0, false), "45.00°");
        let dms = display(LengthFormat::Decimal, AngleFormat::DegreesMinutesSeconds, 0);
        assert_eq!(dms.format_angle(45.5, false), "45d30'0\"");
        let gradians = display(LengthFormat::Decimal, AngleFormat::Gradians, 2);
        assert_eq!(gradians.format_angle(45.0, false), "50.00g");
        let radians = display(LengthFormat::Decimal, AngleFormat::Radians, 4);
        assert_eq!(radians.format_angle(180.0, false), "3.1416r");
        let surveyors = display(LengthFormat::Decimal, AngleFormat::SurveyorsUnits, 0);
        assert_eq!(surveyors.format_angle(45.0, true), "N 45d0'0\" E");
        assert_eq!(surveyors.format_angle(135.0, true), "S 45d0'0\" E");
    }

    #[test]
    fn absolute_angles_apply_the_base_angle_and_clockwise_direction() {
        let mut display = display(LengthFormat::Decimal, AngleFormat::DecimalDegrees, 2);
        display.angle_base_degrees = 90.0;
        assert_eq!(display.format_angle(90.0, true), "0.00°");
        display.angle_clockwise = true;
        assert_eq!(display.format_angle(0.0, true), "90.00°");
        // A relative angle is not a bearing: base/direction do not apply.
        assert_eq!(display.format_angle(0.0, false), "0.00°");
    }
}
