//! Wave 4 named-refusal contracts: `BSCALE`, `MINSERT`, `BLOCKREPLACE`,
//! `BREPLACE`, `-BREPLACE`, `SPLINEDIT`, `MLEDIT`, `-MLEDIT`, `BURST`, `CLIP`,
//! `CLIPIT`, `REGION`, `ADDSELECTED`, `WBLOCK`, `-WBLOCK`, `BSAVE`, `BSAVEAS`,
//! `CHSPACE`.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal must write
//! nothing, bump no revision and record no history. These names are
//! Viewer-legal (they only answer honestly).

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 18] = [
    (
        ExtendedCommandId::BScale,
        "bscale.unsupported: BSCALE needs a factor and an insertion reference; use SCALE on the \
         selected insert",
    ),
    (
        ExtendedCommandId::MInsert,
        "minsert.unsupported: MINSERT needs a block name and row/column counts; use INSERT then \
         ARRAYRECT",
    ),
    (
        ExtendedCommandId::BlockReplace,
        "blockreplace.unsupported: BLOCKREPLACE needs two block definitions; this build has no \
         block-replace editor",
    ),
    (
        ExtendedCommandId::BReplace,
        "breplace.unsupported: BREPLACE needs two block definitions; this build has no \
         block-replace editor",
    ),
    (
        ExtendedCommandId::DashBReplace,
        "-breplace.unsupported: BREPLACE needs two block definitions; this build has no \
         block-replace editor",
    ),
    (
        ExtendedCommandId::SplineEdit,
        "splinedit.unsupported: this build has no NURBS edit surface",
    ),
    (
        ExtendedCommandId::MlEdit,
        "mlinedit.unsupported: this build has no MLINE join/cap edit model",
    ),
    (
        ExtendedCommandId::DashMlEdit,
        "-mlinedit.unsupported: this build has no MLINE join/cap edit model",
    ),
    (
        ExtendedCommandId::Burst,
        "burst.unsupported: this build has no block-attribute model to preserve",
    ),
    (
        ExtendedCommandId::Clip,
        "clip.unsupported: this build has no image/xref clipping model",
    ),
    (
        ExtendedCommandId::ClipIt,
        "clipit.unsupported: this build has no image/xref clipping model",
    ),
    (
        ExtendedCommandId::Region,
        "region.unsupported: this build has no Region geometry model",
    ),
    (
        ExtendedCommandId::AddSelected,
        "addselected.unsupported: add-selected needs per-type creation prompts that this build \
         does not model",
    ),
    (
        ExtendedCommandId::WBlock,
        "wblock.unsupported: writing a block definition to a separate drawing file is not \
         implemented",
    ),
    (
        ExtendedCommandId::DashWBlock,
        "-wblock.unsupported: writing a block definition to a separate drawing file is not \
         implemented",
    ),
    (
        ExtendedCommandId::BSave,
        "bsave.unsupported: this build has no block editor",
    ),
    (
        ExtendedCommandId::BSaveAs,
        "bsaveas.unsupported: this build has no block editor",
    ),
    (
        ExtendedCommandId::ChSpace,
        "chspace.unsupported: moving entities between model and paper space is not implemented",
    ),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave4_refusal_answers_with_its_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a Wave 4 refusal never succeeds");
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
fn wave4_refusals_are_also_available_in_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).unwrap();
    for (ext, expected) in REFUSALS {
        let error = app
            .execute(
                &mut session,
                command(CommandId::Extended(ext), CommandPayload::None),
            )
            .expect_err("a Wave 4 refusal never succeeds");
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
                CommandId::Extended(ExtendedCommandId::BScale),
                CommandPayload::Mode(AppMode::Work),
            ),
        )
        .expect_err("a payload is refused");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert!(error.to_string().contains("BSCALE needs no payload"));
}
