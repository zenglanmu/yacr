//! The stored-field grammars of `DIMSTYLE MODIFY` and `MLEADERSTYLE MODIFY`.
//!
//! Every field here exists on the domain row the table stores
//! ([`cad_db::DimStyle`] / [`cad_db::MLeaderStyle`]); a name outside these lists
//! is refused by name instead of being ignored, and a value outside its grammar
//! is refused before any write. Nothing is clamped or defaulted: `DIMTXT 0`
//! fails, it does not become `2.5`.
//!
//! Field names are accepted in the two spellings the repository already uses:
//! the AutoCAD variable (`DIMTXT`) and the stored field (`text_height`), both
//! ASCII case-insensitive.

use cad_db::{DimStyle, MLeaderStyle};
use cad_domain::MLeaderPathType;

/// Apply one `<field> <value>` pair to a stored dimension style.
///
/// `text_style_known` answers whether a text-style name resolves in the drawing's
/// STYLE table; the caller passes the table's own resolver so a `DIMTXSTY` write
/// can never store a dangling name.
pub(crate) fn apply_dim_style_field(
    style: &mut DimStyle,
    field: &str,
    value: &str,
    text_style_known: impl Fn(&str) -> bool,
) -> Result<(), String> {
    let key = field.to_ascii_lowercase();
    match key.as_str() {
        "dimtxt" | "text_height" => style.text_height = positive(value, field)?,
        "dimasz" | "arrow_size" => style.arrow_size = positive(value, field)?,
        "dimexo" | "ext_offset" => style.ext_offset = non_negative(value, field)?,
        "dimexe" | "ext_beyond" => style.ext_beyond = non_negative(value, field)?,
        "dimgap" | "text_gap" => style.text_gap = non_negative(value, field)?,
        "dimtad" | "text_vertical" => {
            let parsed = integer(value, field)?;
            if !(0..=4).contains(&parsed) {
                return Err(format!(
                    "dimstyle.value_range: {field} must be an integer in 0..=4, got {value}"
                ));
            }
            style.text_vertical = parsed;
        }
        "dimtvp" | "text_vertical_position" => style.text_vertical_position = finite(value, field)?,
        "dimscale" | "scale" => style.scale = positive(value, field)?,
        "dimdec" | "decimals" => {
            let parsed = integer(value, field)?;
            if !(0..=8).contains(&parsed) {
                return Err(format!(
                    "dimstyle.value_range: {field} must be an integer in 0..=8, got {value}"
                ));
            }
            style.decimals = parsed;
        }
        "dimzin" | "zero_suppression" => style.zero_suppression = integer(value, field)?,
        "dimlfac" | "linear_factor" => {
            let parsed = finite(value, field)?;
            if parsed == 0.0 {
                return Err(format!(
                    "dimstyle.value_range: {field} must be non-zero, got {value}"
                ));
            }
            style.linear_factor = parsed;
        }
        "dimrnd" | "rounding" => style.rounding = non_negative(value, field)?,
        "dimpost" | "postfix" => style.postfix = value.to_string(),
        "dimcen" | "center_mark" => style.center_mark = finite(value, field)?,
        "dimtxsty" | "text_style" | "text_style_name" => {
            if !text_style_known(value) {
                return Err(format!(
                    "dimstyle.unknown_text_style: the drawing has no text style named '{value}'"
                ));
            }
            style.text_style_name = value.trim().to_string();
        }
        "dimse1" | "suppress_ext1" => style.suppress_ext1 = boolean(value, field)?,
        "dimse2" | "suppress_ext2" => style.suppress_ext2 = boolean(value, field)?,
        "dimsd1" | "suppress_dim1" => style.suppress_dim1 = boolean(value, field)?,
        "dimsd2" | "suppress_dim2" => style.suppress_dim2 = boolean(value, field)?,
        "dimlunit" | "linear_unit_format" => style.linear_unit_format = integer(value, field)?,
        "dimdsep" | "decimal_separator" => style.decimal_separator = integer(value, field)?,
        "dimjust" | "text_justification" => style.text_justification = integer(value, field)?,
        "dimtih" | "text_inside_horizontal" => {
            style.text_inside_horizontal = boolean(value, field)?
        }
        "dimtoh" | "text_outside_horizontal" => {
            style.text_outside_horizontal = boolean(value, field)?
        }
        "dimatfit" | "text_fit" => style.text_fit = integer(value, field)?,
        "dimtix" | "text_inside_extensions" => {
            style.text_inside_extensions = boolean(value, field)?
        }
        "dimtofl" | "text_outside_flow_line" => {
            style.text_outside_flow_line = boolean(value, field)?
        }
        "dimclrd" | "dim_line_color" => style.dim_line_color = integer(value, field)?,
        "dimclre" | "ext_line_color" => style.ext_line_color = integer(value, field)?,
        "dimclrt" | "text_color" => style.text_color = integer(value, field)?,
        "dimdle" | "dim_line_extension" => style.dim_line_extension = non_negative(value, field)?,
        "dimdli" | "dim_line_increment" => style.dim_line_increment = positive(value, field)?,
        "dimcontinuemode" | "continue_mode" => style.continue_mode = boolean(value, field)?,
        other => {
            return Err(format!(
                "dimstyle.unknown_field: '{other}' is not a stored DIMSTYLE variable; the \
                 writable set is the fields of cad_db::DimStyle"
            ))
        }
    }
    Ok(())
}

/// Apply one `<field> <value>` pair to a stored multileader style.
pub(crate) fn apply_mleader_style_field(
    style: &mut MLeaderStyle,
    field: &str,
    value: &str,
    text_style_known: impl Fn(&str) -> bool,
) -> Result<(), String> {
    let key = field.to_ascii_lowercase();
    match key.as_str() {
        "text_height" | "45" => style.text_height = positive(value, field)?,
        "arrowhead_size" | "44" => style.arrowhead_size = positive(value, field)?,
        "landing_gap" | "42" => style.landing_gap = non_negative(value, field)?,
        "dogleg_length" | "43" => style.dogleg_length = non_negative(value, field)?,
        "text_style" | "text_style_name" | "342" => {
            if !text_style_known(value) {
                return Err(format!(
                    "mleaderstyle.unknown_text_style: the drawing has no text style named \
                     '{value}'"
                ));
            }
            style.text_style_name = value.trim().to_string();
        }
        "path_type" | "173" => {
            style.path_type = if value.eq_ignore_ascii_case("INVISIBLE")
                || value.eq_ignore_ascii_case("NONE")
                || value == "0"
            {
                MLeaderPathType::Invisible
            } else if value.eq_ignore_ascii_case("STRAIGHT")
                || value.eq_ignore_ascii_case("SEGMENTS")
                || value == "1"
            {
                MLeaderPathType::StraightLineSegments
            } else if value.eq_ignore_ascii_case("SPLINE") || value == "2" {
                MLeaderPathType::Spline
            } else {
                return Err(format!(
                    "mleaderstyle.value_range: path_type must be INVISIBLE, STRAIGHT or SPLINE, \
                     got {value}"
                ));
            };
        }
        "content_type" | "170" => {
            let parsed = integer(value, field)?;
            if !(0..=3).contains(&parsed) {
                return Err(format!(
                    "mleaderstyle.value_range: content_type must be 0..=3, got {value}"
                ));
            }
            style.content_type = parsed;
        }
        "left_attachment" | "174" => {
            let parsed = integer(value, field)?;
            if !(0..=10).contains(&parsed) {
                return Err(format!(
                    "mleaderstyle.value_range: left_attachment must be 0..=10, got {value}"
                ));
            }
            style.left_attachment = parsed;
        }
        other => {
            return Err(format!(
                "mleaderstyle.unknown_field: '{other}' is not a stored MLEADERSTYLE variable; the \
                 writable set is the fields of cad_db::MLeaderStyle"
            ))
        }
    }
    Ok(())
}

/// One finite decimal (plain or exponent) for a size/offset field.
fn finite(value: &str, field: &str) -> Result<f64, String> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|parsed| parsed.is_finite())
        .ok_or_else(|| {
            format!("dimstyle.value_number: {field} must be a finite decimal, got '{value}'")
        })
}

/// One strictly positive finite decimal.
fn positive(value: &str, field: &str) -> Result<f64, String> {
    let parsed = finite(value, field)?;
    if parsed <= 0.0 {
        return Err(format!(
            "dimstyle.value_range: {field} must be > 0, got {value}"
        ));
    }
    Ok(parsed)
}

/// One finite, non-negative decimal.
fn non_negative(value: &str, field: &str) -> Result<f64, String> {
    let parsed = finite(value, field)?;
    if parsed < 0.0 {
        return Err(format!(
            "dimstyle.value_range: {field} must be >= 0, got {value}"
        ));
    }
    Ok(parsed)
}

/// One `i16` integer.
fn integer(value: &str, field: &str) -> Result<i16, String> {
    value.trim().parse::<i16>().map_err(|_| {
        format!("dimstyle.value_integer: {field} must be an integer that fits i16, got '{value}'")
    })
}

/// One ON/OFF-style boolean (`ON`/`OFF`, `TRUE`/`FALSE`, `YES`/`NO`, `1`/`0`).
fn boolean(value: &str, field: &str) -> Result<bool, String> {
    if ["ON", "TRUE", "YES", "1"]
        .iter()
        .any(|word| value.eq_ignore_ascii_case(word))
    {
        return Ok(true);
    }
    if ["OFF", "FALSE", "NO", "0"]
        .iter()
        .any(|word| value.eq_ignore_ascii_case(word))
    {
        return Ok(false);
    }
    Err(format!(
        "dimstyle.value_boolean: {field} must be ON/OFF (or TRUE/FALSE, YES/NO, 1/0), got \
         '{value}'"
    ))
}
