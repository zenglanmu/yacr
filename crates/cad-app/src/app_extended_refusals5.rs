//! Wave 5 **named refusals** over the attribute and xref surfaces.
//!
//! Twenty-three AutoCAD 2026 names describe features this build has no model or
//! API for: block attributes (definition, editing, import/export, extraction,
//! redefinition, synchronisation, global edit), entity XDATA, xref bind/insert,
//! external-reference/underlay attachment, an xref editing session and a
//! separate classic xref palette.
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
        ExtendedCommandId::AttDef => "attdef.unsupported: this build has no block-attribute model",
        ExtendedCommandId::DashAttDef => {
            "-attdef.unsupported: this build has no block-attribute model"
        }
        ExtendedCommandId::AttEdit => {
            "attedit.unsupported: this build has no block-attribute model"
        }
        ExtendedCommandId::DashAttEdit => {
            "-attedit.unsupported: this build has no block-attribute model"
        }
        ExtendedCommandId::AttIpEdit => {
            "attipedit.unsupported: this build has no block-attribute model"
        }
        ExtendedCommandId::AttIn => "attin.unsupported: this build has no block-attribute model",
        ExtendedCommandId::AttOut => "attout.unsupported: this build has no block-attribute model",
        ExtendedCommandId::AttExt => "attext.unsupported: this build has no block-attribute model",
        ExtendedCommandId::DashAttExt => {
            "-attext.unsupported: this build has no block-attribute model"
        }
        ExtendedCommandId::EAttEdit => {
            "eattedit.unsupported: this build has no block-attribute model"
        }
        ExtendedCommandId::EAttExt => {
            "eatttext.unsupported: this build has no block-attribute model"
        }
        ExtendedCommandId::DashEAttExt => {
            "-eatttext.unsupported: this build has no block-attribute model"
        }
        ExtendedCommandId::AttRedef => {
            "attredef.unsupported: this build has no block-attribute model"
        }
        ExtendedCommandId::AttSync => {
            "attsync.unsupported: this build has no block-attribute model"
        }
        ExtendedCommandId::Gatte => "gatte.unsupported: this build has no block-attribute model",
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
        ExtendedCommandId::ClassicXRef => {
            "classicxref.unsupported: there is no separate xref palette; use the RESOURCES panel"
        }
        other => {
            return format!(
                "{}: this command is not a Wave 5 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
