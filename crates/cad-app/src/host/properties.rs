//! Host-side property writes for the docked properties panel.
//!
//! The controller method here is the same command funnel every other edit uses
//! (`CommandId::SetEntityProperty` + `CommandPayload::EntityProperty`); it lives
//! in its own file only to keep `host/mod.rs` under the repository's file-size
//! limit.

use super::*;

impl HostController {
    /// Change one property on **every** currently selected entity (the docked
    /// panel's write path; `SETENTITYPROPERTY`).
    ///
    /// The caller supplies only `key` and `value` — never an entity id — so the
    /// host resolves the current selection itself and a stale UI cannot retarget
    /// another object. Accepted keys and their exact grammar are documented in
    /// [`crate::property_value`]: `layer` (an existing layer name), `color`
    /// (`ByLayer`, `ByBlock`, `aci:0..=256`, `#RRGGBB`), `linetype` (`ByLayer`,
    /// `ByBlock`, an existing linetype name), `lineweight` (`ByLayer`, `ByBlock`,
    /// `Default`, a canonical millimetre value) and `transparency` (integer
    /// `0..=90` percent). `text` is refused (`property.unsupported_key`): it has
    /// its own edit path.
    ///
    /// All refs are validated before anything is staged, so one non-addressable
    /// ref (block-instance member or sub-element) refuses the whole command with
    /// `property.not_addressable` and writes nothing. The changed entities are
    /// written in one transaction / one undo step; entities that already hold
    /// the value get no patch, and if nothing changes at all the command records
    /// no undo entry (`property.unchanged`). The selection is preserved across
    /// the edit.
    pub fn set_selection_property(&mut self, key: &str, value: &str) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::SetEntityProperty,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::EntityProperty {
                key: key.to_string(),
                value: value.to_string(),
            },
        })
    }
}
