//! Wave 11 **named refusals** for the 3D/mesh/surface surface.
//!
//! 3D is deliberately lower priority than 2D in this build. These twenty-five
//! AutoCAD 2026 names describe a 3D navigation, gizmo, mesh, 3DFACE, clipping or
//! print/export model that this build does not have. Each is admitted by the
//! vocabulary and refused with `CadError::Unsupported` carrying a **stable**
//! reason; nothing is written, no revision is bumped and no `cad-history` entry
//! is recorded, so every name stays Viewer-legal.
//!
//! A caller payload is refused first with `require_none_payload`, so a malformed
//! request never reaches the feature-specific reason.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 11 command with its stable reason string.
    pub(crate) fn extended_w11_refusals(
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
        ExtendedCommandId::ThreeDAlign => {
            "3dalign.unsupported: 3D alignment is not modeled; use the 2D ALIGN path"
        }
        ExtendedCommandId::ThreeDArray => {
            "3darray.unsupported: 3D arrays are not modeled; use ARRAY/ARRAYRECT/ARRAYPOLAR"
        }
        ExtendedCommandId::ThreeDClip => "3dclip.unsupported: no 3D clipping-plane model",
        ExtendedCommandId::ThreeDCorbit => {
            "3dcorbit.unsupported: no continuous 3D orbit controller"
        }
        ExtendedCommandId::ThreeDDistance => {
            "3ddistance.unsupported: no interactive 3D distance navigation"
        }
        ExtendedCommandId::ThreeDDwf => {
            "3ddwf.unsupported: DWF/DWFx export is not implemented; use PLOT for SVG/PDF"
        }
        ExtendedCommandId::ThreeDEditBar => "3deditbar.unsupported: no NURBS/spline edit bar",
        ExtendedCommandId::ThreeDFace => "3dface.unsupported: no 3DFACE entity model",
        ExtendedCommandId::ThreeDFly => "3dfly.unsupported: no 3D fly navigation",
        ExtendedCommandId::ThreeDForbit => "3dforbit.unsupported: no free 3D orbit controller",
        ExtendedCommandId::ThreeDMesh => "3dmesh.unsupported: no free-form mesh creation model",
        ExtendedCommandId::ThreeDMove => "3dmove.unsupported: no 3D move gizmo",
        ExtendedCommandId::ThreeDOrbit => {
            "3dorbit.unsupported: no interactive 3D orbit controller; use ORBIT"
        }
        ExtendedCommandId::ThreeDOrbitCtr => "3dorbitctr.unsupported: no 3D orbit-center model",
        ExtendedCommandId::Dash3DOsnap => "-3dosnap.unsupported: no 3D object-snap model",
        ExtendedCommandId::ThreeDObjectSnap => "3dosnap.unsupported: no 3D object-snap model",
        ExtendedCommandId::ThreeDPan => "3dpan.unsupported: no 3D pan navigation",
        ExtendedCommandId::ThreeDPoly => "3dpoly.unsupported: no 3D polyline entity model",
        ExtendedCommandId::ThreeDPrint => "3dprint.unsupported: no 3D print model",
        ExtendedCommandId::ThreeDPrintService => {
            "3dprintservice.unsupported: no STL export or 3D print service"
        }
        ExtendedCommandId::ThreeDRotate => "3drotate.unsupported: no 3D rotate gizmo",
        ExtendedCommandId::ThreeDScale => "3dscale.unsupported: no 3D scale gizmo",
        ExtendedCommandId::ThreeDSwivel => "3dswivel.unsupported: no 3D swivel navigation",
        ExtendedCommandId::ThreeDWalk => "3dwalk.unsupported: no 3D walk navigation",
        ExtendedCommandId::ThreeDZoom => "3dzoom.unsupported: no perspective 3D zoom navigation",
        other => {
            return format!(
                "{}: this command is not a Wave 11 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
