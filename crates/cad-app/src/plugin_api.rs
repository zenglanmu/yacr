//! The plugin API's **read projections** of the one document slot.
//!
//! The plugin/`cad_api_call` surface reports host state back to a caller. The
//! authoritative document stays `cad-db`'s; this module only *projects* the
//! controller's own fields (document id, the session's label, the session mode
//! and the database id/revision) into a plain, transport-agnostic value.
//!
//! Why a separate plain struct: `cad-app` sits **below** `cad-ui-core` (see the
//! layering in the repository architecture gate), so the wire type cannot live
//! here. The shared layer maps [`PluginDocument`] onto its own serializable
//! shape; both carriers then serve the identical JSON.
//!
//! Nothing here mutates state: no transaction, no history entry, no `cad-db`
//! write. A missing document is `None` (the caller answers `cad_waiting`), never
//! a fabricated placeholder document.

use crate::host::HostController;
use crate::AppMode;

/// One open document as the plugin API reports it.
///
/// The ids are the decimal spellings of the authoritative identities
/// ([`cad_domain::DocumentId`]/[`cad_domain::DatabaseId`], both `u128`), and the
/// version is the database's revision counter. The strings are pre-rendered here
/// because the wire contract is JSON, not a numeric type with its own precision
/// rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDocument {
    /// The open document's [`cad_domain::DocumentId`], in decimal.
    pub id: String,
    /// The session's own label for the document.
    ///
    /// This is [`HostController::document_name_hint`] verbatim: the import path
    /// writes the opened file's label and the blank-document path writes the
    /// neutral `untitled` placeholder itself. Nothing is invented here — no path
    /// is synthesized and no `untitled` is added on top of an empty hint.
    pub name: String,
    /// Whether the session is in read-only (`Viewer`) mode.
    pub is_read_only: bool,
    /// The open database's [`cad_domain::DatabaseId`], in decimal.
    pub database_id: String,
    /// The open database's revision, in decimal.
    pub database_version: String,
}

impl HostController {
    /// The plugin API projection of the current document slot.
    ///
    /// `None` when the slot holds no document ([`HostController::drawing`]) —
    /// the honest answer for "nothing is open", never an empty document. The
    /// read-only flag is the **session mode** ([`HostController::mode`]), not a
    /// UI setting: `Viewer` is the one read-only mode this build has.
    pub fn plugin_document(&self) -> Option<PluginDocument> {
        let drawing = self.drawing()?;
        Some(PluginDocument {
            id: self.document_id.0.to_string(),
            name: self.document_name_hint.clone(),
            is_read_only: self.mode() == AppMode::Viewer,
            database_id: drawing.id().0.to_string(),
            database_version: drawing.revision().0.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller() -> HostController {
        HostController::with_demo_document([800.0, 600.0]).expect("the demo document opens")
    }

    #[test]
    fn the_projection_is_the_controllers_own_values() {
        let controller = controller();
        let drawing = controller.drawing().expect("a demo document");
        let projected = controller.plugin_document().expect("a document is open");
        assert_eq!(projected.id, controller.document_id.0.to_string());
        assert_eq!(projected.name, "yacr-demo");
        assert_eq!(projected.database_id, drawing.id().0.to_string());
        assert_eq!(projected.database_version, drawing.revision().0.to_string());
        assert!(!projected.is_read_only, "a fresh session is work mode");
    }

    #[test]
    fn viewer_mode_is_read_only_and_work_mode_is_not() {
        let mut controller = controller();
        controller
            .set_mode(AppMode::Viewer)
            .expect("the switch is a command");
        assert!(
            controller
                .plugin_document()
                .expect("a document is open")
                .is_read_only
        );
        controller
            .set_mode(AppMode::Work)
            .expect("the switch is a command");
        assert!(
            !controller
                .plugin_document()
                .expect("a document is open")
                .is_read_only
        );
    }

    #[test]
    fn the_version_and_label_follow_the_document_slot() {
        let mut controller = controller();
        let before = controller.plugin_document().expect("a document is open");
        assert_eq!(before.name, "yacr-demo");
        controller
            .new_blank_document()
            .expect("a blank document replaces the demo");
        let after = controller.plugin_document().expect("a document is open");
        assert_eq!(
            after.name, "untitled",
            "the blank path writes its own label; it is the session's, not ours"
        );
        let drawing = controller.drawing().expect("a document");
        assert_eq!(after.database_id, drawing.id().0.to_string());
        assert_eq!(after.database_version, drawing.revision().0.to_string());
    }

    #[test]
    fn no_document_projects_nothing() {
        let mut controller = controller();
        controller
            .close_document()
            .expect("closing the open document is a real state change");
        assert!(controller.plugin_document().is_none());
    }
}
