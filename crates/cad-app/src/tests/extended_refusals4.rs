//! Wave 4 named-refusal contracts: `BSCALE`, `BLOCKREPLACE`, `BREPLACE`,
//! `-BREPLACE`, `SPLINEDIT`, `REGION`, `WBLOCK`, `-WBLOCK`, `BSAVE`,
//! `BSAVEAS`.
//!
//! `BURST` graduated on 2026-11-24 (ADR-0055 M3): it is the EXPLODE
//! decomposition plus attribute-value-to-TEXT conversion and is no longer part
//! of this table; its contract lives in [`crate::tests::attribute_commands`].
//!
//! `MINSERT`, `ADDSELECTED` and `MLEDIT`/`-MLEDIT` were refusals in this wave and
//! graduated to real entity writes on 2026-11-12 (`app_drawing::minsert`,
//! `app_drawing::addselected`, `app_drawing::mledit`); their contracts live in
//! `tests::drawing::minsert`, `tests::drawing::addselected` and
//! `tests::extended_w9_mleader` and are no longer part of this table.
//!
//! `CLIP` and `CLIPIT` graduated on 2026-11-15 to real clip writes over the
//! existing `ImageClip`/`BlockClip` models (`app_drawing::clip_edit`); their
//! contract lives in `tests::drawing::clip`.
//!
//! `CHSPACE` graduated on 2026-11-16 to a real model↔paper migration
//! (`app_layout::chspace`, one transaction / one undo step) and is Work-only, so
//! it is no longer part of this Viewer-legal refusal table; its contract lives in
//! `tests::layout_chspace`.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal must write
//! nothing, bump no revision and record no history. These names are
//! Viewer-legal (they only answer honestly).

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 10] = [
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
