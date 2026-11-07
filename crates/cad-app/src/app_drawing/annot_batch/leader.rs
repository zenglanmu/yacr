//! `QLEADER` / `LEADER`: the legacy leader names mapped onto the existing
//! `MLEADER` creation path.
//!
//! Both commands build the ordinary [`CommandPayload::MLeader`] payload and
//! call [`Application::create_mleader`], so there is exactly one construction
//! point (style resolution, geometry policy, text validation and the
//! one-transaction / one-undo funnel). No `AcDbLeader` entity is created.

use super::*;

impl Application {
    /// `QLEADER` / `LEADER`: create one MULTILEADER from the typed arrow,
    /// landing and text.
    pub(crate) fn leader_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        id: CommandId,
    ) -> CadResult<CommandOutcome> {
        let prefix = if id == CommandId::QLeader {
            "qleader"
        } else {
            "leader"
        };
        let request = match &command.payload {
            CommandPayload::Annotation(payload) => match payload.as_ref() {
                AnnotationBatchPayload::Leader(request) => request.clone(),
                _ => {
                    return Err(invalid(
                        &format!("{prefix}.payload"),
                        format!(
                            "{prefix} takes `<arrow x,y> <landing x,y> <text>` (the selection is \
                             not its input)"
                        ),
                    ))
                }
            },
            CommandPayload::None => {
                return Err(invalid(
                    &format!("{prefix}.needs_arguments"),
                    format!(
                        "{prefix} needs `<arrow x,y> <landing x,y> <text>`; a bare selector starts \
                         no capture in this build"
                    ),
                ))
            }
            _ => {
                return Err(invalid(
                    &format!("{prefix}.payload"),
                    format!(
                        "{prefix} takes `<arrow x,y> <landing x,y> <text>`; the selection is not \
                         its input"
                    ),
                ))
            }
        };
        if !is_finite_point(request.arrow) || !is_finite_point(request.landing) {
            return Err(invalid(
                &format!("{prefix}.invalid_point"),
                "both the arrow and the landing point must be finite",
            ));
        }
        // Reuse the one MLEADER creation path by rebuilding its ordinary
        // payload; `create_mleader` owns every style/geometry/text refusal and
        // the single transaction.
        let mleader = Command {
            schema_version: command.schema_version,
            id: CommandId::MLeader,
            document: command.document,
            viewport: command.viewport,
            payload: CommandPayload::MLeader {
                arrow: request.arrow,
                landing: request.landing,
                text: request.text,
            },
        };
        self.create_mleader(session, &mleader)
    }
}
