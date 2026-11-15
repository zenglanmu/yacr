//! `PURGE` / `-PURGE`: remove unused named objects through the sanctioned delete
//! paths (split out of `app_utility` so that file stays inside the code-size
//! gate).
//!
//! Layer rows go through the ordinary `DELETELAYER` command path, so PURGE keeps
//! its documented non-undoable layer behaviour. Block definitions, text styles and
//! linetypes go through the database's single-purpose delete APIs and are staged
//! as **one batch / one transaction / one undo step** per invocation, so undo
//! restores every purged row and every purged block member. Each in-use object is
//! a named per-object refusal that removes nothing.

use super::*;

use crate::app_drawing::next_transaction_id;
use crate::app_utility::{document_drawing, purge_table_kinds, with_payload};
use cad_db::DEFAULT_LAYER_NAME;
use cad_history::DrawingUndoRecord;

impl Application {
    /// `PURGE` / `-PURGE`: remove unused named objects through the sanctioned
    /// delete paths.
    ///
    /// Layer rows go through [`Self::layer_command_without_history`] with the
    /// ordinary `DELETELAYER` payload, so the active-layer fallback and override
    /// cleanup are the existing ones while PURGE keeps its documented
    /// non-undoable layer behaviour (it is out of scope for the one-step layer
    /// lane). Each non-default layer is reported as removed or refused (an in-use
    /// layer is refused by the database).
    ///
    /// Block definitions, text styles and linetypes are removed through the
    /// database's single-purpose delete APIs, staged as **one batch** so a single
    /// invocation is one transaction / **one undo step** (undo restores the rows
    /// and every purged block member). An in-use row is a named per-object
    /// refusal that removes nothing:
    ///
    /// | kind | refusal |
    /// | --- | --- |
    /// | block | `block.in_use` — an `INSERT` outside the definition |
    /// | style | `style.in_use` (entity / DIMSTYLE / MLEADERSTYLE) or `style.standard_undeletable` |
    /// | linetype | `linetype.in_use` (layer row / entity name) or `linetype.continuous_undeletable` |
    ///
    /// **Declared limits**: an unnamed block-definition row (import-only, not
    /// addressable by name) is not a purge candidate; and a block freed only by
    /// this same invocation deleting the block that referenced it stays until the
    /// next `PURGE` (AutoCAD's *Purge nested items* is not modelled).
    pub(crate) fn purge(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let target = match &command.payload {
            CommandPayload::None => PurgeTarget::All,
            CommandPayload::Purge(target) => *target,
            _ => {
                return Err(CadError::InvalidInput(
                    "PURGE takes no payload or one named kind".into(),
                ))
            }
        };
        let mut diagnostics = Vec::new();
        let mut changes: Option<Arc<ChangeSet>> = None;

        // 1. Layer rows: unchanged (the `DELETELAYER` path, no history).
        let mut layers_removed = 0usize;
        if matches!(target, PurgeTarget::All | PurgeTarget::Layer) {
            let drawing = document_drawing(self, command)?;
            let names: Vec<String> = drawing
                .layers()
                .filter(|layer| layer.name != DEFAULT_LAYER_NAME)
                .map(|layer| layer.name.clone())
                .collect();
            for name in names {
                let inner = with_payload(
                    command,
                    CommandId::DeleteLayer,
                    CommandPayload::DeleteLayer {
                        layer: name.clone(),
                    },
                );
                match self.layer_command_without_history(session, &inner) {
                    Ok(outcome) => {
                        layers_removed += 1;
                        if outcome.changes.is_some() {
                            changes = outcome.changes;
                        }
                        diagnostics.extend(outcome.diagnostics);
                        diagnostics.push(Diagnostic {
                            object: None,
                            code: "purge.layer_removed".into(),
                            message: format!("PURGE: removed unused layer '{name}'"),
                        });
                    }
                    Err(error) => diagnostics.push(Diagnostic {
                        object: None,
                        code: "purge.layer_refused".into(),
                        message: format!("PURGE: layer '{name}' was not removed: {error}"),
                    }),
                }
            }
        }

        // 2. Block / style / linetype tables: one batch, one undo step.
        let kinds = purge_table_kinds(target);
        let mut tables_removed = 0usize;
        if !kinds.is_empty() {
            let (removed, table_changes, table_diagnostics) =
                self.purge_named_tables(command, &kinds)?;
            tables_removed = removed;
            if table_changes.is_some() {
                changes = table_changes;
            }
            diagnostics.extend(table_diagnostics);
        }

        diagnostics.push(Diagnostic {
            object: None,
            code: if layers_removed + tables_removed == 0 {
                "purge.none".into()
            } else {
                "purge.summary".into()
            },
            message: format!(
                "PURGE: {layers_removed} unused layer row(s) removed (layer deletions are not \
                 undoable) and {tables_removed} unused block/style/linetype row(s) removed in one \
                 undo step"
            ),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// The block/style/linetype half of `PURGE`: one batch, one undo step.
    ///
    /// Eligibility is asked of the database (`ensure_*_deletable`) **before** the
    /// batch is built, so an in-use row becomes a named per-object diagnostic
    /// instead of aborting the whole invocation. The batch validators still run
    /// inside the write, so a stale eligibility answer refuses the whole write and
    /// changes nothing.
    fn purge_named_tables(
        &mut self,
        command: &Command,
        kinds: &[NamedObjectKind],
    ) -> CadResult<(usize, Option<Arc<ChangeSet>>, Vec<Diagnostic>)> {
        use cad_db::{BlockPatch, BlockRow, StylePatch, StyleTableChanges};

        let drawing = document_drawing(self, command)?.as_ref().clone();
        let mut diagnostics = Vec::new();
        let mut block_changes: Vec<(cad_domain::BlockId, Option<BlockRow>)> = Vec::new();
        let mut style_changes: Vec<(cad_domain::StyleId, Option<cad_db::Style>)> = Vec::new();
        let mut linetype_changes: Vec<(cad_domain::LinetypeId, Option<cad_db::LineType>)> =
            Vec::new();
        let mut entity_changes: Vec<(cad_domain::EntityId, Option<cad_db::DbEntity>)> = Vec::new();
        let mut block_patches = Vec::new();
        let mut style_patches = Vec::new();
        let mut linetype_patches = Vec::new();
        let mut member_patches = Vec::new();
        let mut removed = 0usize;

        if kinds.contains(&NamedObjectKind::Block) {
            let named: Vec<(cad_domain::BlockId, String)> = drawing
                .block_metadata_entries()
                .map(|(id, metadata)| (id, metadata.name.clone()))
                .collect();
            for (id, name) in named {
                match drawing.ensure_block_deletable(id) {
                    Ok(()) => {
                        let Some(definition) = drawing.block(id).cloned() else {
                            continue;
                        };
                        for member in &definition.entities {
                            let Some(before) = drawing.entity(*member).cloned() else {
                                continue;
                            };
                            member_patches.push(cad_history::drawing_patch_deleted(
                                *member,
                                before,
                                drawing.entity_display_state(*member),
                            ));
                            entity_changes.push((*member, None));
                        }
                        block_patches.push(BlockPatch {
                            id,
                            before: Some(BlockRow {
                                definition,
                                metadata: drawing.block_metadata(id).cloned(),
                            }),
                            after: None,
                        });
                        block_changes.push((id, None));
                        removed += 1;
                        diagnostics.push(Diagnostic {
                            object: None,
                            code: "purge.block_removed".into(),
                            message: format!("PURGE: removed unused block definition '{name}'"),
                        });
                    }
                    Err(error) => diagnostics.push(Diagnostic {
                        object: None,
                        code: "purge.block_refused".into(),
                        message: format!("PURGE: block '{name}' was not removed: {error}"),
                    }),
                }
            }
        }

        if kinds.contains(&NamedObjectKind::Style) {
            let styles: Vec<(cad_domain::StyleId, String)> = drawing
                .styles()
                .map(|row| (row.id, row.name.clone()))
                .collect();
            for (id, name) in styles {
                match drawing.ensure_style_deletable(id) {
                    Ok(()) => {
                        style_patches.push(StylePatch {
                            id,
                            before: drawing.style(id).cloned(),
                            after: None,
                        });
                        style_changes.push((id, None));
                        removed += 1;
                        diagnostics.push(Diagnostic {
                            object: None,
                            code: "purge.style_removed".into(),
                            message: format!("PURGE: removed unused text style '{name}'"),
                        });
                    }
                    Err(error) => diagnostics.push(Diagnostic {
                        object: None,
                        code: "purge.style_refused".into(),
                        message: format!("PURGE: text style '{name}' was not removed: {error}"),
                    }),
                }
            }
        }

        if kinds.contains(&NamedObjectKind::Linetype) {
            let linetypes: Vec<(cad_domain::LinetypeId, String)> = drawing
                .linetypes()
                .map(|row| (row.id, row.name.clone()))
                .collect();
            for (id, name) in linetypes {
                match drawing.ensure_linetype_deletable(id) {
                    Ok(()) => {
                        linetype_patches.push(cad_history::linetype_patch(
                            id,
                            drawing.linetype(id).cloned(),
                            None,
                        ));
                        linetype_changes.push((id, None));
                        removed += 1;
                        diagnostics.push(Diagnostic {
                            object: None,
                            code: "purge.linetype_removed".into(),
                            message: format!("PURGE: removed unused linetype '{name}'"),
                        });
                    }
                    Err(error) => diagnostics.push(Diagnostic {
                        object: None,
                        code: "purge.linetype_refused".into(),
                        message: format!("PURGE: linetype '{name}' was not removed: {error}"),
                    }),
                }
            }
        }

        if block_changes.is_empty() && style_changes.is_empty() && linetype_changes.is_empty() {
            diagnostics.push(Diagnostic {
                object: None,
                code: "purge.tables_none".into(),
                message: "PURGE: no unused block/style/linetype row could be removed".into(),
            });
            return Ok((0, None, diagnostics));
        }

        let mut candidate = drawing;
        let transaction = next_transaction_id();
        let change_set = candidate
            .apply_drawing_changes_with_blocks_display_layers_layouts_styles_and_linetypes(
                "purge named tables",
                transaction,
                &block_changes,
                entity_changes,
                &[],
                &[],
                &[],
                StyleTableChanges {
                    styles: &style_changes,
                    dim_styles: &[],
                    mleader_styles: &[],
                },
                &linetype_changes,
            )?;
        self.publish_candidate_with_linetypes(
            command.document,
            candidate,
            DrawingUndoRecord {
                transaction,
                label: "purge named tables".into(),
                patches: member_patches,
                block_patches,
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                style_patches,
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                merge_key: None,
            },
            linetype_patches,
        )?;
        Ok((removed, Some(Arc::new(change_set)), diagnostics))
    }
}
