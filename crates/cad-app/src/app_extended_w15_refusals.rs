//! Wave 15 **named refusals**.
//!
//! Every name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a stable reason. Nothing is written, no
//! revision is bumped and no `cad-history` entry is recorded; all are
//! Viewer-legal.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 15 command with its stable reason string.
    pub(crate) fn extended_w15_refusals(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, ext.selector())?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::SurfBlend => {
            "surfblend.unsupported: surface blending is not implemented"
        }
        ExtendedCommandId::SurfExtend => {
            "surfextend.unsupported: surface extension is not implemented"
        }
        ExtendedCommandId::SurfExtractCurve => {
            "surfextractcurve.unsupported: surface curve extraction is not implemented"
        }
        ExtendedCommandId::SurfFillet => {
            "surffillet.unsupported: surface filleting is not implemented"
        }
        ExtendedCommandId::SurfNetwork => {
            "surfnetwork.unsupported: surface network creation is not implemented"
        }
        ExtendedCommandId::SurfOffset => {
            "surfoffset.unsupported: surface offset is not implemented"
        }
        ExtendedCommandId::SurfPatch => {
            "surfpatch.unsupported: surface patching is not implemented"
        }
        ExtendedCommandId::SurfSculpt => {
            "surfsculpt.unsupported: surface sculpting is not implemented"
        }
        ExtendedCommandId::SurfTrim => "surftrim.unsupported: surface trimming is not implemented",
        ExtendedCommandId::SurfUntrim => {
            "surfuntrim.unsupported: surface untrimming is not implemented"
        }
        ExtendedCommandId::MeshCreate => {
            "mesh.unsupported: free-form mesh creation is not implemented"
        }
        ExtendedCommandId::MeshCap => "meshcap.unsupported: mesh capping is not implemented",
        ExtendedCommandId::MeshCollapse => {
            "meshcollapse.unsupported: mesh collapse is not implemented"
        }
        ExtendedCommandId::MeshCrease => "meshcrease.unsupported: mesh creasing is not implemented",
        ExtendedCommandId::MeshExtrude => {
            "meshextrude.unsupported: mesh extrusion is not implemented"
        }
        ExtendedCommandId::MeshMerge => "meshmerge.unsupported: mesh merge is not implemented",
        ExtendedCommandId::MeshOptions => {
            "meshoptions.unsupported: no mesh tessellation options model"
        }
        ExtendedCommandId::MeshPrimitiveOptions => {
            "meshprimitiveoptions.unsupported: no mesh primitive options model"
        }
        ExtendedCommandId::MeshRefine => {
            "meshrefine.unsupported: mesh refinement is not implemented"
        }
        ExtendedCommandId::MeshSmooth => {
            "meshsmooth.unsupported: mesh smoothing conversion is not implemented"
        }
        ExtendedCommandId::MeshSmoothLess => {
            "meshsmoothless.unsupported: no mesh smoothness level model"
        }
        ExtendedCommandId::MeshSmoothMore => {
            "meshsmoothmore.unsupported: no mesh smoothness level model"
        }
        ExtendedCommandId::MeshSpin => "meshspin.unsupported: mesh edge spin is not implemented",
        ExtendedCommandId::MeshSplit => "meshsplit.unsupported: mesh face split is not implemented",
        ExtendedCommandId::MeshUncrease => {
            "meshuncrease.unsupported: mesh uncreasing is not implemented"
        }
        other => {
            return format!(
                "{}: this command is not a Wave 15 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
