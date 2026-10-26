//! The drawing-global POINT display style (`$PDMODE` / `$PDSIZE`) write.
//!
//! The importer records both header values through
//! [`DrawingDatabaseBuilder::set_point_display_mode`](crate::DrawingDatabaseBuilder::set_point_display_mode)
//! and [`DrawingDatabaseBuilder::set_point_display_size`](crate::DrawingDatabaseBuilder::set_point_display_size).
//! This module is the one sanctioned **post-import** write, used by the typed
//! `PDMODE <n>` / `PDSIZE <n>` commands so the style is settable at the command
//! line and no longer only at import.
//!
//! # What it enforces
//!
//! * **a finite size only**: a non-finite `$PDSIZE` is an explicit
//!   [`CadError::InvalidInput`] refusal. The builder keeps whatever a *source
//!   file* declared (a corrupt header stays observable); a *user write* never
//!   stores a value the display layer would have to sanitise.
//! * **the values stay raw**: the mode is an `i16` and the size an `f64`, stored
//!   exactly as the caller supplied them. Interpreting the mode bits, the size
//!   sign and the sanitisation rule is the display layer's single
//!   responsibility, so this write never rewrites or clamps a value.
//! * **one revision bump per real change**: writing the pair that is already
//!   stored is an authoritative no-op — `before == after` in the change set, no
//!   revision bump — the same rule [`DrawingDatabase::set_layer_attributes`]
//!   follows. The bump is what invalidates the derived projections: the render
//!   scene caches the POINT display style per base revision
//!   (`cad_app::render_scene`), so a new revision is exactly the refresh signal.
//! * **no object changes** in the published [`ChangeSet`]: [`ObjectChange`]
//!   addresses entities, and a header value is not an entity. The raised
//!   revision is the staleness tag; inventing an entity change would claim
//!   geometry moved when it did not.
//!
//! # This write is NOT undoable
//!
//! `cad-history` records entity patches only ([`cad_history::DrawingPatch`] is
//! `{ EntityId, before/after DbEntity }`), so there is no header-variable patch
//! kind to record. The command layer records **no** history entry for it rather
//! than fabricating a patch that would restore nothing.

use cad_domain::*;

use super::DrawingDatabase;
use crate::change::ChangeSet;

impl DrawingDatabase {
    /// Store the drawing-global POINT display style (`$PDMODE` / `$PDSIZE`).
    ///
    /// `mode` is the raw `$PDMODE` value (AutoCAD's base glyphs `0..=4` plus the
    /// `32`/`64`/`96` surrounding-shape bits, and any other value a caller
    /// supplies — the display layer interprets it and reports what it cannot
    /// express). `size` is the raw `$PDSIZE` value: `> 0` an absolute drawing-unit
    /// size, `0` viewport-relative (the DWG default) and `< 0` a percentage of the
    /// viewport height.
    ///
    /// Refuses a non-finite `size` ([`CadError::InvalidInput`]) before touching
    /// anything, so a refusal changes no value, no revision and no history.
    /// Writing the already stored pair does not bump the revision. A real change
    /// is exactly one revision bump and an empty-object-changes [`ChangeSet`].
    pub fn set_point_display_style(
        &mut self,
        mode: i16,
        size: f64,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        // Phase 1: validate. This write owns exactly one value rule (finiteness);
        // the mode's meaning belongs to the display layer, never to the db.
        if !size.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "point display size must be a finite number, got {size}"
            )));
        }
        let before = self.revision;
        // Phase 2: an authoritative no-op when nothing observable changes.
        if self.point_display_mode == mode && self.point_display_size == size {
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
            });
        }

        // Phase 3: mutate both header fields together, then raise the revision
        // exactly once — no observer can see the mode and the size disagree.
        self.point_display_mode = mode;
        self.point_display_size = size;
        // Defensive, exactly like `publish_layer_change`: every mutation clears
        // the bounds cache so no stale projection can outlive a revision.
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
