//! One-transaction writes for a **set** of drawing-header values.
//!
//! The per-value setters ([`DrawingDatabase::set_point_display_style`],
//! [`DrawingDatabase::set_linetype_scale`], [`DrawingDatabase::set_insertion_base`]
//! and [`DrawingDatabase::set_model_limits`]) each raise one revision. A command
//! that restores several values at once (`SYSVDLG RESTORE`) must not publish a
//! chain of half-restored revisions, so this module applies a validated patch
//! with **one** revision bump and one header-only [`ChangeSet`].
//!
//! The patch is additive: `None` means "leave this value alone". It cannot clear
//! `$INSBASE`/`$LIMMIN`/`$LIMMAX` back to "not stored" — there is no clear
//! syntax in the command that uses it, and a silent clear would be a different
//! user request.

use cad_domain::*;

use super::DrawingDatabase;
use crate::change::ChangeSet;
use crate::scope::ChangeScope;

/// A validated set of header values to write in one revision.
///
/// `None` leaves that value unchanged; a supplied pair is written exactly as
/// given after finiteness/positivity validation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeaderValuesPatch {
    /// `$PDMODE` / `$PDSIZE` as one pair (the display style is always one
    /// coherent pair; a caller that changes only one supplies the other's
    /// current value).
    pub point_display: Option<(i16, f64)>,
    /// `$LTSCALE` (positive finite).
    pub linetype_scale: Option<f64>,
    /// `$INSBASE` (finite point).
    pub insertion_base: Option<Point3>,
    /// `$LIMMIN` (finite corner).
    pub limits_min: Option<[f64; 2]>,
    /// `$LIMMAX` (finite corner).
    pub limits_max: Option<[f64; 2]>,
}

impl HeaderValuesPatch {
    /// Whether the patch asks for nothing at all.
    pub fn is_empty(&self) -> bool {
        self.point_display.is_none()
            && self.linetype_scale.is_none()
            && self.insertion_base.is_none()
            && self.limits_min.is_none()
            && self.limits_max.is_none()
    }
}

impl DrawingDatabase {
    /// Apply a validated header patch in exactly one revision.
    ///
    /// Phase 1 validates every supplied value before anything is touched (a
    /// refusal leaves the database exactly as it was). Phase 2 compares the
    /// patch with the stored values: an unchanged patch is an authoritative
    /// no-op (`before == after`, no revision bump). Phase 3 writes every value
    /// and raises the revision once, with an empty-object header-only change set.
    /// The write records no undo step: `cad-history` has no header patch kind.
    pub fn apply_header_values(
        &mut self,
        patch: HeaderValuesPatch,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        if let Some((_, size)) = patch.point_display {
            if !size.is_finite() {
                return Err(CadError::InvalidInput(format!(
                    "point display size must be a finite number, got {size}"
                )));
            }
        }
        if let Some(scale) = patch.linetype_scale {
            if !scale.is_finite() || scale <= 0.0 {
                return Err(CadError::InvalidInput(format!(
                    "linetype scale must be a positive finite number, got {scale}"
                )));
            }
        }
        if let Some(point) = patch.insertion_base {
            for (axis, value) in [("x", point.x), ("y", point.y), ("z", point.z)] {
                if !value.is_finite() {
                    return Err(CadError::InvalidInput(format!(
                        "the insertion base point has the non-finite {axis} coordinate {value}"
                    )));
                }
            }
        }
        for (label, corner) in [("$LIMMIN", patch.limits_min), ("$LIMMAX", patch.limits_max)] {
            if let Some([x, y]) = corner {
                for (axis, value) in [("x", x), ("y", y)] {
                    if !value.is_finite() {
                        return Err(CadError::InvalidInput(format!(
                            "{label} has the non-finite {axis} coordinate {value}"
                        )));
                    }
                }
            }
        }

        let before = self.revision;
        let unchanged = patch.point_display.is_none_or(|pair| {
            self.point_display_mode == pair.0 && self.point_display_size == pair.1
        }) && patch
            .linetype_scale
            .is_none_or(|scale| self.linetype_scale == scale)
            && patch
                .insertion_base
                .is_none_or(|point| self.insertion_base == Some(point))
            && patch
                .limits_min
                .is_none_or(|corner| self.limits_min == Some(corner))
            && patch
                .limits_max
                .is_none_or(|corner| self.limits_max == Some(corner));
        if unchanged {
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

        if let Some((mode, size)) = patch.point_display {
            self.point_display_mode = mode;
            self.point_display_size = size;
        }
        if let Some(scale) = patch.linetype_scale {
            self.linetype_scale = scale;
        }
        if let Some(point) = patch.insertion_base {
            self.insertion_base = Some(point);
        }
        if let Some(corner) = patch.limits_min {
            self.limits_min = Some(corner);
        }
        if let Some(corner) = patch.limits_max {
            self.limits_max = Some(corner);
        }
        self.invalidate_bounds_cache();
        let after = Revision(before.0 + 1);
        self.revision = after;
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
