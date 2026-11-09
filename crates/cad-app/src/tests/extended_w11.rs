//! Wave 11 named-refusal contracts: the 3D/mesh/surface surface.
//!
//! 3D is deliberately lower priority. Every name is admitted by the vocabulary
//! and refused with `CadError::Unsupported` carrying a stable reason; nothing is
//! written, no revision is bumped and no history entry appears. All are
//! Viewer-legal.

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 25] = [
    (
        ExtendedCommandId::ThreeDAlign,
        "3dalign.unsupported: 3D alignment is not modeled; use the 2D ALIGN path",
    ),
    (
        ExtendedCommandId::ThreeDArray,
        "3darray.unsupported: 3D arrays are not modeled; use ARRAY/ARRAYRECT/ARRAYPOLAR",
    ),
    (
        ExtendedCommandId::ThreeDClip,
        "3dclip.unsupported: no 3D clipping-plane model",
    ),
    (
        ExtendedCommandId::ThreeDCorbit,
        "3dcorbit.unsupported: no continuous 3D orbit controller",
    ),
    (
        ExtendedCommandId::ThreeDDistance,
        "3ddistance.unsupported: no interactive 3D distance navigation",
    ),
    (
        ExtendedCommandId::ThreeDDwf,
        "3ddwf.unsupported: DWF/DWFx export is not implemented; use PLOT for SVG/PDF",
    ),
    (
        ExtendedCommandId::ThreeDEditBar,
        "3deditbar.unsupported: no NURBS/spline edit bar",
    ),
    (
        ExtendedCommandId::ThreeDFace,
        "3dface.unsupported: no 3DFACE entity model",
    ),
    (
        ExtendedCommandId::ThreeDFly,
        "3dfly.unsupported: no 3D fly navigation",
    ),
    (
        ExtendedCommandId::ThreeDForbit,
        "3dforbit.unsupported: no free 3D orbit controller",
    ),
    (
        ExtendedCommandId::ThreeDMesh,
        "3dmesh.unsupported: no free-form mesh creation model",
    ),
    (
        ExtendedCommandId::ThreeDMove,
        "3dmove.unsupported: no 3D move gizmo",
    ),
    (
        ExtendedCommandId::ThreeDOrbit,
        "3dorbit.unsupported: no interactive 3D orbit controller; use ORBIT",
    ),
    (
        ExtendedCommandId::ThreeDOrbitCtr,
        "3dorbitctr.unsupported: no 3D orbit-center model",
    ),
    (
        ExtendedCommandId::Dash3DOsnap,
        "-3dosnap.unsupported: no 3D object-snap model",
    ),
    (
        ExtendedCommandId::ThreeDObjectSnap,
        "3dosnap.unsupported: no 3D object-snap model",
    ),
    (
        ExtendedCommandId::ThreeDPan,
        "3dpan.unsupported: no 3D pan navigation",
    ),
    (
        ExtendedCommandId::ThreeDPoly,
        "3dpoly.unsupported: no 3D polyline entity model",
    ),
    (
        ExtendedCommandId::ThreeDPrint,
        "3dprint.unsupported: no 3D print model",
    ),
    (
        ExtendedCommandId::ThreeDPrintService,
        "3dprintservice.unsupported: no STL export or 3D print service",
    ),
    (
        ExtendedCommandId::ThreeDRotate,
        "3drotate.unsupported: no 3D rotate gizmo",
    ),
    (
        ExtendedCommandId::ThreeDScale,
        "3dscale.unsupported: no 3D scale gizmo",
    ),
    (
        ExtendedCommandId::ThreeDSwivel,
        "3dswivel.unsupported: no 3D swivel navigation",
    ),
    (
        ExtendedCommandId::ThreeDWalk,
        "3dwalk.unsupported: no 3D walk navigation",
    ),
    (
        ExtendedCommandId::ThreeDZoom,
        "3dzoom.unsupported: no perspective 3D zoom navigation",
    ),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave11_refusal_answers_with_its_exact_stable_reason() {
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
        assert_eq!(revision(&app), before, "{ext:?} writes nothing");
        assert!(
            !app.can_undo(&DocumentId(1)),
            "{ext:?} records no undo step"
        );
        assert!(!ext.requires_work_mode(), "{ext:?} stays Viewer-legal");
    }
}

#[test]
fn every_wave11_refusal_stays_viewer_legal() {
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
