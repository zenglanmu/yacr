//! One-pick solid/mesh face-edit descriptors (CHAMFEREDGE / PRESSPULL).

use cad_domain::{CadError, CadResult};

/// The command, refusal code and target resolver of a one-pick solid face edit.
pub(crate) struct SolidFaceTool {
    pub(crate) command: cad_app::CommandId,
    pub(crate) code: &'static str,
    pub(crate) target: fn(
        &cad_app::Application,
        cad_domain::DocumentId,
        &[cad_domain::SelectionRef],
    ) -> cad_domain::CadResult<cad_app::SolidEditTarget>,
}

/// Map a face-edit capture kind to its command, code prefix and target resolver.
pub(crate) fn solid_face_tool(kind: cad_app::DrawToolKind) -> CadResult<SolidFaceTool> {
    match kind {
        cad_app::DrawToolKind::ChamferEdge => Ok(SolidFaceTool {
            command: cad_app::CommandId::Extended(cad_app::ExtendedCommandId::ChamferEdge),
            code: "chamferedge",
            target: cad_app::chamfer_edge_target,
        }),
        cad_app::DrawToolKind::PressPull => Ok(SolidFaceTool {
            command: cad_app::CommandId::Extended(cad_app::ExtendedCommandId::PressPull),
            code: "presspull",
            target: cad_app::press_pull_target,
        }),
        other => Err(CadError::Invariant(format!(
            "solid_face_tool: {} is not a solid face edit",
            other.key()
        ))),
    }
}
