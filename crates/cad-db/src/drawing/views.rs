//! Persisted named views (`tables/views.json`, `.ymodel` minor 18).
//!
//! The table is document content, not host metadata: it lives on
//! [`DrawingDatabase`], is written by [`DrawingDatabase::snapshot`] when it holds
//! a row, and is decoded by [`DrawingDatabase::from_snapshot`]. This module owns
//! the two write paths:
//!
//! * [`DrawingDatabase::plan_save_named_view`] / [`DrawingDatabase::plan_delete_named_view`]
//!   validate a command against the current table and return the reversible
//!   [`ViewPatch`] **before** anything mutates;
//! * [`DrawingDatabase::apply_named_view_patches`] applies one or more patches as
//!   **one** revision and one `tables/views.json` dirty mark. The command layer
//!   records the patch in history first and then applies it, so a refused step
//!   cannot leave the drawing changed.
//!
//! Undo/redo is the same replay a layer/table patch uses: the history step's
//! `before`/`after` sides are swapped and handed back to
//! [`DrawingDatabase::apply_named_view_patches`]. The camera and space are
//! document rows; the session's current viewport is not touched by undo/redo.

use cad_domain::view::{markers, NamedView, NamedViewTable};
use cad_domain::{CadError, CadResult, Revision, TransactionId};

use super::DrawingDatabase;
use crate::scope::TableKind;
use crate::{ChangeScope, ChangeSet};

/// A reversible named-view row.
///
/// `None` on either side means the row did not exist (a create) / no longer
/// exists (a delete). The name is always present so a patch is addressable even
/// when both sides are absent (a real caller never sends that shape).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewPatch {
    /// The row name (trimmed, exactly as stored).
    pub name: String,
    /// The row before the write.
    pub before: Option<NamedView>,
    /// The row after the write.
    pub after: Option<NamedView>,
}

/// What one planned write will change.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewWrite {
    /// The reversible row.
    pub patch: ViewPatch,
    /// Whether an existing view of the same name will be replaced.
    pub replaced: bool,
}

impl DrawingDatabase {
    /// The document's named views (empty when it has none).
    pub fn named_views(&self) -> &NamedViewTable {
        &self.named_views
    }

    /// Validate a `VIEW SAVE` and return the patch it would write.
    ///
    /// `None` means the table already holds exactly this row (a no-op: no
    /// revision, no history step). An invalid camera/name or a full table is a
    /// named refusal and nothing is changed.
    pub fn plan_save_named_view(&self, mut view: NamedView) -> CadResult<Option<ViewWrite>> {
        let name = view.name.trim().to_string();
        view.name = name.clone();
        view.validate()?;
        let before = self.named_views.resolve(&name).cloned();
        if before.as_ref() == Some(&view) {
            return Ok(None);
        }
        // A new name at the cap is refused here, before history is touched. A
        // replacement is always allowed.
        if before.is_none() && self.named_views.len() >= cad_domain::NAMED_VIEW_CAP {
            return Err(CadError::InvalidInput(format!(
                "{}: `{name}` would be view {}, but the table holds at most {} rows",
                markers::VIEW_CAP_EXCEEDED,
                self.named_views.len() + 1,
                cad_domain::NAMED_VIEW_CAP
            )));
        }
        Ok(Some(ViewWrite {
            patch: ViewPatch {
                name,
                before: before.clone(),
                after: Some(view),
            },
            replaced: before.is_some(),
        }))
    }

    /// Validate a `VIEW DELETE` and return the patch it would write.
    ///
    /// An unknown name is `view.unknown`; nothing is changed.
    pub fn plan_delete_named_view(&self, name: &str) -> CadResult<ViewWrite> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "{}: a named view needs a non-empty name",
                markers::VIEW_NAME_INVALID
            )));
        }
        let before = self
            .named_views
            .resolve(&name)
            .cloned()
            .ok_or_else(|| unknown_view(&name))?;
        let name = before.name.clone();
        Ok(ViewWrite {
            patch: ViewPatch {
                name,
                before: Some(before),
                after: None,
            },
            replaced: false,
        })
    }

    /// Apply named-view patches as **one** revision and one table dirty mark.
    ///
    /// Every patch is validated before anything mutates: a non-empty name, at
    /// least one side, every side a valid row, and no two patches naming the same
    /// view. The returned [`ChangeSet`] has no object changes; its scope marks
    /// `tables/views.json` dirty so an incremental save rewrites it.
    pub fn apply_named_view_patches(
        &mut self,
        patches: &[ViewPatch],
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        if patches.is_empty() {
            return Err(CadError::InvalidInput(
                "a named-view write needs at least one patch".into(),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for patch in patches {
            let name = patch.name.trim();
            if name.is_empty() {
                return Err(CadError::InvalidInput(format!(
                    "{}: a named-view patch has an empty name",
                    markers::VIEW_NAME_INVALID
                )));
            }
            if !seen.insert(name.to_ascii_lowercase()) {
                return Err(CadError::InvalidInput(format!(
                    "{}: `{name}` is patched more than once",
                    markers::VIEW_DUPLICATE_NAME
                )));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a named-view patch needs a before or after row".into(),
                ));
            }
            for row in patch.before.iter().chain(patch.after.iter()) {
                row.validate()?;
            }
        }

        // `plan_*` already refused an over-cap new name, and undo/redo can only
        // move a row that existed at that revision, so the cap check here is the
        // last line of defence, not a routine path.
        let mut next = self.named_views.clone();
        for patch in patches {
            match &patch.after {
                Some(after) => {
                    next.save(after.clone()).map_err(|error| {
                        CadError::InvalidInput(format!("{}: {error}", markers::VIEW_INVALID_CAMERA))
                    })?;
                }
                None => {
                    next.remove(&patch.name);
                }
            }
        }
        if next == self.named_views {
            return Err(CadError::InvalidInput(
                "a named-view patch would change nothing".into(),
            ));
        }

        let before = self.revision;
        self.named_views = next;
        let after = Revision(before.0 + 1);
        self.revision = after;
        let mut scope = ChangeScope::default();
        scope.table(TableKind::Views);
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

fn unknown_view(name: &str) -> CadError {
    CadError::InvalidInput(format!(
        "view.unknown: there is no saved view named `{name}` in this drawing"
    ))
}

impl crate::DrawingDatabaseBuilder {
    /// Install the decoded named-view table (a load path).
    ///
    /// The table was decoded by the snapshot codec, which already refused
    /// duplicates and an over-cap table; this method validates each row once more
    /// so a hand-built table cannot bypass the camera rules.
    pub fn set_named_views(&mut self, table: NamedViewTable) -> CadResult<()> {
        for view in table.views() {
            view.validate()?;
        }
        self.database.named_views = table;
        Ok(())
    }
}
