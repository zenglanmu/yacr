//! The typed argument forms of the 2026-11-07 layout / viewport batch.
//!
//! Each selector's grammar lives in
//! [`super::commands::layout`](super::commands); this module only dispatches the
//! parsed [`cad_app::LayoutPayload`] (or the `MVIEW` corner pair) to its
//! [`CommandId`]. The selector name, never the typed value, is echoed in the
//! command log, exactly like the other typed-argument paths.
//!
//! A bare selector (`LAYOUT`, `VPORTS`, …) is admitted by name and refused by
//! the application for a missing payload; an argument outside the grammar is
//! refused here with the parser's stable sentence and changes nothing. A bare
//! `PAGESETUP`/`-PAGESETUP` keeps the read-only page-setup report; bare
//! `PSETUPIN` and `MVIEW` are refused by the application for the missing
//! payload.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId};

use super::commands::{
    parse_layout_argument, parse_layoutmerge_argument, parse_layoutwizard_argument,
    parse_layvpi_argument, parse_mview_argument, parse_pagesetup_argument, parse_psetupin_argument,
    parse_vpclip_argument, parse_vports_argument, parse_vpscale_argument, DASH_PAGESETUP_SELECTOR,
    DASH_PSETUPIN_SELECTOR, LAYOUTMERGE_SELECTOR, LAYOUTWIZARD_SELECTOR, LAYOUT_SELECTOR,
    LAYVPI_SELECTOR, MVIEW_SELECTOR, PAGESETUP_SELECTOR, PSETUPIN_SELECTOR, VPCLIP_SELECTOR,
    VPORTS_SELECTOR, VPSCALE_SELECTOR,
};
use super::CadSession;

impl CadSession {
    /// Dispatch one typed layout/viewport/page-setup argument line.
    pub(super) fn layout_argument(
        &self,
        selector: &'static str,
        argument: &str,
        _text: &str,
    ) -> Result<(), String> {
        let (id, payload) = match selector {
            LAYOUT_SELECTOR => (CommandId::Layout, parse_layout_argument(argument)),
            LAYOUTMERGE_SELECTOR => (CommandId::LayoutMerge, parse_layoutmerge_argument(argument)),
            LAYOUTWIZARD_SELECTOR => (
                CommandId::LayoutWizard,
                parse_layoutwizard_argument(argument),
            ),
            PAGESETUP_SELECTOR => (
                CommandId::Extended(ExtendedCommandId::PageSetup),
                parse_pagesetup_argument(argument),
            ),
            DASH_PAGESETUP_SELECTOR => (
                CommandId::Extended(ExtendedCommandId::DashPageSetup),
                parse_pagesetup_argument(argument),
            ),
            PSETUPIN_SELECTOR => (
                CommandId::Extended(ExtendedCommandId::PageSetupIn),
                parse_psetupin_argument(argument),
            ),
            DASH_PSETUPIN_SELECTOR => (
                CommandId::Extended(ExtendedCommandId::DashPageSetupIn),
                parse_psetupin_argument(argument),
            ),
            VPORTS_SELECTOR => (CommandId::Vports, parse_vports_argument(argument)),
            VPCLIP_SELECTOR => (CommandId::VpClip, parse_vpclip_argument(argument)),
            VPSCALE_SELECTOR => (CommandId::VpScale, parse_vpscale_argument(argument)),
            _ => {
                debug_assert_eq!(selector, LAYVPI_SELECTOR);
                (CommandId::LayVpi, parse_layvpi_argument(argument))
            }
        };
        let payload = match payload {
            Ok(payload) => payload,
            Err(reason) => return self.refuse_argument(reason),
        };
        self.core.record_command(selector);
        self.dispatch_payload(id, CommandPayload::Layout(Box::new(payload)))
    }

    /// Dispatch one typed `MVIEW <minx> <miny> <maxx> <maxy>` line.
    ///
    /// The four corners ride the ordinary [`CommandPayload::Points`] channel the
    /// other two-corner commands use; the application validates the paper space
    /// and the rectangle ordering again before writing one viewport.
    pub(super) fn mview_argument(&self, argument: &str, _text: &str) -> Result<(), String> {
        let (min, max) = match parse_mview_argument(argument) {
            Ok(pair) => pair,
            Err(reason) => return self.refuse_argument(reason),
        };
        self.core.record_command(MVIEW_SELECTOR);
        self.dispatch_payload(
            CommandId::Extended(ExtendedCommandId::MView),
            CommandPayload::Points(vec![min, max]),
        )
    }
}
