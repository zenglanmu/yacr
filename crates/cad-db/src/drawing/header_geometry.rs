//! Post-import writes for the drawing header values that have a **vector**
//! payload: the insertion base point (`$INSBASE`) and the model-space limits
//! (`$LIMMIN` / `$LIMMAX`).
//!
//! The other header writers in this build are
//! [`crate::DrawingDatabaseBuilder::set_linetype_scale`] /
//! [`DrawingDatabase::set_linetype_scale`] (`$LTSCALE`) and
//! [`crate::DrawingDatabaseBuilder::set_point_display_style`] (`$PDMODE` /
//! `$PDSIZE`). This module follows exactly the same contract:
//!
//! * the database stores the value **raw** — no clamping, no unit conversion and
//!   no "close enough" snapping;
//! * a **non-finite** component is an explicit [`CadError::InvalidInput`]
//!   refusal (a `NaN` coordinate would poison every consumer of the header and
//!   cannot be written to DXF), never a silent reset;
//! * storing the value that is already there is an authoritative **no-op**
//!   (`before == after`, no revision bump);
//! * a real change is exactly **one** revision bump and an empty-object
//!   [`ChangeSet`] — a header value is not an entity;
//! * **no undo step**: `cad-history` has no header-variable patch kind, so the
//!   command layer records none rather than fabricating one it could not replay.
//!
//! # Why the values are `Option`
//!
//! Both are stored as an explicit `None` until something writes them. `None`
//! means "this database carries no such header value": the importer of this build
//! does not read `$INSBASE`/`$LIMMIN`/`$LIMMAX` from a source file, so a
//! fabricated `(0,0,0)` would be indistinguishable from a real origin base point.
//! The readers [`DrawingDatabase::insertion_base`] /
//! [`DrawingDatabase::limits_min`] / [`DrawingDatabase::limits_max`] therefore
//! report `None` and every command that prints the value names the absence
//! (`base.not_stored` / `limits.not_stored`) instead of printing a default.
//!
//! # The limits are two independent header variables
//!
//! `$LIMMIN` and `$LIMMAX` are separate values in DXF, and AutoCAD lets either be
//! set on its own, so [`DrawingDatabase::set_model_limits`] takes each corner
//! independently (`None` = leave that corner alone). [`DrawingDatabase::model_limits`]
//! returns the pair only when **both** corners are stored: a caller that needs a
//! rectangle must handle the half-stored state explicitly instead of receiving a
//! made-up second corner. The exporter declares that state in its report.

use cad_domain::*;

use super::DrawingDatabase;
use crate::change::ChangeSet;

impl DrawingDatabase {
    /// The stored insertion base point (`$INSBASE`), or `None` when this
    /// database carries no value (see the module docs).
    pub fn insertion_base(&self) -> Option<Point3> {
        self.insertion_base
    }

    /// Store the model-space insertion base point (`$INSBASE`).
    ///
    /// A non-finite component is refused before anything is touched. Writing the
    /// already stored point does not bump the revision.
    pub fn set_insertion_base(
        &mut self,
        base: Point3,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        for (label, value) in [("x", base.x), ("y", base.y), ("z", base.z)] {
            if !value.is_finite() {
                return Err(CadError::InvalidInput(format!(
                    "base.invalid_point: the insertion base point has the non-finite {label} \
                     coordinate {value}; nothing was written"
                )));
            }
        }
        let before = self.revision;
        if self.insertion_base == Some(base) {
            return Ok(self.header_no_op(before, transaction, reason));
        }
        self.insertion_base = Some(base);
        Ok(self.header_changed(before, transaction, reason))
    }

    /// The stored `$LIMMIN` corner, or `None` when this database carries none.
    pub fn limits_min(&self) -> Option<[f64; 2]> {
        self.limits_min
    }

    /// The stored `$LIMMAX` corner, or `None` when this database carries none.
    pub fn limits_max(&self) -> Option<[f64; 2]> {
        self.limits_max
    }

    /// Both limit corners, in `(min, max)` order, only when **both** are stored.
    pub fn model_limits(&self) -> Option<([f64; 2], [f64; 2])> {
        match (self.limits_min, self.limits_max) {
            (Some(min), Some(max)) => Some((min, max)),
            _ => None,
        }
    }

    /// Store one or both model-space limit corners (`$LIMMIN` / `$LIMMAX`).
    ///
    /// `None` leaves that corner exactly as it is, so one `LIMITS` command can
    /// write both corners in **one** revision while `SETVAR LIMMIN` writes only
    /// the one it names. A non-finite component is refused before anything is
    /// touched; writing the values that are already stored does not bump the
    /// revision. The corners are stored as given: this build models no
    /// `LIMCHECK` enforcement, so no ordering or area rule is invented here (the
    /// command layer refuses a degenerate rectangle by name).
    pub fn set_model_limits(
        &mut self,
        min: Option<[f64; 2]>,
        max: Option<[f64; 2]>,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        if min.is_none() && max.is_none() {
            return Err(CadError::InvalidInput(
                "limits.no_corners: set_model_limits needs at least one corner".into(),
            ));
        }
        for (corner, label) in [(&min, "$LIMMIN"), (&max, "$LIMMAX")] {
            if let Some([x, y]) = corner {
                for (axis, value) in [("x", *x), ("y", *y)] {
                    if !value.is_finite() {
                        return Err(CadError::InvalidInput(format!(
                            "limits.invalid_point: {label} has the non-finite {axis} coordinate \
                             {value}; nothing was written"
                        )));
                    }
                }
            }
        }
        let before = self.revision;
        // A corner that is not supplied never competes: only the supplied ones
        // are compared with the stored value.
        let unchanged = min.is_none_or(|value| self.limits_min == Some(value))
            && max.is_none_or(|value| self.limits_max == Some(value));
        if unchanged {
            return Ok(self.header_no_op(before, transaction, reason));
        }
        if let Some(min) = min {
            self.limits_min = Some(min);
        }
        if let Some(max) = max {
            self.limits_max = Some(max);
        }
        Ok(self.header_changed(before, transaction, reason))
    }

    /// A header write that stored the value already present: no revision bump.
    fn header_no_op(
        &self,
        revision: Revision,
        transaction: TransactionId,
        reason: &str,
    ) -> ChangeSet {
        ChangeSet {
            database: self.id,
            before: revision,
            after: revision,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
        }
    }

    /// A header write that changed the stored value: one revision bump, no
    /// object changes.
    fn header_changed(
        &mut self,
        before: Revision,
        transaction: TransactionId,
        reason: &str,
    ) -> ChangeSet {
        self.invalidate_bounds_cache();
        let after = Revision(before.0 + 1);
        self.revision = after;
        ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
        }
    }
}
