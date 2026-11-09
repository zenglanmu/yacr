//! Wave 7 named-refusal contracts: dynamic-block actions, parameters, lookup
//! tables, visibility hide/show, block authoring, reset, and the geometric /
//! dimensional constraint family.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal must write
//! nothing, bump no revision and record no history. These names are
//! Viewer-legal (they only answer honestly).

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 28] = [
    (
        ExtendedCommandId::BAction,
        "baction.unsupported: this build has no dynamic-block action model",
    ),
    (
        ExtendedCommandId::BActionBar,
        "bactionbar.unsupported: this build has no dynamic-block action model",
    ),
    (
        ExtendedCommandId::BActionSet,
        "bactionset.unsupported: this build has no dynamic-block action model",
    ),
    (
        ExtendedCommandId::BActionTool,
        "bactiontool.unsupported: this build has no dynamic-block action model",
    ),
    (
        ExtendedCommandId::BAssociate,
        "bassociate.unsupported: this build has no dynamic-block action model",
    ),
    (
        ExtendedCommandId::BAuthorPalette,
        "bauthorpalette.unsupported: this build has no block editor",
    ),
    (
        ExtendedCommandId::BAuthorPaletteClose,
        "bauthorpaletteclose.unsupported: this build has no block editor",
    ),
    (
        ExtendedCommandId::BConstruction,
        "bconstruction.unsupported: this build has no dynamic-block construction geometry",
    ),
    (
        ExtendedCommandId::BCParameter,
        "bcparameter.unsupported: this build has no dynamic-block parameter model",
    ),
    (
        ExtendedCommandId::DashBCParameter,
        "-bcparameter.unsupported: this build has no dynamic-block parameter model",
    ),
    (
        ExtendedCommandId::BParameter,
        "bparameter.unsupported: this build has no dynamic-block parameter model",
    ),
    (
        ExtendedCommandId::BLookupTable,
        "blookuptable.unsupported: this build has no dynamic-block lookup table",
    ),
    (
        ExtendedCommandId::BTable,
        "btable.unsupported: this build has no dynamic-block lookup table",
    ),
    (
        ExtendedCommandId::BvHide,
        "bvhide.unsupported: no per-visibility-state entity hide API",
    ),
    (
        ExtendedCommandId::BvShow,
        "bvshow.unsupported: no per-visibility-state entity show API",
    ),
    (
        ExtendedCommandId::ResetBlock,
        "resetblock.unsupported: no dynamic-block parameter reset API",
    ),
    (
        ExtendedCommandId::AutoConstrain,
        "autoconstrain.unsupported: this build has no geometric-constraint model",
    ),
    (
        ExtendedCommandId::DelConstraint,
        "delconstraint.unsupported: this build has no geometric-constraint model",
    ),
    (
        ExtendedCommandId::DimConstraint,
        "dimconstraint.unsupported: this build has no dimensional-constraint model",
    ),
    (
        ExtendedCommandId::GeomConstraint,
        "geomconstraint.unsupported: this build has no geometric-constraint model",
    ),
    (
        ExtendedCommandId::DcAligned,
        "dcaligned.unsupported: this build has no dimensional-constraint model",
    ),
    (
        ExtendedCommandId::DcAngular,
        "dcangular.unsupported: this build has no dimensional-constraint model",
    ),
    (
        ExtendedCommandId::DcConvert,
        "dcconvert.unsupported: this build has no dimensional-constraint model",
    ),
    (
        ExtendedCommandId::DcDiameter,
        "dcdiameter.unsupported: this build has no dimensional-constraint model",
    ),
    (
        ExtendedCommandId::DcDisplay,
        "dcdisplay.unsupported: this build has no dimensional-constraint model",
    ),
    (
        ExtendedCommandId::DcForm,
        "dcform.unsupported: this build has no dimensional-constraint model",
    ),
    (
        ExtendedCommandId::DcHorizontal,
        "dchorizontal.unsupported: this build has no dimensional-constraint model",
    ),
    (
        ExtendedCommandId::DcLinear,
        "dclinear.unsupported: this build has no dimensional-constraint model",
    ),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave7_refusal_answers_with_its_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a Wave 7 refusal never succeeds");
        assert_eq!(
            error,
            CadError::Unsupported(expected.to_string()),
            "{}",
            ext.selector()
        );
        assert_eq!(revision(&app), before, "{} writes nothing", ext.selector());
        assert!(
            !app.can_undo(&DocumentId(1)),
            "{} records no history",
            ext.selector()
        );
        assert!(
            !ext.requires_work_mode(),
            "{} stays Viewer-legal",
            ext.selector()
        );
    }
}

#[test]
fn wave7_refusals_are_also_available_in_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).unwrap();
    for (ext, expected) in REFUSALS {
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a Wave 7 refusal never succeeds");
        assert_eq!(
            error,
            CadError::Unsupported(expected.to_string()),
            "{}",
            ext.selector()
        );
    }
}

#[test]
fn a_caller_payload_is_refused_before_the_feature_reason() {
    let (mut app, mut session) = application_with_document();
    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::BAction),
                CommandPayload::Mode(AppMode::Work),
            ),
        )
        .expect_err("a payload is refused");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert!(error.to_string().contains("BACTION needs no payload"));
}
