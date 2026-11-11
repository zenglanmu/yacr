//! Post-import writes for the named style tables (`STYLE`, `-STYLE`,
//! `DIMSTYLE`, `MLEADERSTYLE`) and `RENAME BLOCK`.
//!
//! The importer owns the initial rows; this module is the command layer over
//! [`cad_db::DrawingDatabase::apply_style_table_changes`]. Every operation
//! (create, rename, delete, modify) is:
//!
//! 1. **resolved by name** against the drawing with one rule — exact match
//!    first, then a **unique** case-insensitive match; an unknown name and an
//!    ambiguous one are distinct refusals (`<key>.unknown` / `<key>.ambiguous`),
//!    never a silently picked row;
//! 2. **validated and written** by the database batch on a cloned candidate, so
//!    a refusal leaves the document, the revision, the tables and the history
//!    exactly as they were;
//! 3. **published** with one `cad-history` record carrying the before/after
//!    rows, so the whole command is exactly one undo step and redo-safe.
//!
//! # What each command writes
//!
//! | command | row | notes |
//! |---|---|---|
//! | `STYLE NEW` | `Style { name, resource_keys }` | `from <base>` copies the base's font keys |
//! | `STYLE RENAME` | `Style.name` | retargets `DIMTXSTY`/`MLEADERSTYLE 342` names in the same revision |
//! | `STYLE DELETE` | — | refused while any text/dimension/leader row references it |
//! | `STYLE SET` | `Style.resource_keys` | fields `FONT`, `KEYS`, `ADDKEY`, `REMOVEKEY` |
//! | `DIMSTYLE NEW/RENAME/DELETE` | `DimStyle` | rename/delete refused while a dimension still names the row |
//! | `DIMSTYLE MODIFY` | one stored `DimStyle` field per pair | grammar in [`fields`] |
//! | `MLEADERSTYLE NEW/RENAME/DELETE` | `MLeaderStyle` | same reference rule |
//! | `MLEADERSTYLE MODIFY` | one stored `MLeaderStyle` field per pair | grammar in [`fields`] |
//! | `RENAME BLOCK` | `BlockMetadata.name` | one block patch; duplicate/illegal names refused |
//!
//! `DIMSTYLE SET` and `MLEADERSTYLE SET` write the **session's current style
//! name** ([`SessionState::current_dim_style`] /
//! [`SessionState::current_mleader_style`]) — a real session write that
//! creation reads — and touch no table. `STYLE SET` is therefore the modify
//! form, not a "current text style" write: this build has no such session
//! variable.
//!
//! Every write form requires Work mode ([`CadError::PermissionDenied`] in
//! Viewer), checked here because the read-only `LIST` forms of the same
//! selectors stay Viewer-legal.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::document_drawing;
use crate::command_payload::{StyleTableKind, StyleTableOp};
use cad_db::{DimStyle, MLeaderStyle, Style, StyleTableChanges};

mod dim;
mod fields;
mod mleader;
mod text;

impl Application {
    /// Dispatch one typed `STYLE`/`-STYLE`/`MLEADERSTYLE` line.
    ///
    /// A bare selector (`CommandPayload::None`) is the existing read-only list
    /// and takes the caller's report path; a typed payload is a write form.
    pub(crate) fn style_table_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let payload = match &command.payload {
            CommandPayload::StyleTable(table) => table.clone(),
            _ => {
                return Err(CadError::InvalidInput(
                    "this selector's typed style-table payload is required here".into(),
                ))
            }
        };
        self.apply_style_table_op(session, command, payload.kind, payload.op)
    }

    /// Apply one style-table operation (the shared writer for all three tables).
    pub(crate) fn apply_style_table_op(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        kind: StyleTableKind,
        op: StyleTableOp,
    ) -> CadResult<CommandOutcome> {
        if let StyleTableOp::List = op {
            return Err(CadError::InvalidInput(
                "the list form is a read-only report; it has no write path".into(),
            ));
        }
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        match kind {
            StyleTableKind::TextStyle => self.text_style_table_op(session, command, op),
            StyleTableKind::DimStyle => self.dim_style_table_op(session, command, op),
            StyleTableKind::MLeaderStyle => self.mleader_style_table_op(session, command, op),
        }
    }

    // -----------------------------------------------------------------------
    // RENAME BLOCK
    // -----------------------------------------------------------------------

    /// `RENAME BLOCK <old> <new>`: rename one block definition's table name.
    ///
    /// The block table write is the existing combined path with a block patch,
    /// so the rename is one revision and one undo step. The name is trimmed and
    /// validated here (non-empty, single line, AutoCAD-legal characters,
    /// unique case-insensitively) because the block write path deliberately
    /// accepts an unnamed imported row and therefore does not enforce a name
    /// grammar itself. A block named `*...` (an anonymous/dynamic block) is
    /// refused by name: renaming an anonymous block would make it addressable
    /// as a named definition, which this build does not model.
    pub(crate) fn rename_block_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        old: &str,
        new: &str,
    ) -> CadResult<CommandOutcome> {
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let drawing = document_drawing(self, command)?;
        let id = resolve_unique_block(&drawing, old)?;
        let before = block_row(&drawing, id).ok_or_else(|| {
            CadError::InvalidInput(format!("block.unknown: no block named '{old}'"))
        })?;
        let trimmed = new.trim();
        if trimmed.is_empty() {
            return Err(invalid_refusal(
                "block.name_empty",
                "a block name must not be empty or whitespace-only",
            ));
        }
        if trimmed.contains(['\n', '\r']) {
            return Err(invalid_refusal(
                "block.name_multiline",
                "a block name must be a single line",
            ));
        }
        if let Some(illegal) = trimmed
            .chars()
            .find(|c| cad_db::ILLEGAL_LAYER_NAME_CHARS.contains(c))
        {
            return Err(invalid_refusal(
                "block.name_illegal_char",
                format!("a block name must not contain '{illegal}'"),
            ));
        }
        if trimmed.starts_with('*') {
            return Err(CadError::Unsupported(format!(
                "block.anonymous_rename: block '{}' is an anonymous definition and cannot be \
                 renamed by name",
                before
                    .metadata
                    .as_ref()
                    .map(|metadata| metadata.name.as_str())
                    .unwrap_or("<unnamed>")
            )));
        }
        let before_name = before
            .metadata
            .as_ref()
            .map(|metadata| metadata.name.clone())
            .ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "block.name_empty: block {} has no stored name to rename",
                    id.0
                ))
            })?;
        if before_name == trimmed {
            return Ok(report_outcome(vec![diagnostic(
                "rename.unchanged",
                format!("RENAME BLOCK: '{before_name}' already has that name; nothing changed"),
            )]));
        }
        if let Ok(existing) = resolve_unique_block(&drawing, trimmed) {
            if existing != id {
                return Err(invalid_refusal(
                    "block.duplicate_name",
                    format!(
                        "this drawing already has a block named '{trimmed}' (id {}); names are \
                         unique case-insensitively",
                        existing.0
                    ),
                ));
            }
        }
        let mut after = before.clone();
        if let Some(metadata) = after.metadata.as_mut() {
            metadata.name = trimmed.to_string();
        }
        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = candidate
            .apply_drawing_changes_with_blocks_display_layers_and_layouts_and_styles(
                "rename block",
                transaction,
                &[(id, Some(after.clone()))],
                Vec::new(),
                &[],
                &[],
                &[],
                StyleTableChanges::default(),
            )?;
        self.publish_candidate(
            command.document,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "rename block".to_string(),
                patches: Vec::new(),
                block_patches: vec![cad_db::BlockPatch {
                    id,
                    before: Some(before),
                    after: Some(after),
                }],
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![diagnostic(
                "rename.block",
                format!(
                    "RENAME BLOCK: renamed '{before_name}' to '{trimmed}' (one transaction / one \
                     undo step)"
                ),
            )],
            measurement: None,
            point_report: None,
        })
    }

    /// The shared commit: apply on a candidate, then publish one undo record.
    fn commit_style_write(
        &mut self,
        command: &Command,
        label: &str,
        build: impl FnOnce(
            &mut DrawingDatabase,
            TransactionId,
        ) -> CadResult<(
            cad_db::ChangeSet,
            Vec<cad_db::StylePatch>,
            Vec<cad_db::DimStylePatch>,
            Vec<cad_db::MLeaderStylePatch>,
        )>,
    ) -> CadResult<CommandOutcome> {
        let drawing = document_drawing(self, command)?;
        let mut candidate = drawing.as_ref().clone();
        let transaction = next_transaction_id();
        let (change_set, style_patches, dim_style_patches, mleader_style_patches) =
            build(&mut candidate, transaction)?;
        let published = change_set.clone();
        self.publish_candidate(
            command.document,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.to_string(),
                patches: Vec::new(),
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                style_patches,
                dim_style_patches,
                mleader_style_patches,
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(published)),
            diagnostics: Vec::new(),
            measurement: None,
            point_report: None,
        })
    }
}

/// Apply the `STYLE SET` assignments to a copy of the row.
///
/// The `NAME`/`RENAME` field is returned separately so the caller can cascade the
/// name reference into the other two style tables.
fn modified_style_row(
    before: &Style,
    assignments: &[(String, String)],
) -> CadResult<(Style, Option<(String, String)>)> {
    let mut after = before.clone();
    let mut renamed = None;
    for (field, value) in assignments {
        match field.to_ascii_lowercase().as_str() {
            "name" | "rename" => {
                if value.trim() != before.name {
                    renamed = Some((before.name.clone(), value.clone()));
                }
                after.name = value.clone();
            }
            "font" | "resource" | "resource_key" => {
                after.resource_keys = vec![value.trim().to_string()];
            }
            "keys" | "resource_keys" => {
                after.resource_keys = value
                    .split(',')
                    .map(str::trim)
                    .filter(|key| !key.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            "addkey" | "add_key" => {
                let key = value.trim().to_string();
                if !key.is_empty() && !after.resource_keys.contains(&key) {
                    after.resource_keys.push(key);
                }
            }
            "removekey" | "remove_key" => {
                let key = value.trim();
                after.resource_keys.retain(|existing| existing != key);
            }
            other => {
                return Err(CadError::InvalidInput(format!(
                    "style.unknown_field: '{other}' is not a stored text-style field; the writable \
                     set is NAME, FONT, KEYS, ADDKEY and REMOVEKEY"
                )))
            }
        }
    }
    Ok((after, renamed))
}

// ---------------------------------------------------------------------------
// Name resolution (one rule for the whole module)
// ---------------------------------------------------------------------------

fn resolve_unique<K: Copy + Eq + std::fmt::Debug>(
    key: &str,
    label: &str,
    name: &str,
    rows: Vec<(K, String)>,
) -> CadResult<K> {
    let wanted = name.trim();
    if wanted.is_empty() {
        return Err(invalid_refusal(
            &format!("{key}.name_empty"),
            format!("a {label} name must not be empty or whitespace-only"),
        ));
    }
    let mut exact: Vec<K> = rows
        .iter()
        .filter(|(_, stored)| stored == wanted)
        .map(|(id, _)| *id)
        .collect();
    if exact.len() == 1 {
        return Ok(exact.remove(0));
    }
    if exact.len() > 1 {
        return Err(invalid_refusal(
            &format!("{key}.ambiguous"),
            format!(
                "{label} name '{wanted}' matches {} rows exactly",
                exact.len()
            ),
        ));
    }
    let mut folded: Vec<K> = rows
        .iter()
        .filter(|(_, stored)| stored.eq_ignore_ascii_case(wanted))
        .map(|(id, _)| *id)
        .collect();
    if folded.len() == 1 {
        return Ok(folded.remove(0));
    }
    if folded.len() > 1 {
        return Err(invalid_refusal(
            &format!("{key}.ambiguous"),
            format!(
                "{label} name '{wanted}' matches {} rows case-insensitively; address the row by \
                 its exact name",
                folded.len()
            ),
        ));
    }
    Err(invalid_refusal(
        &format!("{key}.unknown"),
        format!("this drawing has no {label} named '{wanted}'"),
    ))
}

fn resolve_unique_style(drawing: &DrawingDatabase, name: &str) -> CadResult<StyleId> {
    resolve_unique(
        "style",
        "text style",
        name,
        drawing
            .styles()
            .map(|row| (row.id, row.name.clone()))
            .collect(),
    )
}

fn resolve_unique_dim_style(drawing: &DrawingDatabase, name: &str) -> CadResult<DimStyleId> {
    resolve_unique(
        "dimstyle",
        "dimension style",
        name,
        drawing
            .dim_styles()
            .map(|row| (row.id, row.name.clone()))
            .collect(),
    )
}

fn resolve_unique_mleader_style(
    drawing: &DrawingDatabase,
    name: &str,
) -> CadResult<MLeaderStyleId> {
    resolve_unique(
        "mleaderstyle",
        "multileader style",
        name,
        drawing
            .mleader_styles()
            .map(|row| (row.id, row.name.clone()))
            .collect(),
    )
}

fn resolve_unique_block(drawing: &DrawingDatabase, name: &str) -> CadResult<BlockId> {
    resolve_unique(
        "block",
        "block",
        name,
        drawing
            .blocks()
            .filter_map(|block| {
                drawing
                    .block_name(block.id)
                    .map(|name| (block.id, name.to_string()))
            })
            .collect(),
    )
}

/// The stored writable row of one block definition.
fn block_row(drawing: &DrawingDatabase, id: BlockId) -> Option<cad_db::BlockRow> {
    let definition = drawing.block(id)?.clone();
    let metadata = drawing.block_metadata(id).cloned();
    Some(cad_db::BlockRow {
        definition,
        metadata,
    })
}

fn invalid_refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

fn style_unknown(name: &str) -> CadError {
    invalid_refusal(
        "style.unknown",
        format!("this drawing has no text style named '{}'", name.trim()),
    )
}

fn dim_style_unknown(name: &str) -> CadError {
    invalid_refusal(
        "dimstyle.unknown",
        format!(
            "this drawing has no dimension style named '{}'",
            name.trim()
        ),
    )
}

fn mleader_style_unknown(name: &str) -> CadError {
    invalid_refusal(
        "mleaderstyle.unknown",
        format!(
            "this drawing has no multileader style named '{}'",
            name.trim()
        ),
    )
}
