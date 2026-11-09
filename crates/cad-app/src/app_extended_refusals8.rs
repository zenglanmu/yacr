//! Wave 8 **named refusals** over the UCS, tablet, window and menu surfaces.
//!
//! Eight AutoCAD 2026 names describe features this build has no model or API
//! for: user coordinate systems (`UCS`, `UCSMAN`), a tablet/digitizer
//! (`TABLET`, `REINIT`), host-owned application window chrome and layout
//! (`TASKBAR`, `SYSWINDOWS`, `RIBBON`), and a menu-file model (`MENU`).
//!
//! Each name is admitted by the vocabulary and then refused with
//! `CadError::Unsupported` carrying a **stable** reason string. A refusal writes
//! nothing, bumps no revision and records no `cad-history` entry, so every one
//! of these stays Viewer-legal. A caller payload is refused first with
//! `require_none_payload`, so a malformed request never reaches the
//! feature-specific reason.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 8 command with its stable reason string.
    pub(crate) fn extended_refusal8(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        require_none_payload(command, name)?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

/// The stable refusal reason for one Wave 8 command.
///
/// These strings are contract-tested and published in
/// `docs/autocad-2026-command-coverage.md` and `docs/compatibility.md`; changing
/// one is a behaviour change.
fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::Ucs => "ucs.unsupported: this build has no UCS model",
        ExtendedCommandId::UcsMan => "ucsman.unsupported: this build has no UCS model",
        ExtendedCommandId::Tablet => "tablet.unsupported: no tablet/digitizer model",
        ExtendedCommandId::TaskBar => {
            "taskbar.unsupported: application window chrome is host-owned"
        }
        ExtendedCommandId::Reinit => "reinit.unsupported: no device/digitizer initialization model",
        ExtendedCommandId::SysWindows => {
            "syswindows.unsupported: application window layout is host-owned"
        }
        ExtendedCommandId::Menu => "menu.unsupported: no menu-file model; the UI is fixed",
        ExtendedCommandId::Ribbon => "ribbon.unsupported: ribbon visibility is host-owned chrome",
        other => {
            return format!(
                "{}: this command is not a Wave 8 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
