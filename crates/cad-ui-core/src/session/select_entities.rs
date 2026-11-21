//! `CadSession::select_entities` — the BIM panel's selection replacement.
//!
//! The host owns the resolution and the atomic replacement
//! (`HostCore::select_entities`); this module only maps the host's refusal to the
//! wire's technical string, exactly like the other typed session entry points. It
//! lives beside them (rather than in `mod.rs`) to keep that file inside the
//! code-size gate.

use super::*;

impl CadSession {
    /// Replace the current selection with these entity ids (`cad_select_entities`).
    ///
    /// Every id must resolve in the open document; an unknown id refuses the whole
    /// request with `selection.unknown_entity: <id>` and leaves the previous
    /// selection untouched (never a partial selection). A host with no document
    /// refuses with `selection.no_document`. The wire's `u64` ids are widened to
    /// `cad_domain::EntityId` by the host.
    pub fn select_entities(&self, ids: &[EntityId]) -> Result<(), String> {
        match self.core.select_entities(ids) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                // The desktop queues the request (`{accepted:true}` is admission),
                // so the real verdict has to reach `cad_status.error`; the direct
                // sink also gets it as this call's return value.
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}
