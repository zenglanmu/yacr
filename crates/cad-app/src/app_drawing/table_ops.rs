//! The post-creation table commands: `TABLEDIT MERGE|UNMERGE`, `MATCHCELL` and
//! `TABLESTYLE`.
//!
//! All three are **Work-only** and share the same shape as the cell-text edit in
//! [`super::table_edit`]: the pre-selection is the object input (exactly one
//! directly addressable `Table`), the new definition is produced by the domain
//! writers in [`cad_domain::TableGeometry`] (which rebuild the derived display),
//! and the whole change is staged as **one** `update_entity` in one transaction /
//! one undo step. A refusal happens before that transaction, so it writes
//! nothing, bumps no revision and records no history.
//!
//! Stable refusal codes (each carries the command's own prefix):
//!
//! * `tabledit.payload` / `.no_selection` / `.multiple` / `.not_table` /
//!   `.out_of_range` / `.unchanged` — the shared pre-selection and addressing
//!   refusals, byte-identical to the cell-text form;
//! * `table.merge_span` / `.merge_out_of_grid` / `.merge_covered` /
//!   `.merge_overlap` / `.not_merged` — the domain writers' structural merge
//!   refusals, kept under the model's own stable `table.*` prefix so the model
//!   and the command answer the same code;
//! * `matchcell.payload` / `.no_selection` / `.multiple` / `.not_table` /
//!   `.out_of_range` / `.no_targets` / `.unchanged`;
//! * `tablestyle.payload` / `.no_selection` / `.multiple` / `.not_table` /
//!   `.empty` / `.invalid` (a non-finite/negative margin or a non-finite/
//!   non-positive text height).
//!
//! Typed spellings (parsed in `cad_ui_core::session`):
//!
//! * `TABLEDIT MERGE <row> <col> <height> <width>`
//! * `TABLEDIT UNMERGE <row> <col>`
//! * `TABLEDIT STYLE <name>` — the `TABLESTYLE <name>` write, reached through the
//!   `TABLEDIT` selector
//! * `MATCHCELL <src_row> <src_col> <dst_row> <dst_col> [<dst_row> <dst_col> ...]`
//! * `TABLESTYLE` (read-only report) and `TABLESTYLE [<name>] [DESC <text>]
//!   [MARGIN <h> <v>] [HEIGHT <h>] [TEXTSTYLE <name>] [BORDER
//!   <TOP|RIGHT|BOTTOM|LEFT|ALL> <ON|OFF|DOUBLE> [<spacing>]] [TITLE <ON|OFF>]
//!   [HEADER <ON|OFF>]`
//!
//! `TABLEDIT <row> <col> [text]` itself is unchanged and still lives in
//! [`super::table_edit`]. Cell blocks/fields (`TINSERT`, `FIELD`) stay named
//! refusals: this build has no block-in-cell or field model.

use super::table_edit::{commit_table_geometry, select_table, table_refusal, TableSelection};
use super::*;
use crate::app_drawing::table_payload::{
    TableCellAddress, TablePayload, TableStyleCommand, TableStyleEdge,
};
use cad_domain::{StyleId, TableGeometry, TableStyleValues};

impl Application {
    /// Dispatch one post-creation table command.
    pub(crate) fn table_ops(
        &mut self,
        session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            // `TABLEDIT` carries both the merge forms and the style assignment;
            // which one this line is comes from its payload, so the two never
            // answer each other's refusal.
            ExtendedCommandId::TableEdit => match &command.payload {
                CommandPayload::Table(payload)
                    if matches!(payload.as_ref(), TablePayload::Style(_)) =>
                {
                    self.table_style(session, command)
                }
                _ => self.table_edit_merge(session, command),
            },
            ExtendedCommandId::MatchCell => self.match_cell(session, command),
            ExtendedCommandId::TableStyleList => self.table_style(session, command),
            other => Err(CadError::Invariant(format!(
                "not a table operation: {other:?}"
            ))),
        }
    }

    /// `TABLEDIT MERGE …` / `TABLEDIT UNMERGE …`.
    fn table_edit_merge(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Table(payload) = &command.payload else {
            return Err(table_refusal(
                "tabledit.payload",
                "TABLEDIT MERGE/UNMERGE needs a table payload (anchor, span) — the cell-text form \
                 keeps its own TableEdit payload",
            ));
        };
        let selection = select_table(self, session, command, "tabledit")?;
        let (updated, message) = match payload.as_ref() {
            TablePayload::Merge {
                anchor,
                height,
                width,
            } => {
                let (row, column) = resolve_cell(*anchor, &selection, "tabledit")?;
                let covered_text =
                    covered_text_count(&selection.table, row, column, *height, *width);
                let updated = selection.table.with_merge(row, column, *height, *width)?;
                let hidden = if covered_text == 0 {
                    String::new()
                } else {
                    format!(
                        "; {covered_text} covered cell(s) keep their stored text, which this \
                         build does not concatenate into the merged cell and therefore does not \
                         draw"
                    )
                };
                (
                    updated,
                    format!(
                        "TABLEDIT MERGE: merged {height} x {width} cells anchored at ({}, {}) of \
                         table {}; one transaction / one undo step{hidden}",
                        anchor.row, anchor.col, selection.before.id.0
                    ),
                )
            }
            TablePayload::Unmerge { cell } => {
                let (row, column) = resolve_cell(*cell, &selection, "tabledit")?;
                let updated = selection.table.with_unmerge(row, column)?;
                (
                    updated,
                    format!(
                        "TABLEDIT UNMERGE: split the merged range covering ({}, {}) of table {}; \
                         one transaction / one undo step",
                        cell.row, cell.col, selection.before.id.0
                    ),
                )
            }
            other => {
                return Err(table_refusal(
                    "tabledit.payload",
                    format!(
                        "TABLEDIT MERGE/UNMERGE does not accept the {} form",
                        payload_name(other)
                    ),
                ))
            }
        };
        commit_table_geometry(
            self,
            command.document,
            &selection,
            updated,
            "edit table cells (merge)",
            "tabledit.merge",
            message,
        )
    }

    /// `MATCHCELL <src_row> <src_col> <dst_row> <dst_col> …`.
    fn match_cell(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Table(payload) = &command.payload else {
            return Err(table_refusal(
                "matchcell.payload",
                "MATCHCELL needs a table payload (source cell, destination cells)",
            ));
        };
        let TablePayload::MatchCell { source, targets } = payload.as_ref() else {
            return Err(table_refusal(
                "matchcell.payload",
                format!(
                    "MATCHCELL does not accept the {} form",
                    payload_name(payload)
                ),
            ));
        };
        if targets.is_empty() {
            return Err(table_refusal(
                "matchcell.no_targets",
                "MATCHCELL needs at least one destination cell after the source cell",
            ));
        }
        let selection = select_table(self, session, command, "matchcell")?;
        let (source_row, source_column) = resolve_cell(*source, &selection, "matchcell")?;
        let mut target_cells = Vec::with_capacity(targets.len());
        for target in targets {
            target_cells.push(resolve_cell(*target, &selection, "matchcell")?);
        }
        let source_style = selection
            .table
            .cell(source_row, source_column)
            .map(|cell| {
                (
                    cell.text_height,
                    cell.style_name.clone(),
                    cell.style,
                    cell.font.clone(),
                    cell.borders,
                )
            })
            .ok_or_else(|| {
                table_refusal(
                    "matchcell.out_of_range",
                    format!("the source cell {} does not exist", address_label(*source)),
                )
            })?;
        let unchanged = target_cells.iter().all(|(row, column)| {
            selection.table.cell(*row, *column).is_some_and(|cell| {
                (
                    cell.text_height,
                    cell.style_name.clone(),
                    cell.style,
                    cell.font.clone(),
                    cell.borders,
                ) == source_style
            })
        });
        if unchanged {
            return Err(table_refusal(
                "matchcell.unchanged",
                format!(
                    "every destination cell already carries the style of {}; nothing was written",
                    address_label(*source)
                ),
            ));
        }
        let updated = selection
            .table
            .with_cell_styles((source_row, source_column), &target_cells)?;
        let addresses = targets
            .iter()
            .map(|target| address_label(*target))
            .collect::<Vec<_>>()
            .join(", ");
        commit_table_geometry(
            self,
            command.document,
            &selection,
            updated,
            "match table cell style",
            "matchcell.applied",
            format!(
                "MATCHCELL: copied the style of {} (text height {}, style `{}`, border \
                 visibility/spacing) onto {addresses} of table {}; one transaction / one undo step",
                address_label(*source),
                source_style.0,
                source_style.1,
                selection.before.id.0
            ),
        )
    }

    /// `TABLESTYLE` (report) / `TABLESTYLE <name> [<field> …]` (write).
    fn table_style(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let selection = select_table(self, session, command, "tablestyle")?;
        let requested = match &command.payload {
            CommandPayload::None => None,
            CommandPayload::Table(payload) => match payload.as_ref() {
                TablePayload::Style(style) => Some(style.as_ref()),
                other => {
                    return Err(table_refusal(
                        "tablestyle.payload",
                        format!(
                            "TABLESTYLE does not accept the {} form",
                            payload_name(other)
                        ),
                    ))
                }
            },
            _ => {
                return Err(table_refusal(
                    "tablestyle.payload",
                    "TABLESTYLE needs no payload (report) or a table style payload",
                ))
            }
        };
        let Some(requested) = requested else {
            let style = selection.table.style();
            return Ok(CommandOutcome {
                objects: vec![ObjectId(selection.before.id.0)],
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: Some(ObjectId(selection.before.id.0)),
                    code: "tablestyle.report".into(),
                    message: format!(
                        "TABLESTYLE: table {} uses style `{}` ({}, margins {} x {}, text height \
                         {}, text style `{}`, borders top/right/bottom/left {}{}{}{}, title row {}, \
                         header row {}){}",
                        selection.before.id.0,
                        style.name,
                        if style.description.is_empty() {
                            "no description"
                        } else {
                            &style.description
                        },
                        style.horizontal_margin,
                        style.vertical_margin,
                        style.text_height,
                        style.text_style_name,
                        edge_label(style.borders.top.visible, style.borders.top.double_spacing),
                        edge_label(
                            style.borders.right.visible,
                            style.borders.right.double_spacing
                        ),
                        edge_label(
                            style.borders.bottom.visible,
                            style.borders.bottom.double_spacing
                        ),
                        edge_label(style.borders.left.visible, style.borders.left.double_spacing),
                        if style.title_suppressed {
                            "suppressed"
                        } else {
                            "shown"
                        },
                        if style.header_suppressed {
                            "suppressed"
                        } else {
                            "shown"
                        },
                        STYLE_GAPS,
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        };
        if requested.is_empty() {
            return Err(table_refusal(
                "tablestyle.empty",
                "TABLESTYLE needs at least one field to write (or no argument at all for the \
                 read-only report)",
            ));
        }
        let current = selection.table.style();
        let name = requested
            .name
            .clone()
            .unwrap_or_else(|| current.name.clone());
        let mut style = if name.eq_ignore_ascii_case(&current.name) {
            current.clone()
        } else {
            TableStyleValues::named(&name)
        };
        style.name = name;
        apply_style_command(&mut style, requested)?;
        style
            .validate()
            .map_err(|error| table_refusal("tablestyle.invalid", cad_error_detail(&error)))?;
        if style == *current {
            return Err(table_refusal(
                "tablestyle.unchanged",
                format!(
                    "table {} already uses exactly this `{}` style; nothing was written",
                    selection.before.id.0, style.name
                ),
            ));
        }
        // Resolve every text style name the new snapshot can stamp **before** the
        // commit takes `&mut self`: the domain writer needs the id/font pair and
        // the commit needs the workspace mutably.
        let mut resolved: Vec<(String, StyleId, Option<String>)> = Vec::new();
        for name in [
            style.text_style_name.clone(),
            current.text_style_name.clone(),
        ] {
            if name.is_empty() || resolved.iter().any(|(seen, _, _)| *seen == name) {
                continue;
            }
            let (id, font) = resolve_text_style(self, command.document, &name);
            resolved.push((name, id, font));
        }
        let snapshot = resolved.clone();
        let updated = selection.table.with_style_resolved(style.clone(), |name| {
            snapshot
                .iter()
                .find(|(seen, _, _)| seen == name)
                .map(|(_, id, font)| (*id, font.clone()))
                .unwrap_or((StyleId(0), None))
        })?;
        let re_resolved = re_resolved_cells(&selection.table, &updated);
        commit_table_geometry(
            self,
            command.document,
            &selection,
            updated,
            "edit table style",
            "tablestyle.applied",
            format!(
                "TABLESTYLE: table {} now uses `{}` (text height {}, text style `{}`); \
                 {re_resolved} inheriting cell(s) re-resolved to the new values, explicitly styled \
                 cells kept theirs; one transaction / one undo step{}",
                selection.before.id.0,
                style.name,
                style.text_height,
                style.text_style_name,
                STYLE_GAPS
            ),
        )
    }
}

/// The style fields this build stores but does not apply (named, never implied).
const STYLE_GAPS: &str = "; the title/header row styles, per-cell border colours/lineweights/\
fills and the annotative flag are not part of this build's table-style model";

/// The `tablestyle.report` gap sentence's first character is joined mid-line, so
/// the constant starts there rather than with a space of its own.
fn edge_label(visible: bool, double_spacing: f64) -> String {
    if !visible {
        return "hidden".to_string();
    }
    if double_spacing > 0.0 {
        format!("double({double_spacing})")
    } else {
        "single".to_string()
    }
}

/// Apply the present fields of a `TABLESTYLE` write onto `style`.
fn apply_style_command(style: &mut TableStyleValues, command: &TableStyleCommand) -> CadResult<()> {
    if let Some(description) = &command.description {
        style.description = description.clone();
    }
    if let Some(horizontal) = command.horizontal_margin {
        style.horizontal_margin = horizontal;
    }
    if let Some(vertical) = command.vertical_margin {
        style.vertical_margin = vertical;
    }
    if let Some(height) = command.text_height {
        style.text_height = height;
    }
    if let Some(name) = &command.text_style_name {
        style.text_style_name = name.clone();
    }
    if let Some(suppressed) = command.title_suppressed {
        style.title_suppressed = suppressed;
    }
    if let Some(suppressed) = command.header_suppressed {
        style.header_suppressed = suppressed;
    }
    for (edge, visible, spacing) in &command.borders {
        let spacing = spacing.unwrap_or(0.0);
        if !spacing.is_finite() || spacing < 0.0 {
            return Err(table_refusal(
                "tablestyle.invalid",
                format!("the double-line spacing must be finite and non-negative, got {spacing}"),
            ));
        }
        let border = cad_domain::TableBorder {
            visible: *visible,
            double_spacing: spacing,
        };
        match edge {
            TableStyleEdge::Top => style.borders.top = border,
            TableStyleEdge::Right => style.borders.right = border,
            TableStyleEdge::Bottom => style.borders.bottom = border,
            TableStyleEdge::Left => style.borders.left = border,
            TableStyleEdge::All => {
                style.borders = cad_domain::TableCellBorders {
                    top: border,
                    right: border,
                    bottom: border,
                    left: border,
                }
            }
        }
    }
    Ok(())
}

/// The drawing's STYLE-table lookup for a cell text-style name.
///
/// The domain cannot resolve a name (it has no database), so the command supplies
/// the id and the first font resource key — exactly what the importer stamped for
/// the same name. An unknown name resolves to [`StyleId(0)`] with no font, which
/// is the value the importer stores for an unresolved name.
fn resolve_text_style(
    app: &Application,
    document: DocumentId,
    name: &str,
) -> (StyleId, Option<String>) {
    let Some(document) = app.workspace.documents.get(&document) else {
        return (StyleId(0), None);
    };
    match document.drawing.style_by_name(name) {
        Some(style) => (style.id, style.resource_keys.first().cloned()),
        None => (StyleId(0), None),
    }
}

/// How many covered cells of the range anchor `(row, column)` still store text.
fn covered_text_count(
    table: &TableGeometry,
    row: usize,
    column: usize,
    height: u32,
    width: u32,
) -> usize {
    let mut count = 0;
    for dr in 0..height as usize {
        for dc in 0..width as usize {
            if dr == 0 && dc == 0 {
                continue;
            }
            if table
                .cell(row + dr, column + dc)
                .is_some_and(|cell| !cell.text.is_empty())
            {
                count += 1;
            }
        }
    }
    count
}

/// How many cells changed by a style assignment (the inheriting ones).
fn re_resolved_cells(before: &TableGeometry, after: &TableGeometry) -> usize {
    let mut changed = 0;
    for (row_index, row) in before.rows().iter().enumerate() {
        for (column_index, cell) in row.cells.iter().enumerate() {
            if after.cell(row_index, column_index).is_some_and(|updated| {
                updated.text_height != cell.text_height
                    || updated.style_name != cell.style_name
                    || updated.borders != cell.borders
            }) {
                changed += 1;
            }
        }
    }
    changed
}

/// One 1-based payload address resolved against the selected table.
fn resolve_cell(
    address: TableCellAddress,
    selection: &TableSelection,
    prefix: &str,
) -> CadResult<(usize, usize)> {
    let Some((row, column)) = address.zero_based() else {
        return Err(table_refusal(
            format!("{prefix}.out_of_range"),
            format!(
                "cell ({}, {}) is outside the 1-based address space of table {}",
                address.row, address.col, selection.before.id.0
            ),
        ));
    };
    if selection.table.cell(row, column).is_none() {
        return Err(table_refusal(
            format!("{prefix}.out_of_range"),
            format!(
                "cell ({}, {}) is outside table {}'s {} rows x {} columns (row and column are \
                 1-based)",
                address.row,
                address.col,
                selection.before.id.0,
                selection.table.rows().len(),
                selection.table.columns().len()
            ),
        ));
    }
    Ok((row, column))
}

fn payload_name(payload: &TablePayload) -> &'static str {
    match payload {
        TablePayload::Merge { .. } => "MERGE",
        TablePayload::Unmerge { .. } => "UNMERGE",
        TablePayload::MatchCell { .. } => "MATCHCELL",
        TablePayload::Style(_) => "STYLE",
    }
}

fn address_label(address: TableCellAddress) -> String {
    format!("({}, {})", address.row, address.col)
}

/// The detail text of a domain refusal for a diagnostic.
fn cad_error_detail(error: &CadError) -> String {
    match error {
        CadError::InvalidInput(detail)
        | CadError::Unsupported(detail)
        | CadError::Invariant(detail)
        | CadError::ResourceMissing(detail)
        | CadError::CorruptData(detail)
        | CadError::GpuFailure(detail) => detail.clone(),
        CadError::NotImplemented(feature) => format!("not implemented: {feature}"),
        other => format!("{other:?}"),
    }
}
