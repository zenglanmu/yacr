//! Wave 16 named-refusal contracts.
//!
//! Each remaining Wave 16 name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a stable reason; nothing is written, no
//! revision is bumped and no history entry appears. All are Viewer-legal.
//! `IMAGE`/`-IMAGE`, `IMAGEATTACH`, `IMAGECLIP` and `IMAGEADJUST` are real
//! commands and are pinned in `tests::drawing::image`.

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 19] = [
    (
        ExtendedCommandId::ImageQuality,
        "imagequality.unsupported: no image display-quality model",
    ),
    (
        ExtendedCommandId::ImageApp,
        "imageapp.unsupported: no external image-editor association model",
    ),
    (
        ExtendedCommandId::ImageAsyncWait,
        "imageasyncwait.unsupported: no asynchronous image loading model",
    ),
    (
        ExtendedCommandId::ImageEdit,
        "imageedit.unsupported: no external image-editor model",
    ),
    (
        ExtendedCommandId::ClassicImage,
        "classicimage.unsupported: no image manager; use IMAGE",
    ),
    (
        ExtendedCommandId::PdfAttach,
        "pdfattach.unsupported: PDF underlays are not implemented",
    ),
    (
        ExtendedCommandId::DashPdfAttach,
        "-pdfattach.unsupported: PDF underlays are not implemented",
    ),
    (
        ExtendedCommandId::PdfClip,
        "pdfclip.unsupported: no PDF underlay clip model",
    ),
    (
        ExtendedCommandId::PdfLayers,
        "pdflayers.unsupported: no PDF underlay layer model",
    ),
    (
        ExtendedCommandId::PdfAdjust,
        "pdfadjust.unsupported: no PDF underlay adjustment model",
    ),
    (
        ExtendedCommandId::PdfImport,
        "pdfimport.unsupported: PDF geometry import is not implemented",
    ),
    (
        ExtendedCommandId::DashPdfImport,
        "-pdfimport.unsupported: PDF geometry import is not implemented",
    ),
    (
        ExtendedCommandId::PdfShxText,
        "pdfshxtext.unsupported: PDF SHX text conversion is not implemented",
    ),
    (
        ExtendedCommandId::DashPdfShxText,
        "-pdfshxtext.unsupported: PDF SHX text conversion is not implemented",
    ),
    (
        ExtendedCommandId::DgnAttach,
        "dgnattach.unsupported: DGN underlays are not implemented",
    ),
    (
        ExtendedCommandId::DashDgnAttach,
        "-dgnattach.unsupported: DGN underlays are not implemented",
    ),
    (
        ExtendedCommandId::DgnClip,
        "dgnclip.unsupported: no DGN underlay clip model",
    ),
    (
        ExtendedCommandId::DgnLayers,
        "dgnlayers.unsupported: no DGN underlay layer model",
    ),
    (
        ExtendedCommandId::DgnAdjust,
        "dgnadjust.unsupported: no DGN underlay adjustment model",
    ),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave16_refusal_answers_with_its_exact_stable_reason() {
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
fn every_wave16_refusal_stays_viewer_legal() {
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
