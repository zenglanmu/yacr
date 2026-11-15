//! The typed argument lines of `SCALELISTEDIT` / `-SCALELISTEDIT`.
//!
//! Both selector spellings share this grammar and one application handler; the
//! only difference is the diagnostics' name, which the caller passes through.
//! The grammar is the AutoCAD `-SCALELISTEDIT` prompt set, minus the modal
//! dialog: `?` (report), `ADD`, `RENAME` and `DELETE`. `RESET` is admitted so it
//! can be refused **by name** (`scalelistedit.reset_unsupported`) instead of
//! being reported as an unknown word: this build has no authoritative
//! default-scale template, and restoring one would silently rewrite the table.
//!
//! Nothing is clamped or defaulted here. A malformed line returns `None` and the
//! caller refuses it before any dispatch; a well-formed line becomes the payload
//! the application's single scale-list handler consumes.
//!
//! # One declared grammar limit
//!
//! The shared command line tokenizer (`split_quoted_tokens`) treats `"` as the
//! quote delimiter and has no escape, so a scale **name that contains a double
//! quote** — AutoCAD's inch spelling, `1" = 1'-0"` — cannot be typed on this line.
//! Everything else handles such a row: the database validates it, a rename
//! cascades into the stored references, a delete counts them, and the exporter
//! writes it. Only the typed spelling is unavailable; a caller that needs it (a
//! host dialog, IPC) supplies the payload directly. The limitation is refused as
//! a malformed line rather than being silently rewritten into a different name.
//!
//! Kept in its own module only to keep `session/mod.rs` inside the file-size
//! budget (the `point_style` / `pedit` / `layer_arguments` split pattern).

use cad_app::{CommandId, CommandPayload, ExtendedCommandId, ScaleListCommand};

use super::commands::{parse_signed_decimal, DASH_SCALELIST_SELECTOR};
use super::CadSession;

/// One parsed `SCALELISTEDIT` / `-SCALELISTEDIT` line.
#[derive(Debug, Clone, PartialEq)]
pub enum ScaleListAction {
    /// `?` or a bare selector: the read-only report.
    Report,
    /// One table write.
    Write(ScaleListCommand),
    /// `RESET`: admitted and refused by name (no default template is modelled).
    Reset,
}

/// Parse the argument of a `SCALELISTEDIT` / `-SCALELISTEDIT` selector.
///
/// Shape (keywords case-insensitive, names may be double-quoted when they
/// contain spaces, e.g. `ADD "1\" = 1'-0\"" 1 12`):
///
/// * `?` — the report; anything after it is refused (there is no name filter);
/// * `ADD <name> <paper_units> <drawing_units>` — one new row. Both units are
///   signed decimals; the database owns the sign/finiteness rule and refuses a
///   non-positive value by name;
/// * `RENAME <old> <new>` — one row, cascading into every stored reference;
/// * `DELETE <name>` — one row, refused while a reference names it;
/// * `RESET` — no argument, refused by name.
///
/// Exactly one operation per line: an extra token is a malformed line rather
/// than a guessed second operation.
pub fn parse_scale_list_argument(argument: &str) -> Option<ScaleListAction> {
    let text = argument.trim();
    if text == "?" {
        return Some(ScaleListAction::Report);
    }
    let tokens = split_scale_tokens(text)?;
    let (verb, rest) = tokens.split_first()?;
    match verb.to_ascii_uppercase().as_str() {
        "ADD" => match rest {
            [name, paper, drawing] => Some(ScaleListAction::Write(ScaleListCommand::Add {
                name: non_empty(name)?,
                paper_units: scale_component(paper)?,
                drawing_units: scale_component(drawing)?,
            })),
            _ => None,
        },
        "RENAME" => match rest {
            [old, new] => Some(ScaleListAction::Write(ScaleListCommand::Rename {
                old: non_empty(old)?,
                new: non_empty(new)?,
            })),
            _ => None,
        },
        "DELETE" => match rest {
            [name] => Some(ScaleListAction::Write(ScaleListCommand::Delete {
                name: non_empty(name)?,
            })),
            _ => None,
        },
        "RESET" if rest.is_empty() => Some(ScaleListAction::Reset),
        _ => None,
    }
}

/// The stable refusal for a `SCALELISTEDIT` argument outside its grammar.
pub fn invalid_scale_list_argument_reason(text: &str) -> String {
    format!(
        "invalid SCALELISTEDIT argument: '{text}' (expected `?` for the report, `ADD <name> \
         <paper_units> <drawing_units>`, `RENAME <old> <new>` or `DELETE <name>`; quote a name \
         that contains spaces)"
    )
}

/// Split one scale-list line into tokens, refusing an **ambiguous** double quote.
///
/// The shared `commands::split_quoted_tokens` removes quote characters, which is
/// right for a name that is quoted as a whole (`"my scale"`) but silently turns
/// AutoCAD's inch spelling `1" = 1'-0"` into `1 = 1'-0`. A quote that appears
/// inside an unquoted token is therefore refused here instead of being dropped:
/// the line is malformed, not a request for a different name.
fn split_scale_tokens(text: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut token_started = false;
    for ch in text.chars() {
        match ch {
            '"' => {
                // A quote may open a token or close a quoted run — never appear
                // inside one the tokenizer did not open.
                if !quoted && !current.is_empty() {
                    return None;
                }
                quoted = !quoted;
                token_started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if token_started || !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                    token_started = false;
                }
            }
            c => {
                current.push(c);
                token_started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if token_started || !current.is_empty() {
        tokens.push(current);
    }
    Some(tokens)
}

/// A scale ratio component: one finite signed decimal.
///
/// The domain rule (both components positive) belongs to `cad-db`, which owns
/// the table; this parser only refuses text that is not a decimal at all, so a
/// `1:0` row reaches the database's named `scalelistedit.invalid_ratio` refusal
/// instead of a generic argument error.
fn scale_component(text: &str) -> Option<f64> {
    let value = parse_signed_decimal(text)?;
    value.is_finite().then_some(value)
}

fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

impl CadSession {
    /// Apply one typed `SCALELISTEDIT` / `-SCALELISTEDIT` line.
    ///
    /// A bare selector never reaches here (it keeps the name-only payload
    /// refusal); `?` dispatches the read-only report, every write dispatches the
    /// **one** application handler with the `ScaleList` payload, and `RESET` is
    /// refused by name before any dispatch.
    pub(super) fn scale_list_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let Some(action) = parse_scale_list_argument(argument) else {
            return self.refuse_argument(invalid_scale_list_argument_reason(text));
        };
        let id = if selector == DASH_SCALELIST_SELECTOR {
            CommandId::Extended(ExtendedCommandId::DashScaleListEdit)
        } else {
            CommandId::Extended(ExtendedCommandId::ScaleListEdit)
        };
        let payload = match action {
            ScaleListAction::Report => CommandPayload::None,
            ScaleListAction::Write(op) => CommandPayload::ScaleList(op),
            ScaleListAction::Reset => {
                return self.refuse_argument(
                    "scalelistedit.reset_unsupported: RESET would replace the drawing's \
                     ACAD_SCALELIST with a default template, and this build models no \
                     authoritative default scale list; nothing was changed (use `ADD`/`DELETE` to \
                     edit the table row by row)"
                        .to_string(),
                );
            }
        };
        self.dispatch_typed_argument(selector, id, payload)
    }
}
