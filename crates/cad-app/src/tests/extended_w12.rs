//! Wave 12 named-refusal contracts: file/import/export/extension surfaces.
//!
//! Every name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a stable reason; nothing is written, no
//! revision is bumped and no history entry appears. All are Viewer-legal.

use super::*;

const REFUSALS: [(ExtendedCommandId, &str); 25] = [
    (
        ExtendedCommandId::ThreeDsIn,
        "3dsin.unsupported: 3DS import is not implemented",
    ),
    (
        ExtendedCommandId::AcisIn,
        "acisin.unsupported: ACIS (SAT) import is not implemented",
    ),
    (
        ExtendedCommandId::AcisOut,
        "acisout.unsupported: ACIS (SAT) export is not implemented",
    ),
    (
        ExtendedCommandId::BmpOut,
        "bmpout.unsupported: raster BMP export is not implemented; use PLOT for vector SVG/PDF",
    ),
    (
        ExtendedCommandId::JpgOut,
        "jpgout.unsupported: raster JPEG export is not implemented; use PLOT for vector SVG/PDF",
    ),
    (
        ExtendedCommandId::PngOut,
        "pngout.unsupported: raster PNG export is not implemented; use PLOT for vector SVG/PDF",
    ),
    (
        ExtendedCommandId::TifOut,
        "tifout.unsupported: raster TIFF export is not implemented; use PLOT for vector SVG/PDF",
    ),
    (
        ExtendedCommandId::WmfIn,
        "wmfin.unsupported: Windows metafile import is not implemented",
    ),
    (
        ExtendedCommandId::WmfOut,
        "wmfout.unsupported: Windows metafile export is not implemented",
    ),
    (
        ExtendedCommandId::IgesImport,
        "igesimport.unsupported: IGES import is not implemented",
    ),
    (
        ExtendedCommandId::IgesExport,
        "igesexport.unsupported: IGES export is not implemented",
    ),
    (
        ExtendedCommandId::StlOut,
        "stlout.unsupported: STL export is not implemented",
    ),
    (
        ExtendedCommandId::ImportFile,
        "import.unsupported: the generic IMPORT picker is not implemented",
    ),
    (
        ExtendedCommandId::ExportToAutoCad,
        "exporttoautocad.unsupported: exporting to another AutoCAD version is not implemented",
    ),
    (
        ExtendedCommandId::DwgConvert,
        "dwgconvert.unsupported: batch DWG conversion is not implemented",
    ),
    (
        ExtendedCommandId::Plt2Dwg,
        "plt2dwg.unsupported: HPGL (PLT) import is not implemented",
    ),
    (
        ExtendedCommandId::DxbIn,
        "dxbin.unsupported: DXB import is not implemented",
    ),
    (
        ExtendedCommandId::AppLoad,
        "appload.unsupported: no plug-in/application loader model",
    ),
    (
        ExtendedCommandId::Arx,
        "arx.unsupported: no ObjectARX loader model",
    ),
    (
        ExtendedCommandId::NetLoad,
        "netload.unsupported: no .NET application loader model",
    ),
    (
        ExtendedCommandId::VbaIde,
        "vbaide.unsupported: no VBA IDE model",
    ),
    (
        ExtendedCommandId::VbaLoad,
        "vbaload.unsupported: no VBA loader model",
    ),
    (
        ExtendedCommandId::VbaRun,
        "vbarun.unsupported: no VBA runtime",
    ),
    (
        ExtendedCommandId::VbaUnload,
        "vbaunload.unsupported: no VBA runtime",
    ),
    (
        ExtendedCommandId::Script,
        "script.unsupported: no script-file runner",
    ),
];

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn every_wave12_refusal_answers_with_its_exact_stable_reason() {
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
fn every_wave12_refusal_stays_viewer_legal() {
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
