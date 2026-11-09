//! Wave 9 **command-line aliases** for the extended batch.
//!
//! These six names are the AutoCAD command-line spellings of commands this build
//! already implements. They are not a second implementation: each one calls the
//! owning handler with the caller's payload, so there is exactly one planner and
//! one write path per feature.
//!
//! * `-BLOCK` → `define_block` (`BLOCK`)
//! * `-INSERT` → `insert_block` (`INSERT`)
//! * `-BOUNDARY` → `create_boundary` (`BOUNDARY`)
//! * `-HATCH` → `create_hatch` (`HATCH`)
//! * `-HATCHEDIT` → `hatch_edit` (`HATCHEDIT`)
//! * `EXOFFSET` → `offset_entity` (`OFFSET`)
//!
//! Each call still commits exactly one transaction / one undo step through the
//! owning handler; the alias adds no write of its own.

use super::*;

impl Application {
    /// Dispatch one Wave 9 alias to its owning command handler.
    pub(crate) fn extended_w9_alias(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::DashBlock => self.define_block(session, command),
            ExtendedCommandId::DashInsert => self.insert_block(session, command),
            ExtendedCommandId::DashBoundary => self.create_boundary(session, command),
            ExtendedCommandId::DashHatch => self.create_hatch(session, command),
            ExtendedCommandId::DashHatchEdit => self.hatch_edit(session, command),
            ExtendedCommandId::ExOffset => self.offset_entity(command),
            other => Err(CadError::Invariant(format!(
                "not a Wave 9 alias: {}",
                other.selector()
            ))),
        }
    }
}
