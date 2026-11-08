//! Wave 3 **named refusals** for the extended command batch.
//!
//! Twenty-two AutoCAD 2026 names describe features this build has no model or
//! API for: jogged dimensions, inspection data, annotative objects, QLEADER
//! annotation attachments, associative-array editing, the block editor,
//! dynamic-block grips/parameters and geometric constraints.
//!
//! Each name is admitted by the vocabulary and then refused with
//! `CadError::Unsupported` carrying a **stable** reason string (the same shape
//! `LAYOUTMERGE`/`DIMBREAK` use). That is a legitimate partial implementation:
//! the command path exists and answers honestly, but it must never be documented
//! as an implemented feature. A refusal writes nothing, bumps no revision and
//! records no `cad-history` entry, so every one of these is Viewer-legal.
//!
//! A caller payload is refused first with `require_none_payload`, so a malformed
//! request never reaches the feature-specific reason.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 3 command with its stable reason string.
    pub(crate) fn extended_refusal3(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        require_none_payload(command, name)?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

/// The stable refusal reason for one Wave 3 command.
///
/// These strings are contract-tested and published in
/// `docs/autocad-2026-command-coverage.md`; changing one is a behaviour change.
fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::DimJogged => {
            "dimjogged.unsupported: the dimension domain stores no jog field"
        }
        ExtendedCommandId::DimInspect => {
            "diminspect.unsupported: the dimension domain stores no inspection data"
        }
        ExtendedCommandId::DashDimInspect => {
            "-diminspect.unsupported: the dimension domain stores no inspection data"
        }
        ExtendedCommandId::AnnoReset => {
            "annoreset.unsupported: this build has no annotative-object model"
        }
        ExtendedCommandId::AnnoUpdate => {
            "annoupdate.unsupported: this build has no annotative-object model"
        }
        ExtendedCommandId::ObjectScale => {
            "objectscale.unsupported: this build has no annotative-object model"
        }
        ExtendedCommandId::DashObjectScale => {
            "-objectscale.unsupported: this build has no annotative-object model"
        }
        ExtendedCommandId::QlAttach => {
            "qlattach.unsupported: this build has no QLEADER annotation-attachment model"
        }
        ExtendedCommandId::QlAttachSet => {
            "qlattachset.unsupported: this build has no QLEADER annotation-attachment model"
        }
        ExtendedCommandId::QlDetachSet => {
            "qldetchset.unsupported: this build has no QLEADER annotation-attachment model"
        }
        ExtendedCommandId::ArrayClose => {
            "arrayclose.unsupported: this build has no associative-array edit state"
        }
        ExtendedCommandId::ArrayEdit => {
            "arrayedit.unsupported: this build has no associative-array edit state"
        }
        ExtendedCommandId::BlockEditor => "bedit.unsupported: this build has no block editor",
        ExtendedCommandId::DashBlockEditor => "-bedit.unsupported: this build has no block editor",
        ExtendedCommandId::BlockEditorClose => "bclose.unsupported: this build has no block editor",
        ExtendedCommandId::BCycleOrder => {
            "bcycleorder.unsupported: this build has no dynamic-block grip model"
        }
        ExtendedCommandId::BGripSet => {
            "bgripset.unsupported: this build has no dynamic-block parameter model"
        }
        ExtendedCommandId::BEditorSettings => {
            "besettings.unsupported: this build has no block editor"
        }
        ExtendedCommandId::BTestBlock => "btestblock.unsupported: this build has no block editor",
        ExtendedCommandId::LayerPalette => {
            "layerpalette.unsupported: there is no separate layer palette; use LAYER"
        }
        ExtendedCommandId::ConstraintBar => {
            "constraintbar.unsupported: this build has no constraint model"
        }
        ExtendedCommandId::ConstraintSettings => {
            "constraintsettings.unsupported: this build has no constraint model"
        }
        other => {
            return format!(
                "{}: this command is not a Wave 3 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
