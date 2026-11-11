//! Wave 4 **named refusals** for the extended command batch.
//!
//! Eighteen AutoCAD 2026 names describe features this build has no model or API
//! for: block scaling/insert-array/replacement, NURBS editing, block-attribute
//! bursts, image/xref clipping, Region geometry, per-type add-selected prompts,
//! writing a block to a separate drawing file, a block editor and moving entities
//! between model and paper space.
//!
//! `MINSERT`, `ADDSELECTED` and `MLEDIT`/`-MLEDIT` graduated to real entity
//! writes (2026-11-12); the two-LINE multiline edit lives in
//! [`crate::app_drawing::mledit`] and is no longer part of this refusal table.
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
    /// Refuse one Wave 4 command with its stable reason string.
    pub(crate) fn extended_refusal4(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        require_none_payload(command, name)?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

/// The stable refusal reason for one Wave 4 command.
///
/// These strings are contract-tested and published in
/// `docs/autocad-2026-command-coverage.md`; changing one is a behaviour change.
fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::BScale => {
            "bscale.unsupported: BSCALE needs a factor and an insertion reference; use SCALE on \
             the selected insert"
        }
        ExtendedCommandId::BlockReplace => {
            "blockreplace.unsupported: BLOCKREPLACE needs two block definitions; this build has \
             no block-replace editor"
        }
        ExtendedCommandId::BReplace => {
            "breplace.unsupported: BREPLACE needs two block definitions; this build has no \
             block-replace editor"
        }
        ExtendedCommandId::DashBReplace => {
            "-breplace.unsupported: BREPLACE needs two block definitions; this build has no \
             block-replace editor"
        }
        ExtendedCommandId::SplineEdit => {
            "splinedit.unsupported: this build has no NURBS edit surface"
        }
        ExtendedCommandId::Burst => {
            "burst.unsupported: this build has no block-attribute model to preserve"
        }
        ExtendedCommandId::Clip => "clip.unsupported: this build has no image/xref clipping model",
        ExtendedCommandId::ClipIt => {
            "clipit.unsupported: this build has no image/xref clipping model"
        }
        ExtendedCommandId::Region => "region.unsupported: this build has no Region geometry model",
        ExtendedCommandId::WBlock => {
            "wblock.unsupported: writing a block definition to a separate drawing file is not \
             implemented"
        }
        ExtendedCommandId::DashWBlock => {
            "-wblock.unsupported: writing a block definition to a separate drawing file is not \
             implemented"
        }
        ExtendedCommandId::BSave => "bsave.unsupported: this build has no block editor",
        ExtendedCommandId::BSaveAs => "bsaveas.unsupported: this build has no block editor",
        ExtendedCommandId::ChSpace => {
            "chspace.unsupported: moving entities between model and paper space is not implemented"
        }
        other => {
            return format!(
                "{}: this command is not a Wave 4 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
