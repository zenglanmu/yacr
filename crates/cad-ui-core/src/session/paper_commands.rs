//! Dispatch for the paper-space/session/point-style typed batch:
//! `VPLAYER`, `MVSETUP`, `APERTURE`, `PTYPE` and `REVCLOUDPROPERTIES`.
//!
//! The grammar lives in [`super::commands::paper_arguments`]; this module turns
//! the parsed value into the typed [`cad_app::CommandPayload`] the application
//! already accepts and dispatches it through the ordinary host funnel. No second
//! IPC and no second write path.
//!
//! * `VPLAYER`/`MVSETUP` ride the existing [`cad_app::CommandPayload::LayerWalk`]
//!   token carrier (the application parses and validates its own grammar);
//!   `VPLAYER ?` carries `CommandPayload::None` and stays the read-only report.
//! * `APERTURE` and `REVCLOUDPROPERTIES` ride the same token carrier for their
//!   one validated value; the application **re-validates** it before writing the
//!   session token, so a malformed request can never slip past a second caller.
//! * `PTYPE` delegates to the existing `PDMODE`/`PDSIZE` write path
//!   ([`cad_app::CommandId::SetPointDisplayMode`] /
//!   [`cad_app::CommandId::SetPointDisplaySize`]); there is no second point-style
//!   implementation. The line echoes the admitted `PTYPE` selector once.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId};

use super::commands::{
    invalid_mvsetup_argument_reason, invalid_spacetrans_argument_reason,
    invalid_vplayer_argument_reason, parse_aperture_argument, parse_mvsetup_argument,
    parse_ptype_argument, parse_revcloud_properties_argument, parse_spacetrans_argument,
    parse_vplayer_argument, VPlayerArgument, APERTURE_SELECTOR, MVSETUP_SELECTOR, PTYPE_SELECTOR,
    REVCLOUDPROPERTIES_SELECTOR, SPACETRANS_SELECTOR, VPLAYER_SELECTOR,
};
use super::CadSession;

impl CadSession {
    /// Route one typed paper-space/session/point-style line to its payload path.
    ///
    /// A refusal is recorded in [`CadSession`]'s error and changes nothing.
    pub(super) fn paper_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            VPLAYER_SELECTOR => self.vplayer_argument(argument, text),
            MVSETUP_SELECTOR => self.mvsetup_argument(argument, text),
            APERTURE_SELECTOR => self.aperture_argument(argument, text),
            PTYPE_SELECTOR => self.ptype_argument(argument, text),
            REVCLOUDPROPERTIES_SELECTOR => self.revcloud_properties_argument(argument, text),
            SPACETRANS_SELECTOR => self.spacetrans_argument(argument, text),
            _ => self.refuse_argument(format!("unsupported paper selector: {selector}")),
        }
    }

    /// `VPLAYER FREEZE|THAW <layer,...> [<index>]`, or the `?` report.
    fn vplayer_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let payload = match parse_vplayer_argument(argument) {
            Some(VPlayerArgument::Report) => CommandPayload::None,
            Some(VPlayerArgument::Tokens(tokens)) => CommandPayload::LayerWalk(tokens),
            None => return self.refuse_argument(invalid_vplayer_argument_reason(text)),
        };
        self.dispatch_typed_argument(
            VPLAYER_SELECTOR,
            CommandId::Extended(ExtendedCommandId::VPlayer),
            payload,
        )
    }

    /// `MVSETUP <rows> <cols> <scale>`: the grid tokens the handler validates.
    fn mvsetup_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(tokens) = parse_mvsetup_argument(argument) else {
            return self.refuse_argument(invalid_mvsetup_argument_reason(text));
        };
        self.dispatch_typed_argument(
            MVSETUP_SELECTOR,
            CommandId::Extended(ExtendedCommandId::MvSetup),
            CommandPayload::LayerWalk(tokens),
        )
    }

    /// `APERTURE <n>`: one validated object-snap aperture value.
    fn aperture_argument(&self, argument: &str, _text: &str) -> Result<(), String> {
        let value = match parse_aperture_argument(argument) {
            Ok(value) => value,
            Err(reason) => return self.refuse_argument(reason),
        };
        self.dispatch_typed_argument(
            APERTURE_SELECTOR,
            CommandId::Extended(ExtendedCommandId::Aperture),
            CommandPayload::LayerWalk(vec![value.to_string()]),
        )
    }

    /// `PTYPE <code> [size]`: dispatch the existing point-style writes.
    ///
    /// Only the admitted selector is echoed. Each header write is a separate
    /// `cad-app` command with no undo entry (there is no header patch kind), so
    /// the pair is two document revisions by construction, never a fabricated
    /// combined transaction.
    fn ptype_argument(&self, argument: &str, _text: &str) -> Result<(), String> {
        let parsed = match parse_ptype_argument(argument) {
            Ok(parsed) => parsed,
            Err(reason) => return self.refuse_argument(reason),
        };
        self.core.record_command(PTYPE_SELECTOR);
        self.dispatch_payload(
            CommandId::SetPointDisplayMode,
            CommandPayload::PointDisplayMode(parsed.mode),
        )?;
        if let Some(size) = parsed.size {
            self.dispatch_payload(
                CommandId::SetPointDisplaySize,
                CommandPayload::PointDisplaySize(size),
            )?;
        }
        Ok(())
    }

    /// `REVCLOUDPROPERTIES <arc-length>`: one validated session chord value.
    fn revcloud_properties_argument(&self, argument: &str, _text: &str) -> Result<(), String> {
        let value = match parse_revcloud_properties_argument(argument) {
            Ok(value) => value,
            Err(reason) => return self.refuse_argument(reason),
        };
        self.dispatch_typed_argument(
            REVCLOUDPROPERTIES_SELECTOR,
            CommandId::Extended(ExtendedCommandId::RevCloudProperties),
            CommandPayload::LayerWalk(vec![format!("{value}")]),
        )
    }

    /// `SPACETRANS <distance>`: one model/paper conversion request.
    fn spacetrans_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(distance) = parse_spacetrans_argument(argument) else {
            return self.refuse_argument(invalid_spacetrans_argument_reason(text));
        };
        self.dispatch_typed_argument(
            SPACETRANS_SELECTOR,
            CommandId::Extended(ExtendedCommandId::SpaceTrans),
            CommandPayload::SpaceTrans(distance),
        )
    }
}
