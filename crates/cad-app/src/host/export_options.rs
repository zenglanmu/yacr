//! Host-set Save-As export options: the ACIS payload version and DXF container.
//!
//! These are **not** document state: they are applied to the `ExportRequest` the
//! host builds, so they never touch `cad-db`, never bump a revision and never
//! create a history record. The defaults are the pre-selector behaviour (ACIS 7.0
//! in a R2010 DXF), so a host that never sets them writes the same bytes as
//! before.
//!
//! The container/payload matrix is enforced by the exporter itself, before any
//! writer runs (`export.acis_asm_requires_r2013`, `export.dwg_version_fixed`).
//! This module never rewrites a refused combination into an accepted one.
pub use cad_import_acadrust::export::{AcisVersion, DxfTarget, ExportOptions};

use cad_domain::{CadError, CadResult};
use cad_import_acadrust::export::{
    export, ExportFormat, ExportLimits, ExportReport, ExportRequest, ExportSpace,
};

use super::HostController;

impl HostController {
    /// Set the export options the next [`Self::export_dxf`]/[`Self::export_dwg`]
    /// uses.
    ///
    /// The value is stored verbatim; an unrepresentable combination is refused by
    /// `export` with a named error rather than silently replaced by another
    /// version.
    pub fn set_export_options(&mut self, options: ExportOptions) {
        self.export_options = options;
    }

    /// The currently configured export options.
    pub fn export_options(&self) -> ExportOptions {
        self.export_options
    }

    /// Reverse-map the active document to lossy DXF bytes plus the export report.
    ///
    /// Host-owned: the platform chooses the path and writes the bytes. This is a
    /// **Save As** whose report records every conversion/drop; a missing document
    /// is an explicit `InvalidInput`, never an empty success.
    pub fn export_dxf(&self) -> CadResult<(Vec<u8>, ExportReport)> {
        self.export_as(ExportFormat::DxfText)
    }

    /// Reverse-map the active document to **experimental** binary DWG bytes.
    ///
    /// Same document/report as [`Self::export_dxf`], written by upstream
    /// `acadrust::DwgWriter`: lossy and NOT verified against AutoCAD/vendor.
    pub fn export_dwg(&self) -> CadResult<(Vec<u8>, ExportReport)> {
        self.export_as(ExportFormat::Dwg)
    }

    /// The one request builder shared by both formats, so they cannot drift.
    fn export_as(&self, format: ExportFormat) -> CadResult<(Vec<u8>, ExportReport)> {
        let document = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        // A DWG is written as AC1032 by the upstream writer, whose version comes
        // from the document; the configured DXF target is a DXF-only choice, so a
        // DWG request names its own container instead of asking for a version the
        // writer would ignore (the exporter refuses any other DWG target with
        // `export.dwg_version_fixed`).
        let dxf_target = match format {
            ExportFormat::DxfText => self.export_options.dxf_target,
            ExportFormat::Dwg => DxfTarget::R2018,
        };
        let request = ExportRequest {
            database: document.drawing.as_ref(),
            document_id: self.document_id,
            space: ExportSpace::Model,
            format,
            limits: ExportLimits::default(),
            units: Some(document.units.clone()),
            acis_version: self.export_options.acis_version,
            dxf_target,
        };
        export(&request)
    }
}
