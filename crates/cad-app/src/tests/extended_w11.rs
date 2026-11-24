//! Wave 11 contracts: the 3D/mesh/surface surface.
//!
//! 3D is deliberately lower priority. Twenty-three names are admitted by the
//! vocabulary and refused with `CadError::Unsupported` carrying a stable reason;
//! nothing is written, no revision is bumped and no history entry appears. All
//! are Viewer-legal. `3DORBIT`/`3DPAN` are **not** refusals: they are real
//! aliases of the `ORBIT`/`PAN` camera handlers, pinned by the alias test below.

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 23] = [
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

fn camera(app: &Application) -> Camera {
    app.workspace.viewports[&ViewportId(1)].camera
}

#[test]
fn three_d_orbit_and_three_d_pan_are_aliases_of_orbit_and_pan() {
    // 1. The bare forms refuse exactly like the canonical commands: no payload
    //    message is invented for the alias.
    let (mut canonical, mut canonical_session) = application_with_document();
    let (mut aliased, mut aliased_session) = application_with_document();
    let orbit_error = canonical
        .execute(
            &mut canonical_session,
            command(CommandId::Orbit, CommandPayload::None),
        )
        .unwrap_err();
    let alias_error = aliased
        .execute(
            &mut aliased_session,
            command(
                CommandId::Extended(ExtendedCommandId::ThreeDOrbit),
                CommandPayload::None,
            ),
        )
        .unwrap_err();
    assert_eq!(alias_error, orbit_error, "3DORBIT is ORBIT, not a new path");

    let pan_error = canonical
        .execute(
            &mut canonical_session,
            command(CommandId::Pan, CommandPayload::None),
        )
        .unwrap_err();
    let alias_pan_error = aliased
        .execute(
            &mut aliased_session,
            command(
                CommandId::Extended(ExtendedCommandId::ThreeDPan),
                CommandPayload::None,
            ),
        )
        .unwrap_err();
    assert_eq!(alias_pan_error, pan_error, "3DPAN is PAN, not a new path");

    // 2. The same typed payloads produce the same camera. `Switch2d3d` is the
    //    shared 3D transition, so each app starts from the same observation.
    for (app, session) in [
        (&mut canonical, &mut canonical_session),
        (&mut aliased, &mut aliased_session),
    ] {
        app.execute(
            session,
            command(CommandId::Switch2d3d, CommandPayload::None),
        )
        .expect("a 3D observation");
    }
    canonical
        .execute(
            &mut canonical_session,
            command(
                CommandId::Orbit,
                CommandPayload::Orbit {
                    yaw: 0.3,
                    pitch: 0.2,
                },
            ),
        )
        .expect("ORBIT");
    aliased
        .execute(
            &mut aliased_session,
            command(
                CommandId::Extended(ExtendedCommandId::ThreeDOrbit),
                CommandPayload::Orbit {
                    yaw: 0.3,
                    pitch: 0.2,
                },
            ),
        )
        .expect("3DORBIT");
    assert_eq!(camera(&aliased), camera(&canonical), "same orbit result");

    let delta = Point3 {
        x: 2.0,
        y: -1.0,
        z: 0.0,
    };
    canonical
        .execute(
            &mut canonical_session,
            command(CommandId::Pan, CommandPayload::Points(vec![delta])),
        )
        .expect("PAN");
    aliased
        .execute(
            &mut aliased_session,
            command(
                CommandId::Extended(ExtendedCommandId::ThreeDPan),
                CommandPayload::Points(vec![delta]),
            ),
        )
        .expect("3DPAN");
    assert_eq!(camera(&aliased), camera(&canonical), "same pan result");

    // 3. Both aliases are view-only selectors, so neither becomes Work-only.
    assert!(!ExtendedCommandId::ThreeDOrbit.requires_work_mode());
    assert!(!ExtendedCommandId::ThreeDPan.requires_work_mode());
}
