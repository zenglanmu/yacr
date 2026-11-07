//! The typed argument forms of the 2026-11-07 layout / viewport batch.
//!
//! Each selector's grammar lives in
//! [`super::commands::layout`](super::commands); this module only dispatches the
//! parsed [`cad_app::LayoutPayload`] to its [`CommandId`]. The selector name,
//! never the typed value, is echoed in the command log, exactly like the other
//! typed-argument paths.
//!
//! A bare selector (`LAYOUT`, `VPORTS`, …) is admitted by name and refused by
//! the application for a missing payload; an argument outside the grammar is
//! refused here with the parser's stable sentence and changes nothing.

use cad_app::{CommandId, CommandPayload};

use super::commands::{
    parse_layout_argument, parse_layoutmerge_argument, parse_layoutwizard_argument,
    parse_layvpi_argument, parse_vpclip_argument, parse_vports_argument, parse_vpscale_argument,
    LAYOUTMERGE_SELECTOR, LAYOUTWIZARD_SELECTOR, LAYOUT_SELECTOR, LAYVPI_SELECTOR, VPCLIP_SELECTOR,
    VPORTS_SELECTOR, VPSCALE_SELECTOR,
};
use super::CadSession;

impl CadSession {
    /// Dispatch one typed layout/viewport argument line.
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
}
