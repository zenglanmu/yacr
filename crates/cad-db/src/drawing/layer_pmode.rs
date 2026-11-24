//! The document-level `LAYERPMODE` gate (`$LAYERPMODE`-equivalent).
//!
//! AutoCAD saves `LAYERPMODE` **in the drawing** (initial value On). This build
//! stores it in the document header and writes it to
//! `document.json` → `variables.layer_pmode` only when it differs from the
//! default (`true`), exactly like the `units_display` block: a document that
//! never switched the gate off keeps the pre-`LAYERPMODE` header bytes, and an
//! absent key decodes to On.
//!
//! Contract (the same shape as [`DrawingDatabase::set_linetype_scale`]):
//!
//! * writing the stored value is an authoritative no-op (`before == after`, no
//!   revision bump);
//! * a real change is exactly **one** revision bump and a header-only
//!   [`ChangeSet`] (no entity/table rows);
//! * **not undoable** — `cad-history` has no header-variable patch kind, so the
//!   command layer records no history entry rather than fabricating one.
//!
//! The gate itself is consumed by `cad-app`'s `LAYERP` snapshot push, which reads
//! this document value; it is not duplicated as session state.

use cad_domain::*;

use super::DrawingDatabase;
use crate::change::ChangeSet;
use crate::scope::ChangeScope;

impl DrawingDatabase {
    /// Whether layer-setting changes record `LAYERP` snapshots (AutoCAD's
    /// `LAYERPMODE`; On is the default and the value of every old archive).
    pub fn layer_pmode(&self) -> bool {
        self.layer_pmode
    }

    /// Set the document's `LAYERPMODE` gate.
    ///
    /// The already stored value is an authoritative no-op. A real change is one
    /// revision bump and an empty-object-changes [`ChangeSet`]; nothing is raised
    /// when the value is unchanged.
    pub fn set_layer_pmode(
        &mut self,
        enabled: bool,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        let before = self.revision;
        if self.layer_pmode == enabled {
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
                scope: ChangeScope::default(),
            });
        }
        self.layer_pmode = enabled;
        let after = Revision(before.0 + 1);
        self.revision = after;
        // `LAYERPMODE` lives in `document.json`; no element or table row moved.
        let mut scope = ChangeScope::default();
        scope.header();
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
            scope,
        })
    }
}
