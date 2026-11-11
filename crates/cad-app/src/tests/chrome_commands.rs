//! Chrome-local vocabulary boundary contracts.
//!
//! `PMTOGGLE` is a chrome-owned palette visibility toggle: the React shell owns
//! the palette-chrome state (`HIDEPALETTES`/`SHOWPALETTES` set it explicitly),
//! and `cad-app` must **not** grow a second router selector or a second
//! authority for it. This test pins that boundary: the extended selector table
//! contains no `PMTOGGLE`, and the command has no payload path here.

use super::*;

#[test]
fn pmtoggle_is_chrome_local_and_has_no_router_selector() {
    assert!(
        !ExtendedCommandId::ALL
            .iter()
            .any(|ext| ext.selector() == "PMTOGGLE"),
        "PMTOGGLE is answered by the chrome, not by the CAD router"
    );
    assert!(
        !crate::command_aliases::COMMAND_ALIASES
            .iter()
            .any(|(alias, target)| *alias == "PMTOGGLE" || *target == "PMTOGGLE"),
        "PMTOGGLE is not an alias of a router command"
    );
}
