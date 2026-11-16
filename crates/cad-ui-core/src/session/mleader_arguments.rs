//! Typed dispatch for the 2026-11-12 MULTILEADER / MLEDIT / TOLERANCE lines.
//!
//! Pure parsing lives in [`super::commands::mleader`]; this module maps one
//! admitted selector to its [`CommandId`] and dispatches the payload through the
//! same `HostCore::command` funnel every other typed form uses. An argument
//! outside a selector's grammar is refused before IPC with a stable reason, and an
//! admitted `MLEADER` option this build's MULTILEADER domain cannot carry is
//! refused by name (`mleader.option_text_angle`) rather than dropped.
//!
//! `MLEADERCOLLECT` takes no argument, so it never reaches this module: its bare
//! selector dispatches with [`CommandPayload::None`] and the current selection.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId};

use super::commands::{
    parse_mleader_argument, parse_mleaderalign_argument, parse_mleaderedit_argument,
    parse_mledit_argument, parse_tolerance_argument, MLeaderParseError, DASH_MLEDIT_SELECTOR,
    MLEADERALIGN_SELECTOR, MLEADEREDIT_SELECTOR, MLEADER_SELECTOR, MLEDIT_SELECTOR,
    TOLERANCE_SELECTOR,
};
use super::diagnostics::cad_error_text;
use super::CadSession;

/// The stable refusal for an `MLEDIT` / `-MLEDIT` argument outside the grammar.
pub(crate) fn invalid_mledit_argument_reason(text: &str) -> String {
    format!(
        "mledit.argument: unsupported or malformed MLEDIT argument in `{}`: the typed forms are \
         `MLEDIT MERGE|CLOSED|CROSS|TEE <x> <y>`, `MLEDIT OPEN <x> <y>` and `MLEDIT BREAK <x1> \
         <y1> <x2> <y2>` (`-MLEDIT` accepts the same grammar); nothing was changed",
        text.trim()
    )
}

/// The stable refusal for a `MLEADER` typed line outside the grammar.
pub(crate) fn invalid_mleader_argument_reason(text: &str) -> String {
    format!(
        "mleader.argument: unsupported or malformed MLEADER argument in `{}`: the typed form is \
         `MLEADER [DOGLEG <positive length>] [DOGLEGANGLE <degrees>] [ATTACH <1-3> <0-10>] \
         <arrow_x> <arrow_y> <landing_x> <landing_y> <text>` (options first, text last); nothing \
         was changed",
        text.trim()
    )
}

/// The stable refusal for a `MLEADERALIGN` / `MLEADEREDIT` argument outside the
/// grammar.
pub(crate) fn invalid_mleader_edit_argument_reason(selector: &str, text: &str) -> String {
    let (code, forms) = if selector == MLEADERALIGN_SELECTOR {
        (
            "mleaderalign.argument",
            "`MLEADERALIGN LEFT|RIGHT|CENTER [positive spacing]`",
        )
    } else {
        (
            "mleaderedit.argument",
            "`MLEADEREDIT ADD <x> <y>`, `MLEADEREDIT REMOVE <0-based line index>`, `MLEADEREDIT \
             PATHTYPE <index> LINE|SPLINE|INVISIBLE`, `MLEADEREDIT ARROWSIZE <index> <positive \
             size>` or `MLEADEREDIT BYSTYLE <index> PATH|ARROW`",
        )
    };
    format!(
        "{code}: unsupported or malformed {selector} argument in `{}`: the typed form is {forms}; \
         nothing was changed",
        text.trim()
    )
}

/// The stable refusal for a `TOLERANCE` argument outside the grammar.
pub(crate) fn invalid_tolerance_argument_reason(text: &str) -> String {
    format!(
        "tolerance.argument: unsupported or malformed TOLERANCE argument in `{}`: the typed form \
         is `TOLERANCE <x> <y> <text>` with `|`-separated compartments; nothing was changed",
        text.trim()
    )
}

impl CadSession {
    pub(super) fn mleader_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            MLEDIT_SELECTOR | DASH_MLEDIT_SELECTOR => {
                let Some(payload) = parse_mledit_argument(argument) else {
                    return self.refuse_argument(invalid_mledit_argument_reason(text));
                };
                let id = if selector == DASH_MLEDIT_SELECTOR {
                    CommandId::Extended(ExtendedCommandId::DashMlEdit)
                } else {
                    CommandId::Extended(ExtendedCommandId::MlEdit)
                };
                self.core.record_command(selector);
                self.submit(id, CommandPayload::MlEdit(Box::new(payload)))
            }
            MLEADER_SELECTOR => {
                match parse_mleader_argument(argument) {
                    Ok(input) => {
                        self.core.record_command(selector);
                        self.submit(
                            CommandId::MLeader,
                            CommandPayload::MLeader {
                                arrow: input.arrow,
                                landing: input.landing,
                                text: input.text,
                                dogleg_length: input.dogleg_length,
                                dogleg_angle: input.dogleg_angle,
                                attachment: input.attachment,
                            },
                        )
                    }
                    // An admitted option the MULTILEADER domain cannot carry: a
                    // named refusal that writes nothing, never a dropped field.
                    Err(MLeaderParseError::Unmodelled(option)) => self.refuse_argument(format!(
                        "{}: the MLEADER option in `{}` cannot be written: {}; nothing was \
                             changed",
                        option.code(),
                        text.trim(),
                        option.detail()
                    )),
                    Err(MLeaderParseError::Invalid) => {
                        self.refuse_argument(invalid_mleader_argument_reason(text))
                    }
                }
            }
            MLEADERALIGN_SELECTOR => {
                let Some(payload) = parse_mleaderalign_argument(argument) else {
                    return self
                        .refuse_argument(invalid_mleader_edit_argument_reason(selector, text));
                };
                self.core.record_command(selector);
                self.submit(
                    CommandId::Extended(ExtendedCommandId::MLeaderAlign),
                    CommandPayload::MLeaderAlign(Box::new(payload)),
                )
            }
            MLEADEREDIT_SELECTOR => {
                let Some(payload) = parse_mleaderedit_argument(argument) else {
                    return self
                        .refuse_argument(invalid_mleader_edit_argument_reason(selector, text));
                };
                self.core.record_command(selector);
                self.submit(
                    CommandId::Extended(ExtendedCommandId::MLeaderEdit),
                    CommandPayload::MLeaderEdit(Box::new(payload)),
                )
            }
            legal => {
                debug_assert_eq!(legal, TOLERANCE_SELECTOR);
                let Some(payload) = parse_tolerance_argument(argument) else {
                    return self.refuse_argument(invalid_tolerance_argument_reason(text));
                };
                self.core.record_command(selector);
                self.submit(
                    CommandId::Extended(ExtendedCommandId::Tolerance),
                    CommandPayload::Tolerance(Box::new(payload)),
                )
            }
        }
    }

    /// Dispatch one payload and fold the result into the session error slot.
    fn submit(&self, id: CommandId, payload: CommandPayload) -> Result<(), String> {
        match self.core.command(id, payload) {
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
