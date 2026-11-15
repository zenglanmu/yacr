//! `DIMOVERRIDE`: the entity style-snapshot override.
//!
//! Split out of [`super::dimension_edit`] to keep that file inside the code-size
//! limit. The contract (the closed variable set and every refusal) lives in
//! [`crate::command_id::dimension_edit`] and the per-variable notes on
//! [`apply_dimension_override`].

use super::dimension_edit::{
    batch_payload, drawing_of, invalid, rebuild_dimension, resolve_selected_dimensions,
    unchanged_outcome, unsupported,
};
use super::*;
use cad_domain::DimensionStyleValues;

impl Application {
    /// `DIMOVERRIDE`: apply one closed variable to every selected dimension's
    /// style snapshot.
    pub(crate) fn dim_override(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = match batch_payload(command, "dimoverride")? {
            DimensionBatchPayload::Override(request) => request.clone(),
            _ => {
                return Err(invalid(
                    "dimoverride.payload",
                    "DIMOVERRIDE takes <variable> <value>",
                ))
            }
        };
        let variable = request.variable.trim().to_ascii_uppercase();
        let references = session.selection.refs();
        if references.is_empty() {
            return Err(invalid(
                "dimoverride.selection",
                "DIMOVERRIDE needs at least one selected dimension",
            ));
        }
        let drawing = drawing_of(self, command)?;
        let selected = resolve_selected_dimensions(&drawing, command, references, "dimoverride")?;
        let mut changes = Vec::new();
        for target in selected {
            let mut style = target.dimension.style().clone();
            apply_dimension_override(&drawing, &mut style, &variable, &request.value)?;
            let after = rebuild_dimension(
                &target.dimension,
                target.dimension.definition().clone(),
                Some(style),
                target.dimension.text_override().map(str::to_string),
                target.dimension.text_h_align(),
            )
            .map_err(|error| {
                invalid(
                    "dimoverride.degenerate",
                    format!("the dimension has no synthesizable display: {error}"),
                )
            })?;
            let mut entity = target.entity.clone();
            entity.geometry = SemanticGeometry::Dimension(Box::new(after));
            if entity.geometry != target.entity.geometry {
                changes.push((target.entity, entity));
            }
        }
        if changes.is_empty() {
            return Ok(unchanged_outcome(
                "dimoverride.unchanged",
                &format!(
                    "DIMOVERRIDE: every selected dimension already stores {variable} at this value \
                     (no transaction)"
                ),
            ));
        }
        let count = changes.len();
        self.commit_dimension_changes(
            command,
            changes,
            "dimoverride",
            "dimoverride.applied",
            &format!(
                "DIMOVERRIDE: applied {variable} to the entity style snapshot of {count} \
                 dimension(s); the DIMSTYLE table was not changed"
            ),
        )
    }
}

/// The closed `DIMOVERRIDE` variable set, applied to the entity style snapshot.
///
/// Every value is validated (never clamped); a variable the snapshot cannot
/// carry is a named capability refusal. See
/// [`crate::command_id::dimension_edit`] for the set and its boundaries.
fn apply_dimension_override(
    drawing: &DrawingDatabase,
    style: &mut DimensionStyleValues,
    variable: &str,
    value: &str,
) -> CadResult<()> {
    let value = value.trim();
    match variable {
        "DIMTXT" => style.text_height = positive(variable, value)?,
        "DIMASZ" => style.arrow_size = positive(variable, value)?,
        "DIMEXO" => style.ext_offset = non_negative(variable, value)?,
        "DIMEXE" => style.ext_beyond = non_negative(variable, value)?,
        "DIMGAP" => style.text_gap = non_negative(variable, value)?,
        "DIMCEN" => style.center_mark = finite(variable, value)?,
        "DIMDLI" => style.dim_line_increment = positive(variable, value)?,
        "DIMTAD" => {
            let code = integer(variable, value)?;
            if !(0..=4).contains(&code) {
                return Err(invalid(
                    "dimoverride.bad_value",
                    format!("DIMTAD must be 0..=4, got {code}"),
                ));
            }
            style.text_vertical = code;
        }
        "DIMTVP" => style.text_vertical_position = finite(variable, value)?,
        "DIMLFAC" => style.linear_factor = positive(variable, value)?,
        "DIMDEC" => {
            let decimals = integer(variable, value)?;
            if !(0..=8).contains(&decimals) {
                return Err(invalid(
                    "dimoverride.bad_value",
                    format!("DIMDEC must be 0..=8, got {decimals}"),
                ));
            }
            style.decimals = decimals as usize;
        }
        "DIMRND" => style.rounding = non_negative(variable, value)?,
        "DIMPOST" => {
            if value.is_empty() {
                return Err(invalid(
                    "dimoverride.bad_value",
                    "DIMPOST needs a non-empty template; `<>` renders the bare measurement",
                ));
            }
            style.postfix = value.to_string();
        }
        "DIMZIN" => {
            let bits = integer(variable, value)?;
            // Bits 1/2 (suppress 0 feet / 0 inches) are declared unimplemented
            // by the formatter, so only the implementable subset is admitted.
            if !matches!(bits, 0 | 4 | 8 | 12) {
                return Err(unsupported(
                    "dimoverride.unsupported_value",
                    format!(
                        "DIMZIN {bits} uses feet/inch zero-suppression bits, which this build \
                         does not implement; the admitted set is 0, 4, 8, 12"
                    ),
                ));
            }
            style.zero_suppression = bits;
        }
        "DIMSE1" => style.suppress_ext1 = boolean(variable, value)?,
        "DIMSE2" => style.suppress_ext2 = boolean(variable, value)?,
        "DIMSD1" => style.suppress_dim1 = boolean(variable, value)?,
        "DIMSD2" => style.suppress_dim2 = boolean(variable, value)?,
        "DIMDSEP" => {
            let code = integer(variable, value)?;
            let admitted = code == 0 || code == 46 || (32..=126).contains(&code);
            if !admitted {
                return Err(invalid(
                    "dimoverride.bad_value",
                    format!(
                        "DIMDSEP must be 0, 46 or a printable ASCII code (32..=126), got {code}"
                    ),
                ));
            }
            style.decimal_separator = code;
        }
        "DIMLUNIT" => {
            let format = integer(variable, value)?;
            if format != 2 {
                return Err(unsupported(
                    "dimoverride.unsupported_value",
                    format!(
                        "DIMLUNIT {format} is a non-decimal unit format; this build renders \
                         decimal (DIMLUNIT 2) only"
                    ),
                ));
            }
            style.linear_unit_format = 2;
        }
        // DIMTXSTY names a TEXTSTYLE row; the snapshot carries the resolved
        // name/identity/font, so the override is honoured by the next synthesis
        // (the measurement text is rebuilt with that style). An unknown name is
        // a value refusal, never a silent fallback to Standard.
        "DIMTXSTY" => {
            let resolved = drawing.style_by_name(value).ok_or_else(|| {
                invalid(
                    "dimoverride.bad_value",
                    format!("the drawing has no text style named '{value}'"),
                )
            })?;
            style.text_style_name = resolved.name.clone();
            style.text_style = resolved.id;
            style.text_font = resolved.resource_keys.first().cloned();
        }
        // Recognised DIMSTYLE variables with no entity-snapshot slot.
        "DIMSCALE" | "DIMCLRD" | "DIMCLRE" | "DIMCLRT" | "DIMJUST" | "DIMATFIT" | "DIMTIX"
        | "DIMTOFL" | "DIMTIH" | "DIMTOH" | "DIMDLE" | "DIMCONTINUEMODE" => {
            return Err(unsupported(
                "dimoverride.unsupported_variable",
                format!(
                    "{variable} is not carried by cad_domain::DimensionStyleValues (the entity \
                     snapshot the synthesis consumes), so it cannot be overridden per dimension"
                ),
            ))
        }
        _ => {
            return Err(invalid(
                "dimoverride.unknown_variable",
                format!(
                    "DIMOVERRIDE does not model '{variable}'; the admitted set is DIMTXT, DIMASZ, \
                     DIMEXO, DIMEXE, DIMGAP, DIMCEN, DIMDLI, DIMTAD, DIMTVP, DIMLFAC, DIMDEC, \
                     DIMRND, DIMPOST, DIMZIN, DIMSE1, DIMSE2, DIMSD1, DIMSD2, DIMDSEP, DIMLUNIT, \
                     DIMTXSTY"
                ),
            ))
        }
    }
    Ok(())
}

fn parse_number(variable: &str, value: &str) -> CadResult<f64> {
    value.parse::<f64>().map_err(|_| {
        invalid(
            "dimoverride.bad_value",
            format!("{variable} needs a finite number, got '{value}'"),
        )
    })
}

fn finite(variable: &str, value: &str) -> CadResult<f64> {
    let number = parse_number(variable, value)?;
    if !number.is_finite() {
        return Err(invalid(
            "dimoverride.bad_value",
            format!("{variable} needs a finite number, got '{value}'"),
        ));
    }
    Ok(number)
}

fn positive(variable: &str, value: &str) -> CadResult<f64> {
    let number = finite(variable, value)?;
    if number <= 0.0 {
        return Err(invalid(
            "dimoverride.bad_value",
            format!("{variable} must be strictly positive, got {number}"),
        ));
    }
    Ok(number)
}

fn non_negative(variable: &str, value: &str) -> CadResult<f64> {
    let number = finite(variable, value)?;
    if number < 0.0 {
        return Err(invalid(
            "dimoverride.bad_value",
            format!("{variable} must be non-negative, got {number}"),
        ));
    }
    Ok(number)
}

fn integer(variable: &str, value: &str) -> CadResult<i16> {
    value.parse::<i16>().map_err(|_| {
        invalid(
            "dimoverride.bad_value",
            format!("{variable} needs a whole number in i16, got '{value}'"),
        )
    })
}

fn boolean(variable: &str, value: &str) -> CadResult<bool> {
    match value.to_ascii_uppercase().as_str() {
        "ON" | "1" | "TRUE" => Ok(true),
        "OFF" | "0" | "FALSE" => Ok(false),
        _ => Err(invalid(
            "dimoverride.bad_value",
            format!("{variable} needs ON/OFF (or 1/0), got '{value}'"),
        )),
    }
}
