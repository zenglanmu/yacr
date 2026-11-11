//! Post-import writes for the three named **style** tables.
//!
//! The importer fills `DB.STYLE`, `DB.DIMSTYLE` and `DB.MLEADERSTYLE` through
//! [`crate::DrawingDatabaseBuilder::insert_style`],
//! [`crate::DrawingDatabaseBuilder::insert_dim_style`] and
//! [`crate::DrawingDatabaseBuilder::insert_mleader_style`]. Until this module
//! existed those were the **only** writers: once the builder was finished the
//! tables were read-only, so `STYLE`, `DIMSTYLE`, `MLEADERSTYLE` and `RENAME`
//! could not change a name or a value. This module is the sanctioned post-import
//! path, mirroring [`super::layer_edit`] and [`super::layout_edit`].
//!
//! # One validated batch, one revision
//!
//! [`DrawingDatabase::apply_style_table_changes`] takes the three tables at
//! once. A rename that cascades (renaming text style `Standard` also retargets
//! the `DIMTXSTY`/`MLEADERSTYLE 342` name references in the same edit) must land
//! in one revision or a reader could observe a dangling name between the two
//! halves. The contract, in order:
//!
//! 1. **shape**: no duplicate id inside a batch, the key matches the row id, a
//!    patch has at least one side, and no patch is a no-op (`before == after`
//!    is refused: a caller that stages nothing must not bump the revision);
//! 2. **names**: trimmed, non-empty, free of the AutoCAD-illegal characters
//!    ([`super::layer_edit::ILLEGAL_LAYER_NAME_CHARS`]) and unique
//!    case-insensitively inside their own table's **final** name set (a name
//!    freed by a delete is available; names in different tables may coincide,
//!    because STYLE/DIMSTYLE/MLEADERSTYLE are separate symbol tables);
//! 3. **deletes**: the row must exist; a DIMSTYLE/MLEADERSTYLE named `Standard`
//!    is refused (dimension/leader creation resolves it), and a style that a
//!    dimension, multileader, text entity or another style row still references
//!    is refused by name instead of leaving a dangling reference;
//! 4. **reserved ids**: a style row may not be *created* with an id that still
//!    exists in the other side of the patch (an upsert of an existing id is an
//!    update, which is fine).
//!
//! Every refusal happens before any mutation, so a refused batch leaves the
//! tables, the name indexes, the allocators and the revision exactly as they
//! were.
//!
//! # What the change set carries
//!
//! Like the layer/layout writes, a style-table edit publishes a [`ChangeSet`]
//! with an **empty** object-change list and a raised revision. There is no
//! style object-change kind; the revision is the staleness tag the derived
//! consumers already bind to, and inventing an entity `Update` would claim
//! geometry changed when it did not.
//!
//! # These methods record no undo step themselves
//!
//! The command layer (`cad-app`) records the before/after rows in one
//! `cad-history` record, exactly like the layer commands, so create, rename,
//! delete and modify are one undo step each.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::*;

use super::layer_edit::ILLEGAL_LAYER_NAME_CHARS;
use super::DrawingDatabase;
use crate::change::ChangeSet;
use crate::dim_style::DimStyle;
use crate::entity::DbEntity;
use crate::mleader_style::MLeaderStyle;
use crate::tables::Style;

/// The canonical name of the DIMSTYLE/MLEADERSTYLE row creation resolves. It is
/// the one row this build refuses to delete.
pub const DEFAULT_DIM_STYLE_NAME: &str = "Standard";

/// A before/after record of one text-style row.
#[derive(Debug, Clone, PartialEq)]
pub struct StylePatch {
    pub id: StyleId,
    pub before: Option<Style>,
    pub after: Option<Style>,
}

/// A before/after record of one dimension-style row.
#[derive(Debug, Clone, PartialEq)]
pub struct DimStylePatch {
    pub id: DimStyleId,
    pub before: Option<DimStyle>,
    pub after: Option<DimStyle>,
}

/// A before/after record of one multileader-style row.
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderStylePatch {
    pub id: MLeaderStyleId,
    pub before: Option<MLeaderStyle>,
    pub after: Option<MLeaderStyle>,
}

/// The three style-table batches of one atomic write.
///
/// Grouping them keeps the combined write path readable and makes it impossible
/// to pass a cascade half-way (a text-style rename that retargets the dim-style
/// references must carry both batches in one call).
#[derive(Debug, Clone, Copy, Default)]
pub struct StyleTableChanges<'a> {
    pub styles: &'a [(StyleId, Option<Style>)],
    pub dim_styles: &'a [(DimStyleId, Option<DimStyle>)],
    pub mleader_styles: &'a [(MLeaderStyleId, Option<MLeaderStyle>)],
}

impl StyleTableChanges<'_> {
    /// Whether every batch is empty.
    pub fn is_empty(&self) -> bool {
        self.styles.is_empty() && self.dim_styles.is_empty() && self.mleader_styles.is_empty()
    }
}

impl DrawingDatabase {
    /// The next id a created text style will receive.
    pub fn next_style_id(&self) -> StyleId {
        StyleId(self.next_style_id)
    }

    /// The next id a created dimension style will receive.
    pub fn next_dim_style_id(&self) -> DimStyleId {
        DimStyleId(self.next_dim_style_id)
    }

    /// The next id a created multileader style will receive.
    pub fn next_mleader_style_id(&self) -> MLeaderStyleId {
        MLeaderStyleId(self.next_mleader_style_id)
    }

    /// The full combined write path plus the three style-table batches.
    ///
    /// See [`Self::apply_drawing_changes_combined`] for the entity/block/layer/
    /// layout contract and the module docs for the style batches. This wrapper
    /// exists so a `cad-history` record that mixes entity and style rows (for
    /// example a future style write that also retargets an entity) is one
    /// revision.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_drawing_changes_with_blocks_display_layers_and_layouts_and_styles(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        block_changes: &[(BlockId, Option<super::BlockRow>)],
        entity_changes: Vec<(EntityId, Option<DbEntity>)>,
        display_changes: &[(EntityId, Option<super::EntityDisplayState>)],
        layer_changes: &[(LayerId, Option<super::LayerRow>)],
        layout_changes: &[(LayoutId, Option<super::LayoutRow>)],
        style_changes: StyleTableChanges<'_>,
    ) -> CadResult<ChangeSet> {
        let display: Vec<(EntityId, super::EntityDisplayState)> = display_changes
            .iter()
            .filter_map(|(id, state)| state.clone().map(|state| (*id, state)))
            .collect();
        self.apply_drawing_changes_combined(
            reason,
            transaction,
            block_changes,
            entity_changes,
            &display,
            layer_changes,
            layout_changes,
            style_changes,
        )
    }

    /// Validate and apply one batch per style table in a single revision.
    ///
    /// See the module docs for the contract. An empty batch (all three tables)
    /// returns an unchanged [`ChangeSet`] without raising the revision.
    pub fn apply_style_table_changes(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        changes: StyleTableChanges<'_>,
    ) -> CadResult<ChangeSet> {
        let before = self.revision;
        if changes.is_empty() {
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
            });
        }

        // Phase 1: validate every batch before the first mutation.
        validate_style_batch(self, changes.styles)?;
        validate_dim_style_batch(self, changes.dim_styles)?;
        validate_mleader_style_batch(self, changes.mleader_styles)?;

        // Phase 2: upserts first, deletes last, so a row freed by the batch is
        // not observed half-applied.
        for (id, change) in changes.styles {
            if let Some(row) = change {
                self.install_style_row(*id, row);
            }
        }
        for (id, change) in changes.dim_styles {
            if let Some(row) = change {
                self.install_dim_style_row(*id, row);
            }
        }
        for (id, change) in changes.mleader_styles {
            if let Some(row) = change {
                self.install_mleader_style_row(*id, row);
            }
        }
        for (id, change) in changes.styles {
            if change.is_none() {
                self.remove_style_row(*id);
            }
        }
        for (id, change) in changes.dim_styles {
            if change.is_none() {
                self.remove_dim_style_row(*id);
            }
        }
        for (id, change) in changes.mleader_styles {
            if change.is_none() {
                self.remove_mleader_style_row(*id);
            }
        }

        let after = Revision(before.0 + 1);
        self.revision = after;
        // A style edit is not a geometry change, but text dimensions/metrics can
        // depend on the table (font keys, resolved DIMSTYLE values), so the
        // revision-keyed bounds memo is cleared explicitly.
        self.invalidate_bounds_cache();
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
        })
    }

    /// Validate the three style batches of a combined write.
    ///
    /// Reused by [`Self::apply_drawing_changes_combined`] so the combined path
    /// and [`Self::apply_style_table_changes`] cannot disagree about names or
    /// delete safety.
    pub(super) fn validate_style_batches_for_combined(
        &self,
        changes: StyleTableChanges<'_>,
    ) -> CadResult<()> {
        validate_style_batch(self, changes.styles)?;
        validate_dim_style_batch(self, changes.dim_styles)?;
        validate_mleader_style_batch(self, changes.mleader_styles)
    }

    /// Apply the three batches' upserts (creates/updates) in table order.
    pub(super) fn install_style_batches(&mut self, changes: StyleTableChanges<'_>) {
        for (id, change) in changes.styles {
            if let Some(row) = change {
                self.install_style_row(*id, row);
            }
        }
        for (id, change) in changes.dim_styles {
            if let Some(row) = change {
                self.install_dim_style_row(*id, row);
            }
        }
        for (id, change) in changes.mleader_styles {
            if let Some(row) = change {
                self.install_mleader_style_row(*id, row);
            }
        }
    }

    /// Apply the three batches' deletes (`None` entries) in table order.
    pub(super) fn remove_style_batches(&mut self, changes: StyleTableChanges<'_>) {
        for (id, change) in changes.styles {
            if change.is_none() {
                self.remove_style_row(*id);
            }
        }
        for (id, change) in changes.dim_styles {
            if change.is_none() {
                self.remove_dim_style_row(*id);
            }
        }
        for (id, change) in changes.mleader_styles {
            if change.is_none() {
                self.remove_mleader_style_row(*id);
            }
        }
    }

    /// Store one validated text-style row and keep its allocator ahead.
    pub(super) fn install_style_row(&mut self, id: StyleId, row: &Style) {
        let name = row.name.trim().to_string();
        self.styles.insert(
            id,
            Style {
                id,
                name,
                resource_keys: row.resource_keys.clone(),
            },
        );
        self.next_style_id = self.next_style_id.max(id.0.saturating_add(1));
    }

    fn remove_style_row(&mut self, id: StyleId) {
        self.styles.remove(&id);
        self.next_style_id = self.next_style_id.max(id.0.saturating_add(1));
    }

    /// Store one validated dimension-style row and keep both the exact-case
    /// name index and the allocator consistent.
    pub(super) fn install_dim_style_row(&mut self, id: DimStyleId, row: &DimStyle) {
        let mut row = row.clone();
        row.id = id;
        row.name = row.name.trim().to_string();
        if let Some(previous) = self.dim_styles.insert(id, row.clone()) {
            if previous.name != row.name {
                self.dim_style_ids_by_name.remove(&previous.name);
            }
        }
        self.dim_style_ids_by_name.insert(row.name, id);
        self.next_dim_style_id = self.next_dim_style_id.max(id.0.saturating_add(1));
    }

    fn remove_dim_style_row(&mut self, id: DimStyleId) {
        if let Some(removed) = self.dim_styles.remove(&id) {
            self.dim_style_ids_by_name.remove(&removed.name);
        }
        self.next_dim_style_id = self.next_dim_style_id.max(id.0.saturating_add(1));
    }

    /// Store one validated multileader-style row (same rules as the dim style).
    pub(super) fn install_mleader_style_row(&mut self, id: MLeaderStyleId, row: &MLeaderStyle) {
        let mut row = row.clone();
        row.id = id;
        row.name = row.name.trim().to_string();
        if let Some(previous) = self.mleader_styles.insert(id, row.clone()) {
            if previous.name != row.name {
                self.mleader_style_ids_by_name.remove(&previous.name);
            }
        }
        self.mleader_style_ids_by_name.insert(row.name, id);
        self.next_mleader_style_id = self.next_mleader_style_id.max(id.0.saturating_add(1));
    }

    fn remove_mleader_style_row(&mut self, id: MLeaderStyleId) {
        if let Some(removed) = self.mleader_styles.remove(&id) {
            self.mleader_style_ids_by_name.remove(&removed.name);
        }
        self.next_mleader_style_id = self.next_mleader_style_id.max(id.0.saturating_add(1));
    }
}

/// Trim and validate one style-table name.
///
/// `code_prefix` is the stable table key in the refusal (`style`, `dimstyle`,
/// `mleaderstyle`), so the caller can tell which table rejected the name.
pub(super) fn validated_style_name(name: &str, code_prefix: &str) -> CadResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CadError::InvalidInput(format!(
            "{code_prefix}.name_empty: a {code_prefix} name must not be empty or \
             whitespace-only"
        )));
    }
    if let Some(illegal) = trimmed
        .chars()
        .find(|c| ILLEGAL_LAYER_NAME_CHARS.contains(c))
    {
        return Err(CadError::InvalidInput(format!(
            "{code_prefix}.name_illegal_char: a {code_prefix} name must not contain '{illegal}'"
        )));
    }
    Ok(trimmed.to_string())
}

/// Validate the patch shape shared by the three tables.
///
/// Returns the touched ids so the caller can build the final name set.
fn validate_patch_shape<K, T>(
    table: &str,
    rows: &[(K, Option<T>)],
    key_matches: impl Fn(&K, &T) -> bool,
) -> CadResult<BTreeSet<K>>
where
    K: Copy + Ord + std::fmt::Debug,
{
    let mut seen = BTreeSet::new();
    let mut staged = BTreeSet::new();
    for (id, change) in rows {
        if !seen.insert(*id) {
            return Err(CadError::Invariant(format!(
                "transaction modifies {table} {id:?} twice"
            )));
        }
        if let Some(row) = change {
            if !key_matches(id, row) {
                return Err(CadError::Invariant(format!(
                    "change key {id:?} does not match the {table} row id"
                )));
            }
            staged.insert(*id);
        }
    }
    Ok(staged)
}

/// Validate the final name set of one table.
///
/// `existing` is the untouched rows' stored names; `staged` is the batch's
/// trimmed new names. Names are unique case-insensitively inside a table.
fn validate_name_set(table: &str, existing: Vec<String>, staged: Vec<String>) -> CadResult<()> {
    let mut final_names: BTreeMap<String, String> = BTreeMap::new();
    for name in existing.into_iter().chain(staged) {
        let folded = name.to_lowercase();
        if let Some(previous) = final_names.insert(folded, name.clone()) {
            return Err(CadError::InvalidInput(format!(
                "{table}.duplicate_name: '{previous}' and '{name}' differ only by case; \
                 {table} names are unique case-insensitively"
            )));
        }
    }
    Ok(())
}

fn validate_style_batch(db: &DrawingDatabase, rows: &[(StyleId, Option<Style>)]) -> CadResult<()> {
    let staged_ids = validate_patch_shape("style", rows, |id, row: &Style| row.id == *id)?;
    let mut existing = Vec::new();
    for style in db.styles.values() {
        if !staged_ids.contains(&style.id) {
            existing.push(style.name.clone());
        }
    }
    let mut staged = Vec::new();
    for (id, change) in rows {
        if let Some(row) = change {
            let name = validated_style_name(&row.name, "style")?;
            if db
                .styles
                .values()
                .any(|other| other.id != *id && other.name.to_lowercase() == name.to_lowercase())
            {
                return Err(CadError::InvalidInput(format!(
                    "style.duplicate_name: this drawing already has a text style named '{}'; \
                     names are unique case-insensitively",
                    row.name
                )));
            }
            staged.push(name);
        }
    }
    validate_name_set("style", existing, staged)?;
    // Deletes: a surviving reference would dangle the moment the row disappears.
    for (id, change) in rows {
        if change.is_some() {
            continue;
        }
        let row = db.styles.get(id).ok_or_else(|| {
            CadError::Invariant(format!(
                "style.unknown_id: cannot delete text style {id:?}; it does not exist"
            ))
        })?;
        let entities = db
            .entities
            .values()
            .filter(|entity| entity_uses_text_style(&entity.geometry, *id))
            .count();
        if entities > 0 {
            return Err(CadError::InvalidInput(format!(
                "style.in_use: text style '{}' ({}) is still referenced by {entities} entit{}; \
                 retarget or delete them first",
                row.name,
                id.0,
                if entities == 1 { "y" } else { "ies" }
            )));
        }
        let named = db
            .dim_styles
            .values()
            .filter(|style| style.text_style_name.eq_ignore_ascii_case(row.name.trim()))
            .count()
            + db.mleader_styles
                .values()
                .filter(|style| style.text_style_name.eq_ignore_ascii_case(row.name.trim()))
                .count();
        if named > 0 {
            return Err(CadError::InvalidInput(format!(
                "style.in_use: text style '{}' ({}) is the DIMTXSTY/lead-style reference of \
                 {named} style row(s); retarget them first",
                row.name, id.0
            )));
        }
    }
    Ok(())
}

fn validate_dim_style_batch(
    db: &DrawingDatabase,
    rows: &[(DimStyleId, Option<DimStyle>)],
) -> CadResult<()> {
    let staged_ids = validate_patch_shape("dimstyle", rows, |id, row: &DimStyle| row.id == *id)?;
    let mut existing = Vec::new();
    for style in db.dim_styles.values() {
        if !staged_ids.contains(&style.id) {
            existing.push(style.name.clone());
        }
    }
    let mut staged = Vec::new();
    for (id, change) in rows {
        if let Some(row) = change {
            let name = validated_style_name(&row.name, "dimstyle")?;
            if db
                .dim_styles
                .values()
                .any(|other| other.id != *id && other.name.to_lowercase() == name.to_lowercase())
            {
                return Err(CadError::InvalidInput(format!(
                    "dimstyle.duplicate_name: this drawing already has a dimension style named \
                     '{}'; names are unique case-insensitively",
                    row.name
                )));
            }
            staged.push(name);
        }
    }
    validate_name_set("dimstyle", existing, staged)?;
    for (id, change) in rows {
        if change.is_some() {
            continue;
        }
        let row = db.dim_styles.get(id).ok_or_else(|| {
            CadError::Invariant(format!(
                "dimstyle.unknown_id: cannot delete dimension style {id:?}; it does not exist"
            ))
        })?;
        if row.name == DEFAULT_DIM_STYLE_NAME {
            return Err(CadError::InvalidInput(format!(
                "dimstyle.standard_undeletable: dimension style '{DEFAULT_DIM_STYLE_NAME}' is \
                 the style dimension creation resolves and cannot be deleted"
            )));
        }
        let referencing = db
            .entities
            .values()
            .filter(|entity| match &entity.geometry {
                SemanticGeometry::Dimension(dimension) => {
                    dimension.style_name().eq_ignore_ascii_case(row.name.trim())
                }
                _ => false,
            })
            .count();
        if referencing > 0 {
            return Err(CadError::InvalidInput(format!(
                "dimstyle.in_use: dimension style '{}' ({}) is still referenced by {referencing} \
                 dimension(s); retarget or delete them first",
                row.name, id.0
            )));
        }
    }
    Ok(())
}

fn validate_mleader_style_batch(
    db: &DrawingDatabase,
    rows: &[(MLeaderStyleId, Option<MLeaderStyle>)],
) -> CadResult<()> {
    let staged_ids =
        validate_patch_shape("mleaderstyle", rows, |id, row: &MLeaderStyle| row.id == *id)?;
    let mut existing = Vec::new();
    for style in db.mleader_styles.values() {
        if !staged_ids.contains(&style.id) {
            existing.push(style.name.clone());
        }
    }
    let mut staged = Vec::new();
    for (id, change) in rows {
        if let Some(row) = change {
            let name = validated_style_name(&row.name, "mleaderstyle")?;
            if db
                .mleader_styles
                .values()
                .any(|other| other.id != *id && other.name.to_lowercase() == name.to_lowercase())
            {
                return Err(CadError::InvalidInput(format!(
                    "mleaderstyle.duplicate_name: this drawing already has a multileader style \
                     named '{}'; names are unique case-insensitively",
                    row.name
                )));
            }
            staged.push(name);
        }
    }
    validate_name_set("mleaderstyle", existing, staged)?;
    for (id, change) in rows {
        if change.is_some() {
            continue;
        }
        let row = db.mleader_styles.get(id).ok_or_else(|| {
            CadError::Invariant(format!(
                "mleaderstyle.unknown_id: cannot delete multileader style {id:?}; it does not \
                 exist"
            ))
        })?;
        if row.name == DEFAULT_DIM_STYLE_NAME {
            return Err(CadError::InvalidInput(format!(
                "mleaderstyle.standard_undeletable: multileader style '{DEFAULT_DIM_STYLE_NAME}' \
                 is the style MLEADER creation resolves and cannot be deleted"
            )));
        }
        let referencing = db
            .entities
            .values()
            .filter(|entity| match &entity.geometry {
                SemanticGeometry::MultiLeader(leader) => {
                    leader.style_name().eq_ignore_ascii_case(row.name.trim())
                }
                _ => false,
            })
            .count();
        if referencing > 0 {
            return Err(CadError::InvalidInput(format!(
                "mleaderstyle.in_use: multileader style '{}' ({}) is still referenced by \
                 {referencing} multileader(s); retarget or delete them first",
                row.name, id.0
            )));
        }
    }
    Ok(())
}

/// Whether an entity's geometry carries this text-style identity.
fn entity_uses_text_style(geometry: &SemanticGeometry, id: StyleId) -> bool {
    match geometry {
        SemanticGeometry::Text { style, .. } => *style == id,
        SemanticGeometry::Dimension(dimension) => dimension.style().text_style == id,
        SemanticGeometry::MultiLeader(leader) => leader.style().text_style == id,
        _ => false,
    }
}
