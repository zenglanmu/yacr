//! Wave 5 **named refusals** over the attribute and xref surfaces.
//!
//! Thirteen AutoCAD 2026 names remain after the block-attribute command family
//! graduated: file-based attribute import (`ATTIN`), export (`ATTOUT`) and
//! extraction (`ATTEXT`/`-ATTEXT`/`EATTEXT`/`-EATTEXT`), entity XDATA, xref
//! bind/insert, external-reference/underlay attachment and an xref editing
//! session. Those file lanes are a separate implementation slice; the in-drawing
//! definition/value commands now live in `app_drawing::attribute_commands`.
//!
//! (`CLASSICXREF` graduated out of this batch: it is a real read-only xref
//! report plus the chrome-local classic panel.)
//!
//! Each name is admitted by the vocabulary and then refused with
//! `CadError::Unsupported` carrying a **stable** reason string. A refusal writes
//! nothing, bumps no revision and records no `cad-history` entry, so every one of
//! these stays Viewer-legal. A caller payload is refused first with
//! `require_none_payload`, so a malformed request never reaches the
//! feature-specific reason.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 5 command with its stable reason string.
    pub(crate) fn extended_refusal5(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        require_none_payload(command, name)?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

/// The stable refusal reason for one Wave 5 command.
///
/// These strings are contract-tested and published in
/// `docs/autocad-2026-command-coverage.md` and `docs/compatibility.md`; changing
/// one is a behaviour change.
fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::AttIn => {
            "attin.unsupported: importing attributes from an external file is not implemented"
        }
        ExtendedCommandId::AttOut => {
            "attout.unsupported: exporting attributes to an external file is not implemented"
        }
        ExtendedCommandId::AttExt => {
            "attext.unsupported: attribute extraction to a template/file is not implemented"
        }
        ExtendedCommandId::DashAttExt => {
            "-attext.unsupported: attribute extraction to a template/file is not implemented"
        }
        ExtendedCommandId::EAttExt => {
            "eatttext.unsupported: attribute extraction to a template/file is not implemented"
        }
        ExtendedCommandId::DashEAttExt => {
            "-eatttext.unsupported: attribute extraction to a template/file is not implemented"
        }
        ExtendedCommandId::XData => "xdata.unsupported: this database does not retain entity XDATA",
        ExtendedCommandId::XBind => "xbind.unsupported: this build has no xref bind/insert model",
        ExtendedCommandId::DashXBind => {
            "-xbind.unsupported: this build has no xref bind/insert model"
        }
        ExtendedCommandId::XAttach => {
            "xattach.unsupported: this build does not attach external drawings as xrefs"
        }
        ExtendedCommandId::Attach => {
            "attach.unsupported: this build does not attach external references or underlays"
        }
        ExtendedCommandId::DashAttach => {
            "-attach.unsupported: this build does not attach external references or underlays"
        }
        ExtendedCommandId::XOpen => "xopen.unsupported: this build has no xref editing session",
        other => {
            return format!(
                "{}: this command is not a Wave 5 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
