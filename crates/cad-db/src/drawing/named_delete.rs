//! Single-purpose **delete** APIs for the three named-object tables a `PURGE`
//! has to be able to remove: block definitions, text styles and linetypes.
//!
//! Until this module existed the only post-import delete was
//! [`DrawingDatabase::delete_layer`], so `PURGE BLOCK|STYLE|LINETYPE` (and the
//! `RENAME`-style table writes) refused those kinds by name. Each entry point
//! below is modelled on `delete_layer`: validate first, refuse **by name** while
//! a surviving reference would dangle, then write through the existing
//! all-or-nothing batch so one call is one revision and one [`ChangeSet`].
//!
//! The reference rules are *not* re-invented here: the batch validators
//! (`validate_block_batch`, `validate_style_batch`, `validate_linetype_batch`) stay
//! the authority and still refuse a dangling reference if a caller slips one
//! through. The `ensure_*` predicates exist so a command can ask the same question
//! *before* staging — `PURGE` reports one refusal per named object instead of
//! aborting the whole invocation — and so the single-purpose delete and `PURGE`
//! cannot disagree about what "in use" means.
//!
//! # What each kind refuses
//!
//! * **block** — an `INSERT` anywhere outside the definition itself (its own
//!   members are deleted with the row, in the same batch). AutoCAD's block `0` is
//!   not modelled as a row, so there is no default-block exception.
//! * **style** — a text entity, a dimension or a multileader that names the style,
//!   and any DIMSTYLE/MLEADERSTYLE whose `text_style_name` resolves to it.
//! * **linetype** — a layer row's `DXF 6` name and an entity's explicit linetype
//!   name, both compared ASCII case-insensitively exactly like the rename
//!   cascade; `Continuous` is refused by name because it is the implicit value of
//!   every layer row without stored attributes.
//!
//! # Undo
//!
//! Nothing here records history: the caller keeps the before/after
//! [`BlockPatch`]/[`StylePatch`]/[`LinetypePatch`] rows (plus the deleted member
//! entities as ordinary drawing patches) in one `cad-history` step.

use cad_domain::*;

use super::style_edit::{entity_uses_text_style, StyleTableChanges};
use super::DrawingDatabase;
use crate::change::ChangeSet;
use crate::entity::{DbEntity, EntityLineType};
use crate::tables::DEFAULT_LINETYPE_NAME;

/// The text-style name AutoCAD always keeps; every drawing resolves it.
const DEFAULT_STYLE_NAME: &str = "Standard";

impl DrawingDatabase {
    /// Refuse a block delete that would leave a dangling `INSERT`.
    ///
    /// `Ok(())` means the row exists and nothing outside the definition
    /// references it. The definition's **own** members do not count: they are
    /// deleted together with the row by [`Self::delete_block`].
    pub fn ensure_block_deletable(&self, id: BlockId) -> CadResult<()> {
        let definition = self.blocks.get(&id).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "block.unknown_id: this drawing has no block definition with id {}",
                id.0
            ))
        })?;
        let name = self
            .block_metadata
            .get(&id)
            .map(|metadata| metadata.name.clone())
            .unwrap_or_else(|| format!("<unnamed {}>", id.0));
        let inserts = self
            .entities
            .values()
            .filter(|entity| entity.space != SpaceId::Block(id))
            .filter(|entity| {
                matches!(
                    &entity.geometry,
                    SemanticGeometry::Insert { block, .. } if *block == id
                )
            })
            .count();
        if inserts > 0 {
            return Err(CadError::InvalidInput(format!(
                "block.in_use: block '{}' ({}) is still referenced by {} INSERT(s); erase them \
                 first",
                name, id.0, inserts
            )));
        }
        let _ = definition;
        Ok(())
    }

    /// Delete one block definition **and its member entities** in one batch.
    ///
    /// See [`Self::ensure_block_deletable`] for the refusal; a refusal leaves the
    /// table, the members and the revision untouched.
    pub fn delete_block(
        &mut self,
        id: BlockId,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        self.ensure_block_deletable(id)?;
        let members: Vec<EntityId> = self
            .blocks
            .get(&id)
            .map(|definition| definition.entities.clone())
            .unwrap_or_default();
        let entity_changes: Vec<(EntityId, Option<DbEntity>)> =
            members.into_iter().map(|member| (member, None)).collect();
        self.apply_drawing_changes_with_blocks(reason, transaction, &[(id, None)], entity_changes)
    }

    /// Refuse a text-style delete that would leave a dangling reference.
    pub fn ensure_style_deletable(&self, id: StyleId) -> CadResult<()> {
        let style = self.styles.get(&id).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "style.unknown_id: this drawing has no text style with id {}",
                id.0
            ))
        })?;
        let name = style.name.trim();
        if name.eq_ignore_ascii_case(DEFAULT_STYLE_NAME) {
            return Err(CadError::InvalidInput(format!(
                "style.standard_undeletable: text style '{DEFAULT_STYLE_NAME}' is the drawing's \
                 default style and cannot be deleted"
            )));
        }
        let entities = self
            .entities
            .values()
            .filter(|entity| entity_uses_text_style(&entity.geometry, id))
            .count();
        if entities > 0 {
            return Err(CadError::InvalidInput(format!(
                "style.in_use: text style '{}' ({}) is still referenced by {entities} entit{}; \
                 retarget or delete them first",
                name,
                id.0,
                if entities == 1 { "y" } else { "ies" }
            )));
        }
        let named = self
            .dim_styles
            .values()
            .filter(|row| row.text_style_name.eq_ignore_ascii_case(name))
            .count()
            + self
                .mleader_styles
                .values()
                .filter(|row| row.text_style_name.eq_ignore_ascii_case(name))
                .count();
        if named > 0 {
            return Err(CadError::InvalidInput(format!(
                "style.in_use: text style '{}' ({}) is the DIMTXSTY/lead-style reference of \
                 {named} style row(s); retarget them first",
                name, id.0
            )));
        }
        Ok(())
    }

    /// Delete one text-style row in one batch.
    pub fn delete_style(
        &mut self,
        id: StyleId,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        self.ensure_style_deletable(id)?;
        let styles = [(id, None)];
        self.apply_drawing_changes_with_blocks_display_layers_and_layouts_and_styles(
            reason,
            transaction,
            &[],
            Vec::new(),
            &[],
            &[],
            &[],
            StyleTableChanges {
                styles: &styles,
                dim_styles: &[],
                mleader_styles: &[],
            },
        )
    }

    /// Refuse a linetype delete that would leave a dangling name.
    pub fn ensure_linetype_deletable(&self, id: LinetypeId) -> CadResult<()> {
        let row = self.linetypes.get(&id).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "linetype.unknown_id: this drawing has no linetype with id {}",
                id.0
            ))
        })?;
        let name = row.name.trim();
        if name.eq_ignore_ascii_case(DEFAULT_LINETYPE_NAME) {
            return Err(CadError::InvalidInput(format!(
                "linetype.continuous_undeletable: linetype '{DEFAULT_LINETYPE_NAME}' is the \
                 drawing's default linetype and the implicit value of every layer row with no \
                 stored attributes, so it cannot be deleted"
            )));
        }
        let layers = self
            .layer_attributes
            .values()
            .filter(|attributes| attributes.linetype.eq_ignore_ascii_case(name))
            .count();
        let entities = self
            .render_attributes
            .values()
            .filter(|attributes| {
                matches!(
                    &attributes.linetype,
                    EntityLineType::Explicit { name: explicit, .. }
                        if explicit.eq_ignore_ascii_case(name)
                )
            })
            .count();
        if layers + entities > 0 {
            return Err(CadError::InvalidInput(format!(
                "linetype.in_use: linetype '{}' ({}) is still referenced by {layers} layer row(s) \
                 and {entities} entity attribute(s); retarget them first",
                name, id.0
            )));
        }
        Ok(())
    }

    /// Delete one linetype row in one batch.
    pub fn delete_linetype(
        &mut self,
        id: LinetypeId,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        self.ensure_linetype_deletable(id)?;
        let rows = [(id, None)];
        self.apply_drawing_changes_with_blocks_display_layers_layouts_styles_and_linetypes(
            reason,
            transaction,
            &[],
            Vec::new(),
            &[],
            &[],
            &[],
            StyleTableChanges::default(),
            &rows,
        )
    }
}
