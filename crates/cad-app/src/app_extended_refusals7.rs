//! Wave 7 **named refusals** over the dynamic-block and constraint surfaces.
//!
//! Twenty-eight AutoCAD 2026 names describe features this build has no model or
//! API for: dynamic-block actions (`BACTION`, `BACTIONBAR`, `BACTIONSET`,
//! `BACTIONTOOL`, `BASSOCIATE`), the block authoring palette (`BAUTHORPALETTE`,
//! `BAUTHORPALETTECLOSE`), construction geometry (`BCONSTRUCTION`), parameters
//! (`BCPARAMETER`, `-BCPARAMETER`, `BPARAMETER`, and `RESETBLOCK`'s reset API),
//! lookup tables (`BLOOKUPTABLE`, `BTABLE`), per-visibility-state hide/show
//! (`BVHIDE`, `BVSHOW`), and geometric/dimensional constraints
//! (`AUTOCONSTRAIN`, `DELCONSTRAINT`, `DIMCONSTRAINT`, `GEOMCONSTRAINT`, and the
//! `DC*` family).
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
    /// Refuse one Wave 7 command with its stable reason string.
    pub(crate) fn extended_refusal7(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        require_none_payload(command, name)?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

/// The stable refusal reason for one Wave 7 command.
///
/// These strings are contract-tested and published in
/// `docs/autocad-2026-command-coverage.md` and `docs/compatibility.md`; changing
/// one is a behaviour change.
fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::BAction => {
            "baction.unsupported: this build has no dynamic-block action model"
        }
        ExtendedCommandId::BActionBar => {
            "bactionbar.unsupported: this build has no dynamic-block action model"
        }
        ExtendedCommandId::BActionSet => {
            "bactionset.unsupported: this build has no dynamic-block action model"
        }
        ExtendedCommandId::BActionTool => {
            "bactiontool.unsupported: this build has no dynamic-block action model"
        }
        ExtendedCommandId::BAssociate => {
            "bassociate.unsupported: this build has no dynamic-block action model"
        }
        ExtendedCommandId::BAuthorPalette => {
            "bauthorpalette.unsupported: this build has no block editor"
        }
        ExtendedCommandId::BAuthorPaletteClose => {
            "bauthorpaletteclose.unsupported: this build has no block editor"
        }
        ExtendedCommandId::BConstruction => {
            "bconstruction.unsupported: this build has no dynamic-block construction geometry"
        }
        ExtendedCommandId::BCParameter => {
            "bcparameter.unsupported: this build has no dynamic-block parameter model"
        }
        ExtendedCommandId::DashBCParameter => {
            "-bcparameter.unsupported: this build has no dynamic-block parameter model"
        }
        ExtendedCommandId::BParameter => {
            "bparameter.unsupported: this build has no dynamic-block parameter model"
        }
        ExtendedCommandId::BLookupTable => {
            "blookuptable.unsupported: this build has no dynamic-block lookup table"
        }
        ExtendedCommandId::BTable => {
            "btable.unsupported: this build has no dynamic-block lookup table"
        }
        ExtendedCommandId::BvHide => "bvhide.unsupported: no per-visibility-state entity hide API",
        ExtendedCommandId::BvShow => "bvshow.unsupported: no per-visibility-state entity show API",
        ExtendedCommandId::ResetBlock => {
            "resetblock.unsupported: no dynamic-block parameter reset API"
        }
        ExtendedCommandId::AutoConstrain => {
            "autoconstrain.unsupported: this build has no geometric-constraint model"
        }
        ExtendedCommandId::DelConstraint => {
            "delconstraint.unsupported: this build has no geometric-constraint model"
        }
        ExtendedCommandId::DimConstraint => {
            "dimconstraint.unsupported: this build has no dimensional-constraint model"
        }
        ExtendedCommandId::GeomConstraint => {
            "geomconstraint.unsupported: this build has no geometric-constraint model"
        }
        ExtendedCommandId::DcAligned => {
            "dcaligned.unsupported: this build has no dimensional-constraint model"
        }
        ExtendedCommandId::DcAngular => {
            "dcangular.unsupported: this build has no dimensional-constraint model"
        }
        ExtendedCommandId::DcConvert => {
            "dcconvert.unsupported: this build has no dimensional-constraint model"
        }
        ExtendedCommandId::DcDiameter => {
            "dcdiameter.unsupported: this build has no dimensional-constraint model"
        }
        ExtendedCommandId::DcDisplay => {
            "dcdisplay.unsupported: this build has no dimensional-constraint model"
        }
        ExtendedCommandId::DcForm => {
            "dcform.unsupported: this build has no dimensional-constraint model"
        }
        ExtendedCommandId::DcHorizontal => {
            "dchorizontal.unsupported: this build has no dimensional-constraint model"
        }
        ExtendedCommandId::DcLinear => {
            "dclinear.unsupported: this build has no dimensional-constraint model"
        }
        other => {
            return format!(
                "{}: this command is not a Wave 7 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
