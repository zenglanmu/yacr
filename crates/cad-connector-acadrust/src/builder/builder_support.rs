//! Dynamic-block visibility attachment for [`ImporterBuilder`].
//!
//! Split out of `builder.rs` to keep that file inside the 1000-line gate.
//! The code is moved verbatim; no behavior changed.

use crate::*;

impl<'a> ImporterBuilder<'a> {
    /// Attach a dynamic visibility descriptor to an imported block, when the
    /// source carries an `AcDbBlockVisibilityParameter` for its definition.
    ///
    /// The descriptor is optional: a block with no visibility parameter is left
    /// untouched. A parameter that cannot be mapped exactly (dangling member
    /// handles, or an active state that cannot be resolved from the members'
    /// visibility flags) is recorded with a stable `Partial` reason and an
    /// explicit diagnostic; the active state is never guessed.
    pub(crate) fn record_dynamic_visibility(
        &mut self,
        def_handle: acadrust::Handle,
        id: BlockId,
        member_ids: &BTreeMap<u64, EntityId>,
        member_visible: &BTreeMap<u64, bool>,
    ) {
        let Some(param) = self.acad.block_visibility_param_for_def(def_handle) else {
            return;
        };
        self.attach_dynamic_visibility(id, param, member_ids, member_visible);
    }

    /// Attach a dynamic visibility descriptor resolved through an INSERT.
    ///
    /// A real evaluated dynamic block stores the visibility parameter on the
    /// dynamic *definition*, reachable from the INSERT through an
    /// `AcDbBlockRepresentationData` object. The INSERT expands the block record
    /// it references (the evaluated anonymous block), whose members the
    /// parameter governs, so the descriptor is mapped against that block's
    /// members. Blocks are always read before entity space, so the member maps
    /// are available here.
    pub(crate) fn record_insert_dynamic_visibility(&mut self, insert: &acadrust::entities::Insert) {
        let insert_handle = insert.common.handle;
        let Some((_def_block, param)) = self.acad.dynamic_visibility_for_insert(insert_handle)
        else {
            return;
        };
        let Some(id) = self.block_ids.get(&insert.block_name).copied() else {
            return;
        };
        // Resolve with an immutable borrow of the stored member maps, then
        // release it before recording (which needs `&mut self`).
        let mapped = {
            let (Some(member_ids), Some(member_visible)) = (
                self.block_member_ids.get(&id),
                self.block_member_visible.get(&id),
            ) else {
                return;
            };
            map_visibility(param, member_ids, member_visible)
        };
        self.attach_mapped(id, mapped);
    }

    /// Map a resolved parameter onto a block and record it, reporting any
    /// partial mapping as an explicit diagnostic.
    fn attach_dynamic_visibility(
        &mut self,
        id: BlockId,
        param: &acadrust::objects::BlockVisibilityParameter,
        member_ids: &BTreeMap<u64, EntityId>,
        member_visible: &BTreeMap<u64, bool>,
    ) {
        let mapped = map_visibility(param, member_ids, member_visible);
        self.attach_mapped(id, mapped);
    }

    fn attach_mapped(&mut self, id: BlockId, mapped: MappedVisibility) {
        if let Completeness::Partial(reasons) = &mapped.completeness {
            self.diagnostics.push(Diagnostic {
                object: None,
                code: "import.dynamic_block_visibility".into(),
                message: format!(
                    "block {:?} dynamic visibility is partial: {}",
                    id,
                    reasons.join(", ")
                ),
            });
        }
        if let Err(e) = self
            .builder
            .set_block_dynamic_visibility(id, mapped.descriptor)
        {
            self.diagnostics.push(Diagnostic {
                object: None,
                code: "import.dynamic_block_failed".into(),
                message: e.to_string(),
            });
        }
    }
}
