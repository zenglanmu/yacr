//! Wave 16 **named refusals**.
//!
//! The names in this module (image adjustment/quality/editor and the PDF/DGN
//! underlays) are admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a stable reason. Nothing is written, no
//! revision is bumped and no `cad-history` entry is recorded; all are
//! Viewer-legal. `IMAGE`/`-IMAGE` (report), `IMAGEATTACH` and `IMAGECLIP` are
//! real commands now, so they no longer appear here.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 16 command with its stable reason string.
    pub(crate) fn extended_w16_refusals(
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
        ExtendedCommandId::ImageAdjust => "imageadjust.unsupported: no image adjustment model",
        ExtendedCommandId::DashImageAdjust => "-imageadjust.unsupported: no image adjustment model",
        ExtendedCommandId::ImageQuality => {
            "imagequality.unsupported: no image display-quality model"
        }
        ExtendedCommandId::ImageApp => {
            "imageapp.unsupported: no external image-editor association model"
        }
        ExtendedCommandId::ImageAsyncWait => {
            "imageasyncwait.unsupported: no asynchronous image loading model"
        }
        ExtendedCommandId::ImageEdit => "imageedit.unsupported: no external image-editor model",
        ExtendedCommandId::ClassicImage => "classicimage.unsupported: no image manager; use IMAGE",
        ExtendedCommandId::PdfAttach => "pdfattach.unsupported: PDF underlays are not implemented",
        ExtendedCommandId::DashPdfAttach => {
            "-pdfattach.unsupported: PDF underlays are not implemented"
        }
        ExtendedCommandId::PdfClip => "pdfclip.unsupported: no PDF underlay clip model",
        ExtendedCommandId::PdfLayers => "pdflayers.unsupported: no PDF underlay layer model",
        ExtendedCommandId::PdfAdjust => "pdfadjust.unsupported: no PDF underlay adjustment model",
        ExtendedCommandId::PdfImport => {
            "pdfimport.unsupported: PDF geometry import is not implemented"
        }
        ExtendedCommandId::DashPdfImport => {
            "-pdfimport.unsupported: PDF geometry import is not implemented"
        }
        ExtendedCommandId::PdfShxText => {
            "pdfshxtext.unsupported: PDF SHX text conversion is not implemented"
        }
        ExtendedCommandId::DashPdfShxText => {
            "-pdfshxtext.unsupported: PDF SHX text conversion is not implemented"
        }
        ExtendedCommandId::DgnAttach => "dgnattach.unsupported: DGN underlays are not implemented",
        ExtendedCommandId::DashDgnAttach => {
            "-dgnattach.unsupported: DGN underlays are not implemented"
        }
        ExtendedCommandId::DgnClip => "dgnclip.unsupported: no DGN underlay clip model",
        ExtendedCommandId::DgnLayers => "dgnlayers.unsupported: no DGN underlay layer model",
        ExtendedCommandId::DgnAdjust => "dgnadjust.unsupported: no DGN underlay adjustment model",
        other => {
            return format!(
                "{}: this command is not a Wave 16 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
