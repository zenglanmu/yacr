//! Wave 3 named-refusal contracts: `DIMJOGGED`, `DIMINSPECT`, `-DIMINSPECT`,
//! `ANNORESET`, `ANNOUPDATE`, `OBJECTSCALE`, `-OBJECTSCALE`, `QLATTACH`,
//! `QLATTACHSET`, `QLDETACHSET`, `ARRAYCLOSE`, `ARRAYEDIT`, `BEDIT`, `-BEDIT`,
//! `BCLOSE`, `BCYCLEORDER`, `BGRIPSET`, `BESETTINGS`, `BTESTBLOCK`,
//! `LAYERPALETTE`, `CONSTRAINTBAR`, `CONSTRAINTSETTINGS`.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal is a
//! legitimate partial implementation in this build (same shape as
//! `LAYOUTMERGE`/`DIMBREAK`): it must write nothing, bump no revision and record
//! no history.

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 22] = [
    (
        ExtendedCommandId::DimJogged,
        "dimjogged.unsupported: the dimension domain stores no jog field",
    ),
    (
        ExtendedCommandId::DimInspect,
        "diminspect.unsupported: the dimension domain stores no inspection data",
    ),
    (
        ExtendedCommandId::DashDimInspect,
        "-diminspect.unsupported: the dimension domain stores no inspection data",
    ),
    (
        ExtendedCommandId::AnnoReset,
        "annoreset.unsupported: this build has no annotative-object model",
    ),
    (
        ExtendedCommandId::AnnoUpdate,
        "annoupdate.unsupported: this build has no annotative-object model",
    ),
    (
        ExtendedCommandId::ObjectScale,
        "objectscale.unsupported: this build has no annotative-object model",
    ),
    (
        ExtendedCommandId::DashObjectScale,
        "-objectscale.unsupported: this build has no annotative-object model",
    ),
    (
        ExtendedCommandId::QlAttach,
        "qlattach.unsupported: this build has no QLEADER annotation-attachment model",
    ),
    (
        ExtendedCommandId::QlAttachSet,
        "qlattachset.unsupported: this build has no QLEADER annotation-attachment model",
    ),
    (
        ExtendedCommandId::QlDetachSet,
        "qldetchset.unsupported: this build has no QLEADER annotation-attachment model",
    ),
    (
        ExtendedCommandId::ArrayClose,
        "arrayclose.unsupported: this build has no associative-array edit state",
    ),
    (
        ExtendedCommandId::ArrayEdit,
        "arrayedit.unsupported: this build has no associative-array edit state",
    ),
    (
        ExtendedCommandId::BlockEditor,
        "bedit.unsupported: this build has no block editor",
    ),
    (
        ExtendedCommandId::DashBlockEditor,
        "-bedit.unsupported: this build has no block editor",
    ),
    (
        ExtendedCommandId::BlockEditorClose,
        "bclose.unsupported: this build has no block editor",
    ),
    (
        ExtendedCommandId::BCycleOrder,
        "bcycleorder.unsupported: this build has no dynamic-block grip model",
    ),
    (
        ExtendedCommandId::BGripSet,
        "bgripset.unsupported: this build has no dynamic-block parameter model",
    ),
    (
        ExtendedCommandId::BEditorSettings,
        "besettings.unsupported: this build has no block editor",
    ),
    (
        ExtendedCommandId::BTestBlock,
        "btestblock.unsupported: this build has no block editor",
    ),
    (
        ExtendedCommandId::LayerPalette,
        "layerpalette.unsupported: there is no separate layer palette; use LAYER",
    ),
    (
        ExtendedCommandId::ConstraintBar,
        "constraintbar.unsupported: this build has no constraint model",
    ),
    (
        ExtendedCommandId::ConstraintSettings,
        "constraintsettings.unsupported: this build has no constraint model",
    ),
];

#[test]
fn every_wave3_refusal_returns_its_exact_stable_reason_and_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before_revision = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let before_entities = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();

    for (ext, expected) in REFUSALS {
        let refused = app.execute(
            &mut session,
            command(CommandId::Extended(ext), CommandPayload::None),
        );
        match refused {
            Err(CadError::Unsupported(reason)) => {
                assert_eq!(reason, expected, "{} reason must be stable", ext.selector());
            }
            other => panic!("{expected} expected Unsupported, got {other:?}"),
        }
        assert_eq!(
            app.workspace.documents[&DocumentId(1)].drawing.revision(),
            before_revision,
            "{} must not bump the revision",
            ext.selector()
        );
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity_count(),
            before_entities,
            "{} must not write entities",
            ext.selector()
        );
        assert!(
            !app.can_undo(&DocumentId(1)),
            "{} must not record history",
            ext.selector()
        );
    }
}

#[test]
fn a_refusal_payload_is_rejected_before_the_feature_reason() {
    let (mut app, mut session) = application_with_document();
    let refused = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::BlockEditor),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    match refused {
        Err(CadError::InvalidInput(message)) => {
            assert!(message.contains("BEDIT needs no payload"), "{message}");
        }
        other => panic!("expected InvalidInput for a payload, got {other:?}"),
    }
    assert!(!app.can_undo(&DocumentId(1)));
}
