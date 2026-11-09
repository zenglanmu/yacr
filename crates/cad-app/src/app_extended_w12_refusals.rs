//! Wave 12 **named refusals** for the file/import/export/extension surface.
//!
//! These twenty-five AutoCAD 2026 names describe import/export formats or
//! extension runtimes this build does not have: 3DS, ACIS, raster images, WMF,
//! IGES, STL, DXB, HPGL, batch conversion, AutoLISP/ARX/.NET/VBA loaders and the
//! script-file runner.
//!
//! Each is admitted by the vocabulary and refused with `CadError::Unsupported`
//! carrying a **stable** reason. A refusal writes nothing, bumps no revision and
//! records no `cad-history` entry, so all are Viewer-legal. This build's only
//! vector output path remains `PLOT` (SVG/PDF).

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 12 command with its stable reason string.
    pub(crate) fn extended_w12_refusals(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, ext.selector())?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::ThreeDsIn => "3dsin.unsupported: 3DS import is not implemented",
        ExtendedCommandId::AcisIn => "acisin.unsupported: ACIS (SAT) import is not implemented",
        ExtendedCommandId::AcisOut => "acisout.unsupported: ACIS (SAT) export is not implemented",
        ExtendedCommandId::BmpOut => {
            "bmpout.unsupported: raster BMP export is not implemented; use PLOT for vector SVG/PDF"
        }
        ExtendedCommandId::JpgOut => {
            "jpgout.unsupported: raster JPEG export is not implemented; use PLOT for vector SVG/PDF"
        }
        ExtendedCommandId::PngOut => {
            "pngout.unsupported: raster PNG export is not implemented; use PLOT for vector SVG/PDF"
        }
        ExtendedCommandId::TifOut => {
            "tifout.unsupported: raster TIFF export is not implemented; use PLOT for vector SVG/PDF"
        }
        ExtendedCommandId::WmfIn => "wmfin.unsupported: Windows metafile import is not implemented",
        ExtendedCommandId::WmfOut => {
            "wmfout.unsupported: Windows metafile export is not implemented"
        }
        ExtendedCommandId::IgesImport => "igesimport.unsupported: IGES import is not implemented",
        ExtendedCommandId::IgesExport => "igesexport.unsupported: IGES export is not implemented",
        ExtendedCommandId::StlOut => "stlout.unsupported: STL export is not implemented",
        ExtendedCommandId::ImportFile => {
            "import.unsupported: the generic IMPORT picker is not implemented"
        }
        ExtendedCommandId::ExportToAutoCad => {
            "exporttoautocad.unsupported: exporting to another AutoCAD version is not implemented"
        }
        ExtendedCommandId::DwgConvert => {
            "dwgconvert.unsupported: batch DWG conversion is not implemented"
        }
        ExtendedCommandId::Plt2Dwg => "plt2dwg.unsupported: HPGL (PLT) import is not implemented",
        ExtendedCommandId::DxbIn => "dxbin.unsupported: DXB import is not implemented",
        ExtendedCommandId::AppLoad => "appload.unsupported: no plug-in/application loader model",
        ExtendedCommandId::Arx => "arx.unsupported: no ObjectARX loader model",
        ExtendedCommandId::NetLoad => "netload.unsupported: no .NET application loader model",
        ExtendedCommandId::VbaIde => "vbaide.unsupported: no VBA IDE model",
        ExtendedCommandId::VbaLoad => "vbaload.unsupported: no VBA loader model",
        ExtendedCommandId::VbaRun => "vbarun.unsupported: no VBA runtime",
        ExtendedCommandId::VbaUnload => "vbaunload.unsupported: no VBA runtime",
        ExtendedCommandId::Script => "script.unsupported: no script-file runner",
        other => {
            return format!(
                "{}: this command is not a Wave 12 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
