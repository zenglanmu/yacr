//! Wave 14 command contracts.
//!
//! ADR-0036 phase C1 graduated the seven primitive families
//! (`BOX`/`WEDGE`/`PYRAMID`/`CYLINDER`/`SPHERE`/`CONE`/`TORUS`) from named
//! refusals to real body creators, so they are no longer in this list: they are
//! Work-only drawing writes (their create contracts live in
//! `tests/drawing/solids.rs`) and a carrier without a modeler refuses them with
//! `unsupported-runtime: solid-modeler`.
//!
//! Every **remaining** name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a stable reason; nothing is written, no
//! revision is bumped and no history entry appears. All are Viewer-legal.

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 18] = [
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

/// The seven primitives that became real body creators in phase C1.
const PRIMITIVES: [ExtendedCommandId; 7] = [
    ExtendedCommandId::Box,
    ExtendedCommandId::Wedge,
    ExtendedCommandId::Pyramid,
    ExtendedCommandId::Cylinder,
    ExtendedCommandId::Sphere,
    ExtendedCommandId::Cone,
    ExtendedCommandId::Torus,
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

#[test]
fn the_seven_primitives_are_drawing_writes_not_viewer_refusals() {
    for ext in PRIMITIVES {
        assert!(
            ext.requires_work_mode(),
            "{ext:?} creates a body, so it is Work-only"
        );
    }
}

#[test]
fn a_viewer_cannot_create_a_primitive() {
    for ext in PRIMITIVES {
        let (mut app, mut session) = application_with_document();
        session
            .switch_mode(AppMode::Viewer)
            .expect("Viewer is legal");
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("Viewer is read-only");
        assert_eq!(error, CadError::PermissionDenied, "{ext:?}");
    }
}

#[test]
fn primitives_without_a_modeler_refuse_by_name_and_write_nothing() {
    let at = |x: f64, y: f64, z: f64| Point3 { x, y, z };
    for ext in PRIMITIVES {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        // Well-formed payloads, so the refusal under test is the missing modeler
        // and not an input rule.
        let payload = if matches!(ext, ExtendedCommandId::Box | ExtendedCommandId::Sphere) {
            CommandPayload::Points(vec![at(0.0, 0.0, 0.0), at(1.0, 1.0, 1.0)])
        } else {
            CommandPayload::Points(vec![
                at(0.0, 0.0, 0.0),
                at(2.0, 2.0, 0.0),
                at(1.0, 1.0, 2.0),
            ])
        };
        let error = app
            .execute(&mut session, command(CommandId::Extended(ext), payload))
            .expect_err("no modeler, no body");
        assert_eq!(
            error,
            CadError::Unsupported("unsupported-runtime: solid-modeler".into()),
            "{ext:?}"
        );
        assert_eq!(revision(&app), before, "{ext:?} writes nothing");
        assert!(!app.can_undo(&DocumentId(1)), "{ext:?} records no undo");
    }
}
