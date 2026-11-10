//! Drawing open entry points: byte-stream import with optional xref resolution.
//!
//! These methods stay verbatim with their previous `host` definitions; this
//! module exists only to keep `host/mod.rs` within the file-size limit.

use std::sync::Arc;

use cad_domain::*;

use crate::import::{FormatImporter, ImportLimits, ImportRequest, Importer};

use super::{HostController, OpenedDrawing, XrefOptions};

impl HostController {
    /// Import a DWG byte stream through the single importer boundary.
    ///
    /// Only a successful import replaces the document. On success the session,
    /// history, selection, layer overrides and active tool are rebuilt for the
    /// new content so the previous drawing's undo records and references can
    /// never act on it (audit B05). A failure leaves the current document and
    /// session untouched.
    ///
    /// External references are not read through this entry: it is the
    /// path-less/browser shape, so every xref is reported `browser-unavailable`.
    /// Hosts that own a real source path use
    /// [`HostController::open_bytes_with_xref`].
    pub fn open_bytes(&mut self, bytes: Arc<[u8]>, label: &str) -> CadResult<OpenedDrawing> {
        self.open_bytes_with_xref(bytes, label, None)
    }

    /// Import a DWG byte stream, optionally resolving external references.
    ///
    /// `xref` is the import-only loading policy/carrier. `None` keeps the
    /// path-less behaviour (every reference is `browser-unavailable`); loaded
    /// reference content is merged into the imported database and never re-read.
    /// The rest of the contract matches [`HostController::open_bytes`].
    pub fn open_bytes_with_xref(
        &mut self,
        bytes: Arc<[u8]>,
        label: &str,
        xref: Option<Arc<XrefOptions>>,
    ) -> CadResult<OpenedDrawing> {
        let request = ImportRequest {
            document: self.document_id,
            database: DatabaseId(1),
            bytes,
            limits: ImportLimits::default(),
            generation: self.session.generation,
            xref,
        };
        let importer = FormatImporter::new();
        let imported = importer.import(&request, &|| false)?;
        self.pending_open_label = Some(label.to_string());
        self.publish_imported(
            imported,
            &TaskStamp::new(self.document_id, self.session.generation),
        )
    }
}
