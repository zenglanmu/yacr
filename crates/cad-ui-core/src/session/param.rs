//! The session entry of the document-level parameter list (ADR-0043 P1).
//!
//! [`CadSession`] owns the `HostCore`; this module is the thin bridge between the
//! request-carrying host-read seam and [`cad_host::HostCore::param_list`]. It
//! adds no filtering or ordering: the host reads the authoritative table and the
//! active configuration's status.
//!
//! There is no dedicated command and no React consumer in P1; the list is a
//! wire-only read over [`crate::host_read::HostReadRequest::ParamList`].

use crate::param::ParamListResponse;

use super::commands::{
    invalid_dash_parameters_argument_reason, parse_dash_parameters_argument, DashParametersOps,
    ParamTarget, DASH_PARAMETERS_SELECTOR,
};
use super::diagnostics::cad_error_text;
use super::CadSession;

impl CadSession {
    /// The document-level parameter list under the active configuration.
    ///
    /// No document is an empty answer (`active: None`, `rows: []`, `total: 0`),
    /// never an error: the caller asked a real question.
    pub fn param_list(&self) -> Result<ParamListResponse, String> {
        self.core
            .param_list()
            .map(ParamListResponse::from_host)
            .map_err(|error| cad_error_text(&error))
    }

    /// `-PARAMETERS <argument>`: the command-line Parameter Manager grammar.
    ///
    /// The names are resolved against the session's own parameter list (the same
    /// projection the panel reads), so an expression identifier and a target name
    /// mean exactly what the table says. Every op is dispatched as one
    /// [`cad_app::CommandId::Param`] payload, so the one merged write path owns
    /// the transaction, revision, history entry and every semantic refusal.
    pub(super) fn param_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let list = self.param_list()?;
        let resolve = |name: &str| {
            list.rows
                .iter()
                .find(|row| row.name == name)
                .map(|row| row.seq)
        };
        let ops = match parse_dash_parameters_argument(argument, resolve) {
            Ok(ops) => ops,
            Err(error) if error.code() == "param.unknown_reference" => {
                return self.refuse_argument(format!("{}: {}", error.code(), error.detail()));
            }
            Err(_) => {
                return self.refuse_argument(invalid_dash_parameters_argument_reason(text));
            }
        };
        let (id, payload) = match ops {
            DashParametersOps::List => {
                (cad_app::CommandId::ParamList, cad_app::CommandPayload::None)
            }
            DashParametersOps::Create { name, kind, expr } => (
                cad_app::CommandId::Param,
                cad_app::CommandPayload::Param(cad_app::ParamOp::Create { name, kind, expr }),
            ),
            DashParametersOps::SetValue { target, expr } => (
                cad_app::CommandId::Param,
                cad_app::CommandPayload::Param(cad_app::ParamOp::SetValue {
                    seq: self.param_target_seq(target)?,
                    expr,
                }),
            ),
            DashParametersOps::Rename { target, name } => (
                cad_app::CommandId::Param,
                cad_app::CommandPayload::Param(cad_app::ParamOp::Rename {
                    seq: self.param_target_seq(target)?,
                    name,
                }),
            ),
            DashParametersOps::Delete { target } => (
                cad_app::CommandId::Param,
                cad_app::CommandPayload::Param(cad_app::ParamOp::Delete {
                    seq: self.param_target_seq(target)?,
                }),
            ),
        };
        self.dispatch_typed_argument(DASH_PARAMETERS_SELECTOR, id, payload)
    }

    /// Resolve one parsed target: a resolved/integer sequence passes through; an
    /// unknown name is refused by name, never guessed into a sequence.
    fn param_target_seq(&self, target: ParamTarget) -> Result<cad_domain::param::ParamSeq, String> {
        match target {
            ParamTarget::Seq(seq) => Ok(cad_domain::param::ParamSeq(seq)),
            ParamTarget::Name(name) => {
                let reason =
                    format!("param.unknown_name: this document has no parameter named {name:?}");
                *self.error.borrow_mut() = Some(reason.clone());
                Err(reason)
            }
        }
    }
}
