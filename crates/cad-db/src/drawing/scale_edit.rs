//! Post-import write for the drawing's **named annotation scales**
//! (`ACAD_SCALELIST`).
//!
//! The importer fills the table through
//! [`DrawingDatabaseBuilder::insert_scale`](crate::DrawingDatabaseBuilder::insert_scale)
//! and, until this module existed, that was the only writer: `SCALELISTEDIT`
//! could report the rows but every write was a named refusal. This module is the
//! sanctioned post-import path for the three writes AutoCAD's
//! `-SCALELISTEDIT` performs — **create**, **rename** and **delete**.
//!
//! # What a scale row is
//!
//! [`Scale`] is a complete row (`id`, `name`, `paper_units`, `drawing_units`),
//! unlike a layer or a block, which carry an attribute row beside them. So a
//! [`ScalePatch`] is just the before/after row pair and no second table has to be
//! kept in step.
//!
//! # Name rules
//!
//! A scale name is **not** a symbol-table name: AutoCAD's scale names are ratio
//! spellings such as `1:100` or `1" = 1'-0"`, so the layer/style illegal-character
//! set (which forbids `:` and `"`) must **not** be applied here. The rules are:
//!
//! * trimmed, non-empty, single line (no control characters);
//! * unique **case-insensitively** across the final table — the resolution rule
//!   [`DrawingDatabase::scale_by_name`] uses is exact-then-case-insensitive, so
//!   two rows whose names differ only by case would resolve ambiguously;
//! * a ratio whose two components are **finite and positive**. A zero or
//!   negative component cannot produce a usable factor, and
//!   [`Scale::factor`] deliberately falls back to `1.0` for imported data — a
//!   *new* row must not enter that fallback.
//!
//! # A rename cascades, and the database owns the cascade
//!
//! A scale **name** is stored in two other places:
//!
//! * [`ActiveAnnotationScale::name`] (the `CANNOSCALE` header value), the
//!   drawing-global active annotation scale;
//! * [`AnnotativeScaleOverride::scale`](crate::AnnotativeScaleOverride), the
//!   per-scale placement override an annotative entity carries for each imported
//!   context leaf.
//!
//! Renaming the row without retargeting both would leave dangling name
//! references that the exporter would write and a later read would resolve to
//! no scale (silently degrading to the header's fallback factor). The cascade is
//! therefore part of the same mutation as the row write, exactly like
//! `RENAME LINETYPE`'s layer/entity cascade. Every entity whose override was
//! retargeted is published as an `Update` with [`ChangeMask::STYLE`], so its
//! display representation is rebuilt.
//!
//! # A delete is refused while a name reference survives
//!
//! Both reference stores above are counted before the first mutation and the
//! delete is a named refusal (`scalelistedit.in_use`) when either is non-empty.
//!
//! There is deliberately **no** viewport or plot-settings reference check: this
//! build's [`PaperViewport`](crate::PaperViewport) stores a model↔paper
//! *transform* and [`PlotSettingsRecord`](crate::PlotSettingsRecord) stores the
//! raw `scale_numerator`/`scale_denominator` pair, so neither holds a scale
//! **name**. Deleting a row therefore cannot invalidate a viewport or a plot
//! setting, and the command layer says so instead of pretending to check them.
//!
//! # One revision, one `ChangeSet`
//!
//! The row write and the cascade raise the revision once and publish one
//! [`ChangeSet`]. Like the other header/table writes in this build, the module
//! records **no undo step**: `cad-history` has no scale-row patch kind, so the
//! command layer declares the write non-undoable rather than fabricating a
//! history entry that could not be replayed.

use cad_domain::*;

use super::DrawingDatabase;
use crate::change::{ChangeMask, ChangeSet, ObjectChange};
use crate::tables::Scale;

/// A before/after record of one scale-table row.
///
/// `None` means the row did not exist (`before`) / no longer exists (`after`).
/// The type is public so a caller (and a future `cad-history` patch kind) carries
/// the exact row shape instead of inventing a second one.
#[derive(Debug, Clone, PartialEq)]
pub struct ScalePatch {
    pub id: ScaleId,
    pub before: Option<Scale>,
    pub after: Option<Scale>,
}

impl DrawingDatabase {
    /// Allocate the next scale id.
    ///
    /// Monotonic like [`DrawingDatabase::allocate_layer_id`]: the builder
    /// initialises the counter from `max(stored id) + 1` and a delete never
    /// rewinds it, so a deleted id is never handed out again.
    pub fn allocate_scale_id(&mut self) -> ScaleId {
        let id = ScaleId(self.next_scale_id);
        self.next_scale_id = self.next_scale_id.saturating_add(1);
        id
    }

    /// Add one named scale to the table.
    ///
    /// Validates the name and the ratio (see the module docs), then installs the
    /// row, raises the revision once and publishes an empty-object [`ChangeSet`].
    /// A refusal changes nothing: no row, no revision, no allocation.
    pub fn create_scale(
        &mut self,
        name: &str,
        paper_units: f64,
        drawing_units: f64,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<(ChangeSet, Scale)> {
        let name = validated_scale_name(name)?;
        ensure_unique_scale_name(self, &name, None)?;
        validated_scale_ratio(&name, paper_units, drawing_units)?;
        let id = self.allocate_scale_id();
        let row = Scale {
            id,
            name,
            paper_units,
            drawing_units,
        };
        let before = self.revision;
        self.scales.insert(id, row.clone());
        let changeset = self.commit_scale_write(before, transaction, reason);
        Ok((changeset, row))
    }

    /// Rename the scale called `name` to `new_name`, retargeting every stored
    /// reference.
    ///
    /// The row keeps its id and ratio. The `CANNOSCALE` reference and every
    /// annotative entity override that names the old spelling are rewritten in
    /// the **same** mutation and the same revision; each retargeted entity is
    /// published as a `STYLE` update so its representation is rebuilt. Renaming
    /// to the stored spelling is an authoritative no-op (`before == after`, no
    /// revision bump).
    pub fn rename_scale(
        &mut self,
        name: &str,
        new_name: &str,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        let lookup = name.trim();
        let id = self
            .scale_id_by_name(lookup)
            .ok_or_else(|| unknown_scale(lookup))?;
        let stored = self
            .scales
            .get(&id)
            .cloned()
            .ok_or_else(|| unknown_scale(lookup))?;
        let new_name = validated_scale_name(new_name)?;
        if new_name == stored.name {
            let revision = self.revision;
            return Ok(ChangeSet {
                database: self.id,
                before: revision,
                after: revision,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
            });
        }
        ensure_unique_scale_name(self, &new_name, Some(id))?;
        let before = self.revision;
        let mut renamed = stored.clone();
        renamed.name = new_name.clone();
        self.scales.insert(id, renamed);
        let touched = self.cascade_scale_name(&stored.name, &new_name);
        let mut changeset = self.commit_scale_write(before, transaction, reason);
        changeset.changes = touched
            .into_iter()
            .map(|entity| ObjectChange::Update(ObjectId(entity.0), ChangeMask::STYLE))
            .collect();
        Ok(changeset)
    }

    /// Delete the scale called `name`.
    ///
    /// Refused by name (`scalelistedit.in_use`) while the `CANNOSCALE` value or
    /// any annotative override still references it; the refusal counts both so a
    /// caller can report what blocked it. A successful delete removes the row,
    /// raises the revision once and publishes an empty-object [`ChangeSet`].
    pub fn delete_scale(
        &mut self,
        name: &str,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        let lookup = name.trim();
        let id = self
            .scale_id_by_name(lookup)
            .ok_or_else(|| unknown_scale(lookup))?;
        let stored = self
            .scales
            .get(&id)
            .cloned()
            .ok_or_else(|| unknown_scale(lookup))?;
        let references = self.scale_reference_counts(&stored.name);
        if references.total() > 0 {
            return Err(CadError::InvalidInput(format!(
                "scalelistedit.in_use: scale '{}' ({}) is still referenced by {} (CANNOSCALE is \
                 '{}') and {} annotative entity override(s); rename or clear those references \
                 first",
                stored.name,
                id.0,
                references.annotation_scale,
                self.active_annotation_scale
                    .as_ref()
                    .map(|active| active.name.as_str())
                    .unwrap_or("unset"),
                references.entity_overrides
            )));
        }
        let before = self.revision;
        self.scales.remove(&id);
        Ok(self.commit_scale_write(before, transaction, reason))
    }

    /// How many stored name references point at `name`.
    ///
    /// The command layer uses this to report a delete refusal *before* it starts
    /// a write, and [`Self::delete_scale`] uses it as the write's own guard, so
    /// the pre-flight and the write can never disagree.
    pub fn scale_reference_counts(&self, name: &str) -> ScaleReferenceCounts {
        let name = name.trim();
        let matches = |reference: &str| reference.trim().eq_ignore_ascii_case(name);
        let annotation_scale = usize::from(
            self.active_annotation_scale
                .as_ref()
                .is_some_and(|active| matches(&active.name)),
        );
        let entity_overrides = self
            .render_attributes
            .values()
            .map(|attributes| {
                attributes
                    .annotative
                    .overrides
                    .iter()
                    .filter(|over| matches(&over.scale))
                    .count()
            })
            .sum();
        ScaleReferenceCounts {
            annotation_scale,
            entity_overrides,
        }
    }

    /// The row id whose name matches `name` exactly, else case-insensitively.
    fn scale_id_by_name(&self, name: &str) -> Option<ScaleId> {
        let name = name.trim();
        self.scales
            .values()
            .find(|scale| scale.name == name)
            .map(|scale| scale.id)
            .or_else(|| {
                self.scales
                    .values()
                    .find(|scale| scale.name.eq_ignore_ascii_case(name))
                    .map(|scale| scale.id)
            })
    }

    /// Retarget the `CANNOSCALE` value and every annotative override from `old`
    /// to `new`, returning the entity ids whose override changed.
    fn cascade_scale_name(&mut self, old: &str, new: &str) -> std::collections::BTreeSet<EntityId> {
        let mut touched = std::collections::BTreeSet::new();
        if let Some(active) = &mut self.active_annotation_scale {
            if active.name.eq_ignore_ascii_case(old) {
                active.name = new.to_string();
            }
        }
        for (id, attributes) in self.render_attributes.iter_mut() {
            let mut changed = false;
            for over in &mut attributes.annotative.overrides {
                if over.scale.eq_ignore_ascii_case(old) {
                    over.scale = new.to_string();
                    changed = true;
                }
            }
            if changed {
                touched.insert(*id);
            }
        }
        touched
    }

    /// Raise the revision once and publish the empty-object `ChangeSet` a scale
    /// table write produces.
    fn commit_scale_write(
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

/// How many stored references still name one scale row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScaleReferenceCounts {
    /// `1` when the drawing's `CANNOSCALE` header value names this scale.
    pub annotation_scale: usize,
    /// The number of annotative per-scale placement overrides naming it.
    pub entity_overrides: usize,
}

impl ScaleReferenceCounts {
    pub fn total(self) -> usize {
        self.annotation_scale + self.entity_overrides
    }
}

/// Validate one scale name (see the module docs for the rules).
fn validated_scale_name(name: &str) -> CadResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CadError::InvalidInput(
            "scalelistedit.name_empty: a scale name must not be empty or whitespace-only".into(),
        ));
    }
    if let Some(control) = trimmed.chars().find(|c| c.is_control()) {
        return Err(CadError::InvalidInput(format!(
            "scalelistedit.name_illegal_char: a scale name must be a single line; found the \
             control character {:?}",
            control.escape_default().to_string()
        )));
    }
    Ok(trimmed.to_string())
}

/// Validate one paper/drawing ratio for a **new** row.
fn validated_scale_ratio(name: &str, paper_units: f64, drawing_units: f64) -> CadResult<()> {
    for (label, value) in [
        ("paper units", paper_units),
        ("drawing units", drawing_units),
    ] {
        if !value.is_finite() {
            return Err(CadError::InvalidInput(format!(
                "scalelistedit.invalid_ratio: scale '{name}' has the non-finite {label} {value}"
            )));
        }
        if value <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "scalelistedit.invalid_ratio: scale '{name}' needs a positive {label}, got {value}"
            )));
        }
    }
    Ok(())
}

/// Refuse a name that another row in the final table already uses
/// (case-insensitively). `keep` is the id being renamed, which does not compete
/// with itself.
fn ensure_unique_scale_name(
    db: &DrawingDatabase,
    name: &str,
    keep: Option<ScaleId>,
) -> CadResult<()> {
    if let Some(existing) = db
        .scales
        .values()
        .find(|scale| Some(scale.id) != keep && scale.name.eq_ignore_ascii_case(name))
    {
        return Err(CadError::InvalidInput(format!(
            "scalelistedit.duplicate_name: a scale named '{}' already exists (as '{}'); scale \
             names are unique case-insensitively",
            name, existing.name
        )));
    }
    Ok(())
}

fn unknown_scale(name: &str) -> CadError {
    CadError::InvalidInput(format!(
        "scalelistedit.unknown_scale: no scale named '{}' is in the drawing's scale list; the \
         bare selector lists every stored row",
        name
    ))
}
