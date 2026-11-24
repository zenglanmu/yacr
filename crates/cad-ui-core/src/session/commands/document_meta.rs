//! The typed `DWGPROPS` summary-metadata grammar.
//!
//! `DWGPROPS` is one selector: the bare name is the read-only report, and the
//! typed forms write exactly one value through the ordinary command funnel:
//!
//! * `TITLE <text>`, `AUTHOR <text>`, `SUBJECT <text>`, `KEYWORDS <text>`,
//!   `COMMENTS <text>` — the five AutoCAD Drawing Properties ▸ Summary fields.
//!   The text is the rest of the line, so unquoted spaces are kept. A fully
//!   quoted empty token (`TITLE ""`) clears the field; a bare `TITLE` with no
//!   text is refused instead of guessing "clear".
//! * `CUSTOM <name> <text>` — one custom property. A name with spaces is
//!   quoted (`CUSTOM "My Name" <text>`); an empty value removes the property.
//!   An empty name is refused.
//!
//! Parsing stops here: length bounds, NUL refusal and the one-revision write
//! are `cad-app`'s contract. This module only splits the typed line, so a
//! malformed field name is answered before any dispatch.

use cad_app::DwgPropsCommand;

use super::arguments::split_quoted_tokens;

/// `DWGPROPS <field> <text>` / `DWGPROPS CUSTOM <name> <text>`: the summary
/// metadata write form of the report selector.
pub const DWGPROPS_SELECTOR: &str = "DWGPROPS";

/// Why one `DWGPROPS` argument line was refused before dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DwgPropsArgumentError {
    /// Outside the declared grammar (missing text, unbalanced quotes, …).
    Malformed,
    /// A field word that is not TITLE/AUTHOR/SUBJECT/KEYWORDS/COMMENTS/CUSTOM.
    UnknownField(String),
}

/// Parse one typed `DWGPROPS` line.
pub fn parse_dwgprops_argument(argument: &str) -> Result<DwgPropsCommand, DwgPropsArgumentError> {
    let tokens = split_quoted_tokens(argument.trim()).ok_or(DwgPropsArgumentError::Malformed)?;
    let Some(field) = tokens.first() else {
        return Err(DwgPropsArgumentError::Malformed);
    };
    if field.eq_ignore_ascii_case("CUSTOM") {
        let Some(name) = tokens.get(1) else {
            return Err(DwgPropsArgumentError::Malformed);
        };
        if name.trim().is_empty() {
            return Err(DwgPropsArgumentError::Malformed);
        }
        // Everything after the name is the value, so unquoted spaces survive.
        // `CUSTOM "name"` with no value is "remove", but the grammar requires
        // the caller to say so with an explicit empty token, exactly like the
        // named fields; a missing remainder is malformed.
        let value = tokens.get(2..).map(|rest| rest.join(" "));
        let Some(value) = value else {
            return Err(DwgPropsArgumentError::Malformed);
        };
        if value.is_empty() && tokens.len() == 2 {
            // `CUSTOM name ""` is tokens ["CUSTOM", "name", ""] (len 3); a bare
            // `CUSTOM name` (len 2) never silently removes the property.
            return Err(DwgPropsArgumentError::Malformed);
        }
        return Ok(DwgPropsCommand::SetCustom {
            name: name.trim().to_string(),
            value,
        });
    }
    let field = match field.to_ascii_uppercase().as_str() {
        "TITLE" => cad_db::DocumentMetadataField::Title,
        "AUTHOR" => cad_db::DocumentMetadataField::Author,
        "SUBJECT" => cad_db::DocumentMetadataField::Subject,
        "KEYWORDS" => cad_db::DocumentMetadataField::Keywords,
        "COMMENTS" => cad_db::DocumentMetadataField::Comments,
        other => return Err(DwgPropsArgumentError::UnknownField(other.to_string())),
    };
    if tokens.len() < 2 {
        return Err(DwgPropsArgumentError::Malformed);
    }
    // The rest of the line is the text; a quoted empty token stays empty and
    // therefore clears the field.
    let value = tokens[1..].join(" ");
    Ok(DwgPropsCommand::SetField { field, value })
}
