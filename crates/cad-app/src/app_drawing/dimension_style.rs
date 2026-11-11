//! The `DIMSTYLE` / `-DIMSTYLE` command.
//!
//! Split out of `app_drawing::dimension_edit` so that file stays inside the
//! code-size gate; the command is still dispatched from
//! [`crate::Application::execute`] and behaves exactly as documented there.
//!
//! `LIST`/`CURRENT` are read-only reports and stay Viewer-legal. `SET` makes the
//! row the session's current dimension style (creation resolves it through
//! [`cad_db::DrawingDatabase::resolve_dim_style`]), and
//! `NEW`/`RENAME`/`DELETE`/`MODIFY` are post-import table writes through the
//! shared writer in `app_drawing::style_tables` — one transaction / one undo step
//! each, refused in Viewer mode.

use super::*;
use crate::app_reports_ext::report_outcome;
use crate::command_payload::{StyleTableKind, StyleTableOp};

impl Application {
    /// `DIMSTYLE` / `-DIMSTYLE`: the stored DIMSTYLE table.
    pub(crate) fn dimstyle_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = batch_style_payload(command)?.clone();
        let drawing = super::dimension_edit::drawing_of(self, command)?;
        match request {
            DimStyleCommand::List => {
                let mut diagnostics = Vec::new();
                for style in drawing.dim_styles() {
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "dimstyle.entry".into(),
                        message: format!(
                            "DIMSTYLE: {} (id {}, text height {}, arrow size {}, DIMDLI {})",
                            style.name,
                            style.id.0,
                            style.text_height,
                            style.arrow_size,
                            style.dim_line_increment
                        ),
                    });
                }
                if diagnostics.is_empty() {
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "dimstyle.empty".into(),
                        message: "DIMSTYLE: the drawing stores no dimension styles".into(),
                    });
                }
                Ok(report_outcome(diagnostics))
            }
            DimStyleCommand::Current => {
                // Creation resolves the session's current style (default
                // `Standard`); the report names that row, not a hard-coded one.
                let current = session.current_dim_style_name().to_string();
                let diagnostic = match drawing.dim_style_by_name(&current) {
                    Some(style) => Diagnostic {
                        object: None,
                        code: "dimstyle.current".into(),
                        message: format!(
                            "DIMSTYLE CURRENT: {} (id {}); dimension creation resolves this style",
                            style.name, style.id.0
                        ),
                    },
                    None => Diagnostic {
                        object: None,
                        code: "dimstyle.current_missing".into(),
                        message: format!(
                            "DIMSTYLE CURRENT: the drawing has no dimension style named \
                             '{current}'; creation would refuse (nothing was changed)"
                        ),
                    },
                };
                Ok(report_outcome(vec![diagnostic]))
            }
            DimStyleCommand::Set(name) => self.apply_style_table_op(
                session,
                command,
                StyleTableKind::DimStyle,
                StyleTableOp::Set { name },
            ),
            DimStyleCommand::New { name, from } => self.apply_style_table_op(
                session,
                command,
                StyleTableKind::DimStyle,
                StyleTableOp::Create { name, from },
            ),
            DimStyleCommand::Rename { old, new } => self.apply_style_table_op(
                session,
                command,
                StyleTableKind::DimStyle,
                StyleTableOp::Rename { old, new },
            ),
            DimStyleCommand::Delete(name) => self.apply_style_table_op(
                session,
                command,
                StyleTableKind::DimStyle,
                StyleTableOp::Delete { name },
            ),
            DimStyleCommand::Modify { name, assignments } => self.apply_style_table_op(
                session,
                command,
                StyleTableKind::DimStyle,
                StyleTableOp::Modify { name, assignments },
            ),
        }
    }
}

/// A named input refusal for this command.
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// Read the `DIMSTYLE` option, refusing any other dimension payload by name.
fn batch_style_payload(command: &Command) -> CadResult<&DimStyleCommand> {
    match &command.payload {
        CommandPayload::Dimension(payload) => match payload.as_ref() {
            DimensionBatchPayload::Style(style) => Ok(style),
            _ => Err(invalid(
                "dimstyle.payload",
                "DIMSTYLE takes LIST, CURRENT, SET <name>, NEW, RENAME, DELETE or MODIFY",
            )),
        },
        CommandPayload::None => Err(invalid(
            "dimstyle.needs_option",
            "the typed form needs an option on the same command line",
        )),
        _ => Err(invalid(
            "dimstyle.payload",
            "DIMSTYLE takes LIST, CURRENT, SET <name>, NEW, RENAME, DELETE or MODIFY",
        )),
    }
}
