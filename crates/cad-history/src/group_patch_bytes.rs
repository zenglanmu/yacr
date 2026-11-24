//! Byte accounting for the group patches carried by one undo step.
//!
//! Split out of `lib.rs` for the code-size gate. Validation lives beside the
//! step (`DrawingHistoryStep::validate`) because it also has to prove the step
//! carries no other write path.

use cad_domain::EntityId;

pub(crate) fn group_patch_bytes(patches: &[cad_db::GroupPatch]) -> usize {
    let mut bytes = 0;
    for patch in patches {
        bytes += std::mem::size_of::<cad_db::GroupPatch>();
        for row in patch.before.iter().chain(patch.after.iter()) {
            bytes += std::mem::size_of::<cad_db::GroupRow>()
                + row.name.len()
                + row.description.len()
                + row.members.len() * std::mem::size_of::<EntityId>();
        }
    }
    bytes
}
