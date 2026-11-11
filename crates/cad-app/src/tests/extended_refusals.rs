//! Wave 2 named-refusal contracts: `HYPERLINKOPTIONS`, `INPUTSEARCHOPTIONS`,
//! `-INPUTSEARCHOPTIONS`, `SIGVALIDATE`, `BASE`, `LIMITS`, `VIEWPLOTDETAILS`.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal is a
//! legitimate partial implementation in this build (same shape as
//! `LAYOUTMERGE`/`DIMBREAK`): it must write nothing, bump no revision and record
//! no history.
//!
//! # Graduated names (2026-11-11)
//!
//! `VPLAYER`, `MVSETUP`, `PSBSCALE` and `PSTSCALE` have left this list: they are
//! real paper-space writes now, dispatched through
//! [`Application::extended_paper_write`] and contract-tested in
//! [`paper_space_writes`] (`tests/drawing/layout_commands.rs`). Their old
//! `*.unsupported` strings are gone on purpose — a graduated command must not
//! keep a refusal string that claims the feature has no model.

use super::*;

// The graduated Wave 2 paper-space writes (`VPLAYER`, `MVSETUP`, `PSBSCALE`,
// `PSTSCALE`, and the real `VPMAX`/`VPMIN` state). Declared here with an
// explicit path so `tests/layout_commands.rs` (at the size ceiling) does not
// have to grow.
#[path = "drawing/layout_commands.rs"]
mod paper_space_writes;

const REFUSALS: [(ExtendedCommandId, &str); 7] = [
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
            CommandId::Extended(ExtendedCommandId::HyperlinkOptions),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    match refused {
        Err(CadError::InvalidInput(message)) => {
            assert!(
                message.contains("HYPERLINKOPTIONS needs no payload"),
                "{message}"
            );
        }
        other => panic!("expected InvalidInput for a payload, got {other:?}"),
    }
    assert!(!app.can_undo(&DocumentId(1)));
}
