//! Wave 4 named-refusal contracts: `BSCALE`, `BLOCKREPLACE`, `BREPLACE`,
//! `-BREPLACE`, `SPLINEDIT`, `BURST`, `CLIP`, `CLIPIT`, `REGION`, `WBLOCK`,
//! `-WBLOCK`, `BSAVE`, `BSAVEAS`, `CHSPACE`.
//!
//! `MINSERT`, `ADDSELECTED` and `MLEDIT`/`-MLEDIT` were refusals in this wave and
//! graduated to real entity writes on 2026-11-12 (`app_drawing::minsert`,
//! `app_drawing::addselected`, `app_drawing::mledit`); their contracts live in
//! `tests::drawing::minsert`, `tests::drawing::addselected` and
//! `tests::extended_w9_mleader` and are no longer part of this table.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal must write
//! nothing, bump no revision and record no history. These names are
//! Viewer-legal (they only answer honestly).

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 14] = [
    (
        ExtendedCommandId::BScale,
        "bscale.unsupported: BSCALE needs a factor and an insertion reference; use SCALE on the \
         selected insert",
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
