//! The post-creation table commands: `TABLEDIT MERGE|UNMERGE|FORMAT`, `MATCHCELL`
//! and `TABLESTYLE`.
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
//!   `.out_of_range` / `.unchanged` / `.format_empty` — the shared pre-selection
//!   and addressing refusals, byte-identical to the cell-text form;
//! * `tabledit.format_color` / `.format_lineweight` — an ACI index or a
//!   lineweight outside the value set the DXF record can carry exactly
//!   (`1..=255`, the canonical AutoCAD weights), refused rather than snapped;
//! * `tabledit.format_unsupported` — a recognised `TABLEDIT FORMAT` field this
//!   build cannot store (**border line type**), with the reason;
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
//! * `TABLEDIT FORMAT <row> <col> [ALIGN <TL|TC|TR|ML|MC|MR|BL|BC|BR>]
//!   [ROTATION <degrees>] [COLOR <aci|BYLAYER>] [FILL <aci|NONE>]
//!   [LINEWEIGHT <mm|BYLAYER>] [BORDERCOLOR <aci|BYLAYER>]`
//! * `TABLEDIT STYLE <name>` — the `TABLESTYLE <name>` write, reached through the
//!   `TABLEDIT` selector
//! * `MATCHCELL <src_row> <src_col> <dst_row> <dst_col> [<dst_row> <dst_col> ...]`
//! * `TABLESTYLE` (read-only report) and `TABLESTYLE [<name>] [DESC <text>]
//!   [MARGIN <h> <v>] [ROW <DATA|TITLE|HEADER>] [HEIGHT <h>] [TEXTSTYLE <name>]
//!   [ALIGN <code>] [COLOR <aci|BYLAYER>] [FILL <aci|NONE>]
//!   [LINEWEIGHT <mm|BYLAYER>] [BORDERCOLOR <aci|BYLAYER>]
//!   [BORDER <TOP|RIGHT|BOTTOM|LEFT|ALL> <ON|OFF|DOUBLE>
//!   [<spacing>]] [TITLE <ON|OFF>] [HEADER <ON|OFF>]`
//!
//! `TABLEDIT <row> <col> [text]` itself is unchanged and still lives in
//! [`super::table_edit`]. Cell blocks/fields (`TINSERT`, `FIELD`) stay named
//! refusals: this build has no block-in-cell or field model.

use super::table_edit::{commit_table_geometry, select_table, table_refusal, TableSelection};
use super::*;
use crate::app_drawing::table_payload::{
    aci_srgb, canonical_lineweight_hundredths, TableCellAddress, TableColorValue,
    TableLineweightValue, TablePayload, TableRowStyleCommand, TableStyleCommand, TableStyleEdge,
    TableStyleRowScope,
};
use cad_domain::{
    StyleId, TableCellAppearanceEdit, TableFieldEdit, TableGeometry, TableRowKind, TableRowStyle,
    TableStyleValues,
};

impl Application {
    /// Dispatch one post-creation table command.
    pub(crate) fn table_ops(
        &mut self,
        session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            // `TABLEDIT` carries the merge forms, the format form and the style
            // assignment; which one this line is comes from its payload, so the
            // forms never answer each other's refusal.
            ExtendedCommandId::TableEdit => match &command.payload {
                CommandPayload::Table(payload) => match payload.as_ref() {
                    TablePayload::Style(_) => self.table_style(session, command),
                    TablePayload::Format { .. } => self.table_edit_format(session, command),
                    _ => self.table_edit_merge(session, command),
                },
                _ => Err(table_refusal(
                    "tabledit.payload",
                    "TABLEDIT needs a table payload (MERGE/UNMERGE/FORMAT) or the cell-text form",
                )),
            },
            ExtendedCommandId::MatchCell => self.match_cell(session, command),
            ExtendedCommandId::TableStyleList => self.table_style(session, command),
            other => Err(CadError::Invariant(format!(
                "not a table operation: {other:?}"
            ))),
        }
    }

    /// `TABLEDIT FORMAT <row> <col> [ALIGN <code>] [ROTATION <degrees>]
    /// [COLOR <aci|BYLAYER>] [FILL <aci|NONE>] [LINEWEIGHT <mm|BYLAYER>]
    /// [BORDERCOLOR <aci|BYLAYER>]`.
    ///
    /// Sets the per-cell formatting this build stores *and draws*: the text
    /// alignment, the text rotation (relative to the table frame) and the cell's
    /// appearance — text colour, background fill, and the four border edges'
    /// colour and lineweight. Every field combines in **one** write, so any
    /// combination is one transaction / one undo step. A field this build cannot
    /// represent (a border line type) is refused by the parser before this point,
    /// with its own named reason; the payload has no slot for it, so it can never
    /// be silently accepted here.
    fn table_edit_format(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Table(payload) = &command.payload else {
            return Err(table_refusal(
                "tabledit.payload",
                "TABLEDIT FORMAT needs a table payload (cell address, alignment, rotation, colour, \
                 fill, lineweight)",
            ));
        };
        let TablePayload::Format {
            cell,
            alignment,
            rotation,
            color,
            fill,
            lineweight,
            border_color,
        } = payload.as_ref()
        else {
            return Err(table_refusal(
                "tabledit.payload",
                format!(
                    "TABLEDIT FORMAT does not accept the {} form",
                    payload_name(payload)
                ),
            ));
        };
        if alignment.is_none()
            && rotation.is_none()
            && color.is_none()
            && fill.is_none()
            && lineweight.is_none()
            && border_color.is_none()
        {
            return Err(table_refusal(
                "tabledit.format_empty",
                "TABLEDIT FORMAT needs at least one of ALIGN <code>, ROTATION <degrees>, \
                 COLOR <aci|BYLAYER>, FILL <aci|NONE>, LINEWEIGHT <mm|BYLAYER> or \
                 BORDERCOLOR <aci|BYLAYER>",
            ));
        }
        // Resolve every colour/lineweight **before** the write: an ACI index or a
        // non-canonical weight is refused here, so nothing half-applies.
        let appearance = TableCellAppearanceEdit {
            text_color: color_edit(*color)?,
            fill: color_edit(*fill)?,
            border_color: color_edit(*border_color)?,
            border_lineweight: lineweight_edit(*lineweight)?,
        };
        let selection = select_table(self, session, command, "tabledit")?;
        let (row, column) = resolve_cell(*cell, &selection, "tabledit")?;
        let before = selection.table.cell(row, column).cloned().ok_or_else(|| {
            table_refusal(
                "tabledit.out_of_range",
                format!("the cell {} does not exist", address_label(*cell)),
            )
        })?;
        // The state the write would produce, so an identical re-write is
        // `tabledit.unchanged` and opens no transaction.
        let mut wanted = before.clone();
        if let Some(value) = alignment {
            wanted.alignment = *value;
        }
        if let Some(value) = rotation {
            wanted.rotation = *value;
        }
        appearance.text_color.apply(&mut wanted.text_color);
        appearance.fill.apply(&mut wanted.fill_color);
        appearance.apply_borders(&mut wanted.borders);
        if wanted == before {
            return Err(table_refusal(
                "tabledit.unchanged",
                format!(
                    "cell {} of table {} already carries this format; nothing was written",
                    address_label(*cell),
                    selection.before.id.0
                ),
            ));
        }
        let mut updated = selection.table.clone();
        if !appearance.is_empty() {
            updated = updated.with_cell_appearance(row, column, appearance)?;
        }
        if alignment.is_some() || rotation.is_some() {
            updated = updated.with_cell_format(row, column, *alignment, *rotation)?;
        }
        let after = updated.cell(row, column).cloned().unwrap_or(before);
        commit_table_geometry(
            self,
            command.document,
            &selection,
            updated,
            "format table cell",
            "tabledit.format",
            format!(
                "TABLEDIT FORMAT: cell {} of table {} now has alignment {}, rotation {}° \
                 (relative to the table frame), text colour {}, fill {}, border colour {}, border \
                 lineweight {}; one transaction / one undo step",
                address_label(*cell),
                selection.before.id.0,
                after.alignment.label(),
                after.rotation.to_degrees(),
                color_label(after.text_color, "BYLAYER"),
                color_label(after.fill_color, "NONE"),
                borders_color_label(&after.borders),
                borders_lineweight_label(&after.borders)
            ),
        )
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
        // The whole per-cell format this build models — text height, text style
        // identity/name/font, the four border edges (visibility, spacing, colour
        // and lineweight), the text alignment, the text rotation, the cell's text
        // colour and its background fill. Text and merge spans are not part of
        // it.
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
                    cell.alignment,
                    cell.rotation,
                    cell.text_color,
                    cell.fill_color,
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
                    cell.alignment,
                    cell.rotation,
                    cell.text_color,
                    cell.fill_color,
                ) == source_style
            })
        });
        if unchanged {
            return Err(table_refusal(
                "matchcell.unchanged",
                format!(
                    "every destination cell already carries the format of {}; nothing was written",
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
            "match table cell format",
            "matchcell.applied",
            format!(
                "MATCHCELL: copied the format of {} (text height {}, style `{}`, border \
                 visibility/spacing/colour/lineweight, alignment {}, rotation {}°, text colour \
                 {}, fill {}) onto {addresses} of table {}; one transaction / one undo step",
                address_label(*source),
                source_style.0,
                source_style.1,
                source_style.5.label(),
                source_style.6.to_degrees(),
                color_label(source_style.7, "BYLAYER"),
                color_label(source_style.8, "NONE"),
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
                        "TABLESTYLE: table {} uses style `{}` ({}, margins {} x {}; {}; title row \
                         {}, header row {}){}",
                        selection.before.id.0,
                        style.name,
                        if style.description.is_empty() {
                            "no description"
                        } else {
                            &style.description
                        },
                        style.horizontal_margin,
                        style.vertical_margin,
                        row_sets_label(style),
                        suppression_label(style.title_suppressed),
                        suppression_label(style.header_suppressed),
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
        // the commit needs the workspace mutably. The three row value sets can
        // name three different styles, and the old sets' names are resolved too so
        // a cell that stops inheriting keeps a real id/font pair.
        let mut names: Vec<String> = Vec::new();
        for kind in [
            TableRowKind::Data,
            TableRowKind::Title,
            TableRowKind::Header,
        ] {
            names.push(style.row_style(kind).text_style_name);
            names.push(current.row_style(kind).text_style_name);
        }
        let mut resolved: Vec<(String, StyleId, Option<String>)> = Vec::new();
        for name in names {
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
                "TABLESTYLE: table {} now uses `{}` ({}); {re_resolved} inheriting cell(s) \
                 re-resolved to the new values, explicitly formatted cells kept theirs; one \
                 transaction / one undo step{}",
                selection.before.id.0,
                style.name,
                row_sets_label(&style),
                STYLE_GAPS
            ),
        )
    }
}

/// How the report and the write message summarise the three row value sets.
fn row_sets_label(style: &TableStyleValues) -> String {
    let data = style.data_row();
    let mut parts = Vec::with_capacity(3);
    for kind in [
        TableRowKind::Data,
        TableRowKind::Title,
        TableRowKind::Header,
    ] {
        let row = style.row_style(kind);
        let linked = if kind != TableRowKind::Data && row == data {
            " = data row"
        } else {
            ""
        };
        parts.push(format!(
            "{} row: text height {}, text style `{}`, alignment {}, borders {}, text colour {}, \
             fill {}{linked}",
            kind.label(),
            row.text_height,
            row.text_style_name,
            row.alignment.label(),
            borders_label(&row.borders),
            color_label(row.text_color, "BYLAYER"),
            color_label(row.fill_color, "NONE")
        ));
    }
    parts.join("; ")
}

/// `top/right/bottom/left` edge labels, in one short form.
fn borders_label(borders: &cad_domain::TableCellBorders) -> String {
    format!(
        "{}/{}/{}/{}",
        edge_label(borders.top.visible, borders.top.double_spacing),
        edge_label(borders.right.visible, borders.right.double_spacing),
        edge_label(borders.bottom.visible, borders.bottom.double_spacing),
        edge_label(borders.left.visible, borders.left.double_spacing)
    )
}

fn suppression_label(suppressed: bool) -> &'static str {
    if suppressed {
        "suppressed"
    } else {
        "shown"
    }
}

/// The style fields this build does not model (named, never implied).
const STYLE_GAPS: &str =
    "; a per-border line type is not modelled, a stored border lineweight is exported but drawn \
     one pixel wide, and the style's annotative flag is not stored";

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
///
/// The row-scoped groups apply in typed order, so `ROW TITLE HEIGHT 6 ROW DATA
/// ALIGN MC` writes exactly what it reads. A group with no explicit `ROW` scope
/// reaches all three value sets (the pre-row-style behaviour: one height, one
/// text style and one border set for every row).
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
    if let Some(suppressed) = command.title_suppressed {
        style.title_suppressed = suppressed;
    }
    if let Some(suppressed) = command.header_suppressed {
        style.header_suppressed = suppressed;
    }
    for group in &command.rows {
        apply_row_style_command(style, group)?;
    }
    Ok(())
}

/// Apply one row-scoped `TABLESTYLE` group.
fn apply_row_style_command(
    style: &mut TableStyleValues,
    group: &TableRowStyleCommand,
) -> CadResult<()> {
    let kinds: &[TableRowKind] = group.scope.unwrap_or(TableStyleRowScope::All).kinds();
    // Each targeted row starts from its **current effective** values, so a group
    // that only sets a height keeps that row's text style, alignment and borders.
    let mut rows: Vec<(TableRowKind, TableRowStyle)> = kinds
        .iter()
        .map(|kind| (*kind, style.row_style(*kind)))
        .collect();
    for (kind, row) in &mut rows {
        if let Some(height) = group.text_height {
            if !height.is_finite() || height <= 0.0 {
                return Err(table_refusal(
                    "tablestyle.invalid",
                    format!(
                        "the {} row text height must be finite and positive, got {height}",
                        kind.label()
                    ),
                ));
            }
            row.text_height = height;
        }
        if let Some(name) = &group.text_style_name {
            row.text_style_name = name.clone();
        }
        if let Some(alignment) = group.alignment {
            row.alignment = alignment;
        }
        if let Some(choice) = group.text_color {
            row.text_color = resolve_color_choice(choice)?;
        }
        if let Some(choice) = group.fill_color {
            row.fill_color = resolve_color_choice(choice)?;
        }
        if group.border_color.is_some() || group.border_lineweight.is_some() {
            let color = match group.border_color {
                Some(choice) => Some(resolve_color_choice(choice)?),
                None => None,
            };
            let lineweight = match group.border_lineweight {
                Some(choice) => Some(resolve_lineweight_choice(choice)?),
                None => None,
            };
            for edge in [
                &mut row.borders.top,
                &mut row.borders.right,
                &mut row.borders.bottom,
                &mut row.borders.left,
            ] {
                if let Some(color) = color {
                    edge.color = color;
                }
                if let Some(lineweight) = lineweight {
                    edge.lineweight_hundredths = lineweight;
                }
            }
        }
        for (edge, visible, spacing) in &group.borders {
            let spacing = spacing.unwrap_or(0.0);
            if !spacing.is_finite() || spacing < 0.0 {
                return Err(table_refusal(
                    "tablestyle.invalid",
                    format!(
                        "the double-line spacing must be finite and non-negative, got {spacing}"
                    ),
                ));
            }
            // A visibility/spacing assignment keeps the edge's stored colour and
            // lineweight: the geometry command does not clear an appearance.
            let template = match edge {
                TableStyleEdge::Top => row.borders.top,
                TableStyleEdge::Right => row.borders.right,
                TableStyleEdge::Bottom => row.borders.bottom,
                TableStyleEdge::Left => row.borders.left,
                TableStyleEdge::All => row.borders.top,
            };
            let border = cad_domain::TableBorder {
                visible: *visible,
                double_spacing: spacing,
                ..template
            };
            match edge {
                TableStyleEdge::Top => row.borders.top = border,
                TableStyleEdge::Right => row.borders.right = border,
                TableStyleEdge::Bottom => row.borders.bottom = border,
                TableStyleEdge::Left => row.borders.left = border,
                TableStyleEdge::All => {
                    row.borders = cad_domain::TableCellBorders {
                        top: border,
                        right: border,
                        bottom: border,
                        left: border,
                    }
                }
            }
        }
    }
    for (kind, row) in rows {
        style.set_row_style(kind, row);
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
                    || updated.alignment != cell.alignment
                    || updated.text_color != cell.text_color
                    || updated.fill_color != cell.fill_color
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
        TablePayload::Format { .. } => "FORMAT",
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

/// The `TableFieldEdit` a typed colour assignment names.
///
/// `BYLAYER`/`NONE` clear the stored override; an ACI index resolves through the
/// crate's one palette table, and an index without a palette entry (only the
/// symbolic `0`/`256`, which the parser excludes) is refused by name.
fn color_edit(choice: Option<TableColorValue>) -> CadResult<TableFieldEdit<[u8; 3]>> {
    match choice {
        None => Ok(TableFieldEdit::Keep),
        Some(TableColorValue::None) => Ok(TableFieldEdit::Clear),
        Some(TableColorValue::Aci(index)) => {
            aci_srgb(index).map(TableFieldEdit::Set).ok_or_else(|| {
                table_refusal(
                    "tabledit.format_color",
                    format!("ACI index {index} is not a colour (0/256 are the symbolic variants)"),
                )
            })
        }
    }
}

/// The `TableFieldEdit` a typed lineweight assignment names.
///
/// The typed value is millimetres; the stored value is the DXF 1/100 mm integer,
/// so only a **canonical** AutoCAD weight can be expressed exactly and anything
/// else is refused by name instead of being snapped to a neighbour.
fn lineweight_edit(choice: Option<TableLineweightValue>) -> CadResult<TableFieldEdit<u16>> {
    match choice {
        None => Ok(TableFieldEdit::Keep),
        Some(TableLineweightValue::ByLayer) => Ok(TableFieldEdit::Clear),
        Some(TableLineweightValue::Millimetres(millimetres)) => {
            canonical_lineweight_hundredths(millimetres)
                .map(TableFieldEdit::Set)
                .ok_or_else(|| {
                    table_refusal(
                        "tabledit.format_lineweight",
                        format!(
                            "{millimetres} mm is not a canonical AutoCAD lineweight; the DXF \
                             border groups carry 1/100 mm integers, so only the canonical list \
                             (0.00, 0.05, 0.09, 0.13, 0.15, 0.18, 0.20, 0.25, 0.30, 0.35, 0.40, \
                             0.50, 0.53, 0.60, 0.70, 0.80, 0.90, 1.00, 1.06, 1.20, 1.40, 1.58, \
                             2.00, 2.11) can be stored exactly"
                        ),
                    )
                })
        }
    }
}

/// A colour assignment resolved for a `TABLESTYLE` row value set (`None` clears
/// the row's value, matching the cell writer).
fn resolve_color_choice(choice: TableColorValue) -> CadResult<Option<[u8; 3]>> {
    match color_edit(Some(choice))? {
        TableFieldEdit::Clear => Ok(None),
        TableFieldEdit::Set(rgb) => Ok(Some(rgb)),
        TableFieldEdit::Keep => Ok(None),
    }
}

/// A border-lineweight assignment resolved for a `TABLESTYLE` row value set.
fn resolve_lineweight_choice(choice: TableLineweightValue) -> CadResult<Option<u16>> {
    match lineweight_edit(Some(choice))? {
        TableFieldEdit::Clear => Ok(None),
        TableFieldEdit::Set(hundredths) => Ok(Some(hundredths)),
        TableFieldEdit::Keep => Ok(None),
    }
}

/// `BYLAYER`/`NONE` for an absent override, `#RRGGBB` for a stored colour.
fn color_label(color: Option<[u8; 3]>, absent: &str) -> String {
    match color {
        None => absent.to_string(),
        Some([r, g, b]) => format!("#{r:02X}{g:02X}{b:02X}"),
    }
}

/// The four edges' colours, in top/right/bottom/left order.
fn borders_color_label(borders: &cad_domain::TableCellBorders) -> String {
    format!(
        "{}/{}/{}/{}",
        color_label(borders.top.color, "BYLAYER"),
        color_label(borders.right.color, "BYLAYER"),
        color_label(borders.bottom.color, "BYLAYER"),
        color_label(borders.left.color, "BYLAYER")
    )
}

/// The four edges' lineweights, in top/right/bottom/left order.
fn borders_lineweight_label(borders: &cad_domain::TableCellBorders) -> String {
    let label = |weight: Option<u16>| match weight {
        None => "BYLAYER".to_string(),
        Some(hundredths) => format!("{:.2}mm", f64::from(hundredths) / 100.0),
    };
    format!(
        "{}/{}/{}/{}",
        label(borders.top.lineweight_hundredths),
        label(borders.right.lineweight_hundredths),
        label(borders.bottom.lineweight_hundredths),
        label(borders.left.lineweight_hundredths)
    )
}
