//! The **one** line-break / stored-type-key authority shared by MTEXT creation
//! and `TEXTEDIT` (ADR-0021 D4).
//!
//! `SemanticGeometry::Text` stores the **raw** source string. Only an `AcDbMText`
//! source interprets `\P` in the viewport (`cad-representation`'s MTEXT parser),
//! so every writer must agree on how a line break becomes a stored value:
//!
//! * a **multi-line** text (`AcDbMText`, and a MULTILEADER whose content is text)
//!   stores `\r\n`, `\r` and `\n` as `\P`;
//! * a **single-line** TEXT (`AcDbText`) does **not** interpret `\P`, so a draft
//!   containing a line break is **refused** by name
//!   ([`single_line_newline_error`]) instead of being stored as a value the
//!   renderer draws over several lines and `TEXTEDIT` can never produce again;
//! * every other control code passes through verbatim (this is a raw string, not
//!   a rich-text editor).
//!
//! The mapping and the refusal live here so MTEXT creation and the edit channels
//! cannot drift; `text_edit` documents the policy, it does not own a copy.

use super::*;

/// The stored DXF type key of a **multiline** text entity.
///
/// A created `Text` geometry normally stamps `class_name()` (`AcDbText`); MTEXT
/// creation overrides that key through the shared create funnel so the newline
/// policy and the export path both treat the entity as MTEXT.
pub(crate) const MTEXT_TYPE_KEY: &str = "AcDbMText";

/// Whether `text` contains a line break (`\r` or `\n`).
///
/// The predicate is kept next to the mapping so a caller cannot test one form
/// (`\n`) and then convert another (`\r`).
pub(crate) fn has_line_break(text: &str) -> bool {
    text.contains(['\r', '\n'])
}

/// The named refusal for a **single-line** TEXT draft carrying a line break.
///
/// `context` names the entry point (`CreateText`, `TEXT`, …) so the message says
/// which channel refused; the stable machine key is
/// `text.single_line_newline`.
pub(crate) fn single_line_newline_error(context: &str) -> CadError {
    CadError::InvalidInput(format!(
        "text.single_line_newline: {context} is a single-line TEXT, which does not interpret \\P; \
         use MTEXT for multiple lines"
    ))
}

/// `\r\n`, `\r` and `\n` → `\P`, in one pass.
///
/// `\P` is the only paragraph-break form the viewport's MTEXT parser
/// (`cad-representation`) interprets, so the stored string stays consistent
/// between our renderer and AutoCAD. No other code is inspected or rewritten.
pub(crate) fn newlines_to_paragraph_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                // A CRLF pair is one line break, not two.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\P");
            }
            '\n' => out.push_str("\\P"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_break_form_becomes_one_paragraph_break() {
        assert_eq!(newlines_to_paragraph_breaks("a\nb"), "a\\Pb");
        assert_eq!(newlines_to_paragraph_breaks("a\rb"), "a\\Pb");
        assert_eq!(newlines_to_paragraph_breaks("a\r\nb"), "a\\Pb");
        assert_eq!(newlines_to_paragraph_breaks("a\r\n\r\nb"), "a\\P\\Pb");
        assert_eq!(newlines_to_paragraph_breaks("plain"), "plain");
        // Every other code passes through verbatim.
        assert_eq!(newlines_to_paragraph_breaks("{\\fArial;x}"), "{\\fArial;x}");
        assert!(has_line_break("a\nb") && has_line_break("a\rb"));
        assert!(!has_line_break("a\\Pb"));
        let error = single_line_newline_error("CreateText");
        match error {
            CadError::InvalidInput(message) => {
                assert!(
                    message.starts_with("text.single_line_newline: "),
                    "{message}"
                );
                assert!(message.contains("CreateText"), "{message}");
            }
            other => panic!("expected an InvalidInput refusal, got {other:?}"),
        }
    }
}
