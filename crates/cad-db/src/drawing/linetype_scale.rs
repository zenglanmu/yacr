//! The drawing-global linetype scale (`$LTSCALE`) post-import write.
//!
//! The importer records the value through
//! [`DrawingDatabaseBuilder::set_linetype_scale`](crate::DrawingDatabaseBuilder::set_linetype_scale).
//! This module is the one sanctioned **post-import** write, used by the typed
//! `LTSCALE <value>` command so the sysvar is settable at the command line.
//!
//! Contract (mirrors the builder's validation exactly):
//!
//! * **a positive finite value only** — non-finite, zero and negative values are
//!   an explicit [`CadError::InvalidInput`] refusal, never a clamp and never a
//!   silent reset to `1.0`. The render path multiplies dash patterns by this
//!   value, so a poisoned header would leak into every derived line.
//! * **one revision bump per real change** — writing the stored value is an
//!   authoritative no-op (`before == after`, no revision bump), the same rule
//!   [`DrawingDatabase::set_point_display_style`] follows.
//! * **no object changes** in the published [`ChangeSet`]: a header value is not
//!   an entity, so the raised revision is the only staleness tag.
//! * **not undoable** — `cad-history` has no header-variable patch kind, so the
//!   command layer records no history entry rather than fabricating one.

use cad_domain::*;

use super::DrawingDatabase;
use crate::change::ChangeSet;

impl DrawingDatabase {
    /// Store the drawing-global linetype scale (`$LTSCALE`).
    ///
    /// A non-finite or non-positive `scale` is refused ([`CadError::InvalidInput`])
    /// before anything is touched, so a refusal changes no value, no revision and
    /// no history. Writing the already stored value does not bump the revision.
    /// A real change is exactly one revision bump and an empty-object-changes
    /// [`ChangeSet`].
    pub fn set_linetype_scale(
        &mut self,
        scale: f64,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        // Phase 1: validate with the builder's one rule (positive finite).
        if !scale.is_finite() || scale <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "linetype scale must be a positive finite number, got {scale}"
            )));
        }
        let before = self.revision;
        // Phase 2: an authoritative no-op when nothing observable changes.
        if self.linetype_scale == scale {
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
            });
        }

        // Phase 3: mutate, then raise the revision exactly once. The bounds cache
        // is revision-keyed, so clearing it keeps every derived projection honest.
        self.linetype_scale = scale;
        self.invalidate_bounds_cache();
        let after = Revision(before.0 + 1);
        self.revision = after;
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
        })
    }
}
