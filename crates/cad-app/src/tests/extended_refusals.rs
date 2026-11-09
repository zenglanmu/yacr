//! Wave 2 named-refusal contracts: `HYPERLINKOPTIONS`, `INPUTSEARCHOPTIONS`,
//! `-INPUTSEARCHOPTIONS`, `SIGVALIDATE`, `BASE`, `LIMITS`, `VIEWPLOTDETAILS`,
//! `VPLAYER`, `PSBSCALE`, `PSTSCALE`, `MVSETUP` (`MVIEW` graduated to a real
//! viewport write).
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal is a
//! legitimate partial implementation in this build (same shape as
//! `LAYOUTMERGE`/`DIMBREAK`): it must write nothing, bump no revision and record
//! no history.

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 11] = [
    (
        ExtendedCommandId::HyperlinkOptions,
        "hyperlinkoptions.unsupported: this build has no hyperlink storage model",
    ),
    (
        ExtendedCommandId::InputSearchOptions,
        "inputsearchoptions.unsupported: input-search settings are not modeled",
    ),
    (
        ExtendedCommandId::DashInputSearchOptions,
        "-inputsearchoptions.unsupported: input-search settings are not modeled",
    ),
    (
        ExtendedCommandId::SigValidate,
        "sigvalidate.unsupported: this build has no digital-signature model",
    ),
    (
        ExtendedCommandId::BasePoint,
        "base.unsupported: the drawing stores no insertion base point",
    ),
    (
        ExtendedCommandId::DrawingLimits,
        "limits.unsupported: this build has no drawing-limits model",
    ),
    (
        ExtendedCommandId::ViewPlotDetails,
        "viewplotdetails.unsupported: no plot-details model",
    ),
    (
        ExtendedCommandId::VPlayer,
        "vplayer.viewport_layer_state_unsupported: no per-viewport layer state",
    ),
    (
        ExtendedCommandId::PsbScale,
        "psbscale.write_unsupported: no viewport scale write API",
    ),
    (
        ExtendedCommandId::PstScale,
        "pstscale.write_unsupported: no viewport scale write API",
    ),
    (
        ExtendedCommandId::MvSetup,
        "mvsetup.unsupported: no viewport-group model",
    ),
];

#[test]
fn every_wave2_refusal_returns_its_exact_stable_reason_and_changes_nothing() {
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
            CommandId::Extended(ExtendedCommandId::VPlayer),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    match refused {
        Err(CadError::InvalidInput(message)) => {
            assert!(message.contains("VPLAYER needs no payload"), "{message}");
        }
        other => panic!("expected InvalidInput for a payload, got {other:?}"),
    }
    assert!(!app.can_undo(&DocumentId(1)));
}
