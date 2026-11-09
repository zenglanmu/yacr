//! Wave 14 named-refusal contracts.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a stable reason; nothing is written, no
//! revision is bumped and no history entry appears. All are Viewer-legal.

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 25] = [
    (
        ExtendedCommandId::Box,
        "box.unsupported: 3D solid primitives are not implemented",
    ),
    (
        ExtendedCommandId::Cone,
        "cone.unsupported: 3D solid primitives are not implemented",
    ),
    (
        ExtendedCommandId::Cylinder,
        "cylinder.unsupported: 3D solid primitives are not implemented",
    ),
    (
        ExtendedCommandId::Sphere,
        "sphere.unsupported: 3D solid primitives are not implemented",
    ),
    (
        ExtendedCommandId::Torus,
        "torus.unsupported: 3D solid primitives are not implemented",
    ),
    (
        ExtendedCommandId::Wedge,
        "wedge.unsupported: 3D solid primitives are not implemented",
    ),
    (
        ExtendedCommandId::Pyramid,
        "pyramid.unsupported: 3D solid primitives are not implemented",
    ),
    (
        ExtendedCommandId::PolySolid,
        "polysolid.unsupported: 3D solid primitives are not implemented",
    ),
    (
        ExtendedCommandId::Extrude,
        "extrude.unsupported: 3D extrusion is not implemented",
    ),
    (
        ExtendedCommandId::Revolve,
        "revolve.unsupported: 3D revolution is not implemented",
    ),
    (
        ExtendedCommandId::Sweep,
        "sweep.unsupported: 3D sweep is not implemented",
    ),
    (
        ExtendedCommandId::Loft,
        "loft.unsupported: 3D loft is not implemented",
    ),
    (
        ExtendedCommandId::Intersect,
        "intersect.unsupported: no 3D boolean model",
    ),
    (
        ExtendedCommandId::Union,
        "union.unsupported: no 3D boolean model",
    ),
    (
        ExtendedCommandId::Subtract,
        "subtract.unsupported: no 3D boolean model",
    ),
    (
        ExtendedCommandId::Slice,
        "slice.unsupported: no 3D slicing model",
    ),
    (
        ExtendedCommandId::Thicken,
        "thicken.unsupported: no surface-to-solid model",
    ),
    (
        ExtendedCommandId::PressPull,
        "presspull.unsupported: no press/pull model",
    ),
    (
        ExtendedCommandId::Imprint,
        "imprint.unsupported: no 3D imprint model",
    ),
    (
        ExtendedCommandId::OffsetEdge,
        "offsetedge.unsupported: no 3D face/edge offset model",
    ),
    (
        ExtendedCommandId::ChamferEdge,
        "chamferedge.unsupported: no 3D edge chamfer model",
    ),
    (
        ExtendedCommandId::FilletEdge,
        "filletedge.unsupported: no 3D edge fillet model",
    ),
    (
        ExtendedCommandId::Edge,
        "edge.unsupported: no 3D face-edge visibility model",
    ),
    (
        ExtendedCommandId::Mirror3D,
        "mirror3d.unsupported: no 3D mirror-plane model",
    ),
    (
        ExtendedCommandId::Rotate3D,
        "rotate3d.unsupported: no 3D rotate-axis model",
    ),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave14_refusal_answers_with_its_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a refusal never succeeds");
        match error {
            CadError::Unsupported(reason) => assert_eq!(reason, expected),
            other => panic!("{ext:?} must be Unsupported, got {other:?}"),
        }
        assert_eq!(revision(&app), before);
        assert!(!app.can_undo(&DocumentId(1)));
        assert!(!ext.requires_work_mode(), "{ext:?} stays Viewer-legal");
    }
}

#[test]
fn every_wave14_refusal_stays_viewer_legal() {
    for (ext, _) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is legal");
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("the feature is still missing");
        assert!(matches!(error, CadError::Unsupported(_)));
    }
}
