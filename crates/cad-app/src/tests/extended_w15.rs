//! Wave 15 named-refusal contracts.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a stable reason; nothing is written, no
//! revision is bumped and no history entry appears. All are Viewer-legal.

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 25] = [
    (
        ExtendedCommandId::SurfBlend,
        "surfblend.unsupported: surface blending is not implemented",
    ),
    (
        ExtendedCommandId::SurfExtend,
        "surfextend.unsupported: surface extension is not implemented",
    ),
    (
        ExtendedCommandId::SurfExtractCurve,
        "surfextractcurve.unsupported: surface curve extraction is not implemented",
    ),
    (
        ExtendedCommandId::SurfFillet,
        "surffillet.unsupported: surface filleting is not implemented",
    ),
    (
        ExtendedCommandId::SurfNetwork,
        "surfnetwork.unsupported: surface network creation is not implemented",
    ),
    (
        ExtendedCommandId::SurfOffset,
        "surfoffset.unsupported: surface offset is not implemented",
    ),
    (
        ExtendedCommandId::SurfPatch,
        "surfpatch.unsupported: surface patching is not implemented",
    ),
    (
        ExtendedCommandId::SurfSculpt,
        "surfsculpt.unsupported: surface sculpting is not implemented",
    ),
    (
        ExtendedCommandId::SurfTrim,
        "surftrim.unsupported: surface trimming is not implemented",
    ),
    (
        ExtendedCommandId::SurfUntrim,
        "surfuntrim.unsupported: surface untrimming is not implemented",
    ),
    (
        ExtendedCommandId::MeshCreate,
        "mesh.unsupported: free-form mesh creation is not implemented",
    ),
    (
        ExtendedCommandId::MeshCap,
        "meshcap.unsupported: mesh capping is not implemented",
    ),
    (
        ExtendedCommandId::MeshCollapse,
        "meshcollapse.unsupported: mesh collapse is not implemented",
    ),
    (
        ExtendedCommandId::MeshCrease,
        "meshcrease.unsupported: mesh creasing is not implemented",
    ),
    (
        ExtendedCommandId::MeshExtrude,
        "meshextrude.unsupported: mesh extrusion is not implemented",
    ),
    (
        ExtendedCommandId::MeshMerge,
        "meshmerge.unsupported: mesh merge is not implemented",
    ),
    (
        ExtendedCommandId::MeshOptions,
        "meshoptions.unsupported: no mesh tessellation options model",
    ),
    (
        ExtendedCommandId::MeshPrimitiveOptions,
        "meshprimitiveoptions.unsupported: no mesh primitive options model",
    ),
    (
        ExtendedCommandId::MeshRefine,
        "meshrefine.unsupported: mesh refinement is not implemented",
    ),
    (
        ExtendedCommandId::MeshSmooth,
        "meshsmooth.unsupported: mesh smoothing conversion is not implemented",
    ),
    (
        ExtendedCommandId::MeshSmoothLess,
        "meshsmoothless.unsupported: no mesh smoothness level model",
    ),
    (
        ExtendedCommandId::MeshSmoothMore,
        "meshsmoothmore.unsupported: no mesh smoothness level model",
    ),
    (
        ExtendedCommandId::MeshSpin,
        "meshspin.unsupported: mesh edge spin is not implemented",
    ),
    (
        ExtendedCommandId::MeshSplit,
        "meshsplit.unsupported: mesh face split is not implemented",
    ),
    (
        ExtendedCommandId::MeshUncrease,
        "meshuncrease.unsupported: mesh uncreasing is not implemented",
    ),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave15_refusal_answers_with_its_exact_stable_reason() {
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
fn every_wave15_refusal_stays_viewer_legal() {
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
