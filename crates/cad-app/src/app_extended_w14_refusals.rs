//! Wave 14 **named refusals**.
//!
//! Every name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a stable reason. Nothing is written, no
//! revision is bumped and no `cad-history` entry is recorded; all are
//! Viewer-legal.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 14 command with its stable reason string.
    pub(crate) fn extended_w14_refusals(
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
        ExtendedCommandId::Box => "box.unsupported: 3D solid primitives are not implemented",
        ExtendedCommandId::Cone => "cone.unsupported: 3D solid primitives are not implemented",
        ExtendedCommandId::Cylinder => {
            "cylinder.unsupported: 3D solid primitives are not implemented"
        }
        ExtendedCommandId::Sphere => "sphere.unsupported: 3D solid primitives are not implemented",
        ExtendedCommandId::Torus => "torus.unsupported: 3D solid primitives are not implemented",
        ExtendedCommandId::Wedge => "wedge.unsupported: 3D solid primitives are not implemented",
        ExtendedCommandId::Pyramid => {
            "pyramid.unsupported: 3D solid primitives are not implemented"
        }
        ExtendedCommandId::PolySolid => {
            "polysolid.unsupported: 3D solid primitives are not implemented"
        }
        ExtendedCommandId::Extrude => "extrude.unsupported: 3D extrusion is not implemented",
        ExtendedCommandId::Revolve => "revolve.unsupported: 3D revolution is not implemented",
        ExtendedCommandId::Sweep => "sweep.unsupported: 3D sweep is not implemented",
        ExtendedCommandId::Loft => "loft.unsupported: 3D loft is not implemented",
        ExtendedCommandId::Intersect => "intersect.unsupported: no 3D boolean model",
        ExtendedCommandId::Union => "union.unsupported: no 3D boolean model",
        ExtendedCommandId::Subtract => "subtract.unsupported: no 3D boolean model",
        ExtendedCommandId::Slice => "slice.unsupported: no 3D slicing model",
        ExtendedCommandId::Thicken => "thicken.unsupported: no surface-to-solid model",
        ExtendedCommandId::PressPull => "presspull.unsupported: no press/pull model",
        ExtendedCommandId::Imprint => "imprint.unsupported: no 3D imprint model",
        ExtendedCommandId::OffsetEdge => "offsetedge.unsupported: no 3D face/edge offset model",
        ExtendedCommandId::ChamferEdge => "chamferedge.unsupported: no 3D edge chamfer model",
        ExtendedCommandId::FilletEdge => "filletedge.unsupported: no 3D edge fillet model",
        ExtendedCommandId::Edge => "edge.unsupported: no 3D face-edge visibility model",
        ExtendedCommandId::Mirror3D => "mirror3d.unsupported: no 3D mirror-plane model",
        ExtendedCommandId::Rotate3D => "rotate3d.unsupported: no 3D rotate-axis model",
        other => {
            return format!(
                "{}: this command is not a Wave 14 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
