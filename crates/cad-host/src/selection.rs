//! Host-side selection replacement (`cad_select_entities`).
//!
//! The BIM panel's click action asks for **one logical component**, which is one
//! or more geometry rows, to become the current selection. The wire carries the
//! rows' entity ids as `u64` (the same truncation the BIM page uses); this module
//! is where the ids meet the authoritative database:
//!
//! * the open document is the only source of truth — a host with no document
//!   refuses by marker ([`SELECTION_NO_DOCUMENT`]), it never fabricates an empty
//!   selection;
//! * every id must resolve to a real entity. The whole request is refused with
//!   [`SELECTION_UNKNOWN_ENTITY`]` : <id>` when one does not: a stale BIM row (for
//!   example after an open replaced the document) must not silently select the
//!   subset that still exists;
//! * the replacement itself is the ordinary selection command
//!   ([`cad_app::host::HostController::set_selection`]) — read-only, no history
//!   entry, the same funnel a click-select uses.
//!
//! The selection is validated **before** it is applied, so a refused request
//! leaves the previous selection exactly as it was.

use cad_domain::{CadError, CadResult, EntityId};

use crate::HostCore;

/// The stable refusal of a selection request on a host with no open document.
pub const SELECTION_NO_DOCUMENT: &str = "selection.no_document";

/// The stable prefix of the refusal for an id that is not in the open document.
///
/// The wire marker is `selection.unknown_entity: <id>` (decimal id appended).
pub const SELECTION_UNKNOWN_ENTITY: &str = "selection.unknown_entity";

impl HostCore {
    /// Replace the current selection with the given entity ids.
    ///
    /// Ids are the wire's `u64` spelling converted to [`EntityId`]. The document
    /// binding comes from the controller itself, so the refs always address the
    /// open document (an instance path and a sub-element are not expressible on
    /// this channel: the panel selects whole geometry rows).
    pub fn select_entities(&self, ids: &[EntityId]) -> CadResult<()> {
        let mut controller = self.controller.borrow_mut();
        let Some(drawing) = controller.drawing() else {
            return Err(CadError::Unsupported(SELECTION_NO_DOCUMENT.to_string()));
        };
        let document = controller.document_id;
        let mut refs = Vec::with_capacity(ids.len());
        for entity in ids {
            if drawing.entity(*entity).is_none() {
                return Err(CadError::Unsupported(format!(
                    "{SELECTION_UNKNOWN_ENTITY}: {}",
                    entity.0
                )));
            }
            refs.push(cad_app::selection::model_ref(document, *entity));
        }
        controller.set_selection(refs)
    }
}
