//! Wave 14 **named refusals**.
//!
//! Every name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a stable reason. Nothing is written, no
//! revision is bumped and no `cad-history` entry is recorded; all are
//! Viewer-legal.
//!
//! ADR-0036 phase C1 graduated the seven primitive families
//! (`BOX`/`WEDGE`/`PYRAMID`/`CYLINDER`/`SPHERE`/`CONE`/`TORUS`) out of this
//! list: they are real body creators now (`app_drawing::solids`) and are
//! dispatched before this refusal arm, so they no longer appear in `reason`.
//! Phase C2 did the same for `UNION`/`SUBTRACT`/`INTERSECT`: they start a
//! background boolean job (`tasks::solid`) and are dispatched ahead of this arm.
//! Everything else here is still a refusal.

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
        ExtendedCommandId::PolySolid => {
            "polysolid.unsupported: 3D solid primitives are not implemented"
        }
        ExtendedCommandId::Extrude => "extrude.unsupported: 3D extrusion is not implemented",
        ExtendedCommandId::Revolve => "revolve.unsupported: 3D revolution is not implemented",
        ExtendedCommandId::Sweep => "sweep.unsupported: 3D sweep is not implemented",
        ExtendedCommandId::Loft => "loft.unsupported: 3D loft is not implemented",
        ExtendedCommandId::Slice => "slice.unsupported: no 3D slicing model",
        ExtendedCommandId::Thicken => "thicken.unsupported: no surface-to-solid model",
        ExtendedCommandId::PressPull => "presspull.unsupported: no press/pull model",
        ExtendedCommandId::Imprint => "imprint.unsupported: no 3D imprint model",
        ExtendedCommandId::OffsetEdge => "offsetedge.unsupported: no 3D face/edge offset model",
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
