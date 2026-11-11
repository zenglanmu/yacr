//! The typed argument grammars of the named style tables:
//! `STYLE`, `-STYLE` and `MLEADERSTYLE`.
//!
//! These are **argument forms of canonical selectors**, not new commands: the
//! bare selector keeps the read-only table report, and a typed line names one
//! operation the application already models (`cad_app::StyleTableOp`). The
//! parsers are pure — they only tokenise and split — because the database owns
//! name validation, duplicate detection and delete safety, and the application
//! owns each table's field grammar.
//!
//! | selector | typed forms |
//! |---|---|
//! | `STYLE` / `-STYLE` | `?`, `NEW <name> [<base>]`, `RENAME <old> <new>`, `DELETE <name>`, `SET <name> <FONT\|KEYS\|ADDKEY\|REMOVEKEY> <value> …` |
//! | `MLEADERSTYLE` | `?`, `NEW <name> [<base>]`, `RENAME <old> <new>`, `DELETE <name>`, `SET <name>`, `MODIFY <name> <field> <value> …` |
//!
//! Names are one token (quote a name containing spaces). `SET`/`MODIFY` require
//! at least one `<field> <value>` pair and an even token count after the name,
//! so a half-written pair is refused here rather than reaching the handler.
//! `DIMSTYLE` keeps its grammar in `session::dim_arguments`.

use cad_app::{
    CommandId, CommandPayload, ExtendedCommandId, StyleTableCommand, StyleTableKind, StyleTableOp,
};

use super::arguments::split_quoted_tokens;
use crate::session::diagnostics::cad_error_text;
use crate::session::CadSession;

/// `STYLE`: the drawing's text-style table (report or write form).
pub const STYLE_SELECTOR: &str = "STYLE";

/// `-STYLE`: the command-line spelling; same table and same grammar.
pub const DASH_STYLE_SELECTOR: &str = "-STYLE";

/// `MLEADERSTYLE`: the multileader-style table.
pub const MLEADERSTYLE_SELECTOR: &str = "MLEADERSTYLE";

/// Parse one `STYLE`/`-STYLE` argument.
pub fn parse_text_style_argument(argument: &str) -> Option<StyleTableCommand> {
    Some(StyleTableCommand {
        kind: StyleTableKind::TextStyle,
        op: parse_style_op(argument, StyleTableKind::TextStyle)?,
    })
}

/// Parse one `MLEADERSTYLE` argument.
pub fn parse_mleader_style_argument(argument: &str) -> Option<StyleTableCommand> {
    Some(StyleTableCommand {
        kind: StyleTableKind::MLeaderStyle,
        op: parse_style_op(argument, StyleTableKind::MLeaderStyle)?,
    })
}

/// The stable refusal for an argument outside a style selector's grammar.
pub fn invalid_style_argument_reason(selector: &str, text: &str) -> String {
    let forms = match selector {
        STYLE_SELECTOR | DASH_STYLE_SELECTOR => {
            "?, NEW <name> [<base>], RENAME <old> <new>, DELETE <name> or \
             SET <name> <FONT|KEYS|ADDKEY|REMOVEKEY> <value>"
        }
        _ => {
            "?, NEW <name> [<base>], RENAME <old> <new>, DELETE <name>, SET <name> or \
             MODIFY <name> <field> <value>"
        }
    };
    format!(
        "unsupported or malformed {selector} argument in `{}`: the typed forms are {forms}; \
         quote names that contain spaces (nothing was changed)",
        text.trim()
    )
}

/// Shared parser: one operation or `None` when the line is outside the grammar.
fn parse_style_op(argument: &str, kind: StyleTableKind) -> Option<StyleTableOp> {
    let tokens = split_quoted_tokens(argument)?;
    let first = tokens.first()?.to_ascii_uppercase();
    let op = match first.as_str() {
        "?" | "LIST" if tokens.len() == 1 => StyleTableOp::List,
        "NEW" if tokens.len() == 2 || tokens.len() == 3 => StyleTableOp::Create {
            name: tokens[1].clone(),
            from: tokens.get(2).cloned(),
        },
        "RENAME" if tokens.len() == 3 => StyleTableOp::Rename {
            old: tokens[1].clone(),
            new: tokens[2].clone(),
        },
        "DELETE" | "DEL" if tokens.len() == 2 => StyleTableOp::Delete {
            name: tokens[1].clone(),
        },
        "SET" if kind == StyleTableKind::MLeaderStyle && tokens.len() == 2 => StyleTableOp::Set {
            name: tokens[1].clone(),
        },
        "SET" | "MODIFY"
            if kind == StyleTableKind::TextStyle
                && tokens.len() >= 4
                && (tokens.len() - 2) % 2 == 0 =>
        {
            StyleTableOp::Modify {
                name: tokens[1].clone(),
                assignments: tokenize_assignments(&tokens),
            }
        }
        "MODIFY" if tokens.len() >= 4 && (tokens.len() - 2) % 2 == 0 => StyleTableOp::Modify {
            name: tokens[1].clone(),
            assignments: tokenize_assignments(&tokens),
        },
        _ => return None,
    };
    Some(op)
}

fn tokenize_assignments(tokens: &[String]) -> Vec<(String, String)> {
    let mut assignments = Vec::new();
    let mut index = 2;
    while index + 1 < tokens.len() {
        assignments.push((tokens[index].clone(), tokens[index + 1].clone()));
        index += 2;
    }
    assignments
}

impl CadSession {
    /// Dispatch one typed `STYLE`/`-STYLE`/`MLEADERSTYLE` line.
    pub(crate) fn style_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let (id, payload) = match selector {
            STYLE_SELECTOR => (
                CommandId::Extended(ExtendedCommandId::TextStyle),
                parse_text_style_argument(argument),
            ),
            DASH_STYLE_SELECTOR => (
                CommandId::Extended(ExtendedCommandId::DashStyle),
                parse_text_style_argument(argument),
            ),
            MLEADERSTYLE_SELECTOR => (
                CommandId::Extended(ExtendedCommandId::MLeaderStyleList),
                parse_mleader_style_argument(argument),
            ),
            _ => {
                return self
                    .refuse_argument(format!("unsupported style-table selector: {selector}"))
            }
        };
        let Some(command) = payload else {
            return self.refuse_argument(invalid_style_argument_reason(selector, text));
        };
        self.core.record_command(selector);
        match self.core.command(id, CommandPayload::StyleTable(command)) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}
