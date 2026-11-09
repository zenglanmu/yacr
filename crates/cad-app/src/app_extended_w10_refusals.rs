//! Wave 10 **named refusals** over palette, start-tab, help and image surfaces.
//!
//! Ten names describe features with no model or API in this build: the help
//! system (`HELP`), the classic insert/group dialogs (`CLASSICINSERT`,
//! `CLASSICGROUP`), file-tab and palette chrome (`FILETAB`, `FILETABCLOSE`,
//! `HIDEPALETTES`, `SHOWPALETTES`, `RIBBONCLOSE`), the start tab (`GOTOSTART`)
//! and image transparency (`TRANSPARENCY`).
//!
//! Each is refused with `CadError::Unsupported` carrying a stable reason. A
//! refusal writes nothing, bumps no revision and records no `cad-history` entry,
//! so all ten stay Viewer-legal.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 10 command with its stable reason string.
    pub(crate) fn extended_w10_refusals(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, ext.selector())?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::Help => {
            "help.unsupported: the help system is host-owned and not implemented"
        }
        ExtendedCommandId::ClassicInsert => {
            "classicinsert.unsupported: no classic insert dialog; use INSERT"
        }
        ExtendedCommandId::ClassicGroup => {
            "classicgroup.unsupported: no classic group dialog; use GROUP"
        }
        ExtendedCommandId::FileTab => "filetab.unsupported: file-tab chrome is host-owned",
        ExtendedCommandId::FileTabClose => {
            "filetabclose.unsupported: file-tab chrome is host-owned"
        }
        ExtendedCommandId::HidePalettes => "hidepalettes.unsupported: palette chrome is host-owned",
        ExtendedCommandId::ShowPalettes => "showpalettes.unsupported: palette chrome is host-owned",
        ExtendedCommandId::RibbonClose => {
            "ribbonclose.unsupported: ribbon chrome is host-owned; use RIBBON"
        }
        ExtendedCommandId::GoToStart => {
            "gotostart.unsupported: switching to the start tab is host-owned"
        }
        ExtendedCommandId::Transparency => {
            "transparency.unsupported: image transparency is not modeled"
        }
        other => {
            return format!(
                "{}: this command is not a Wave 10 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
