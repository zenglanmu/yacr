//! `TABLE` / `-TABLE` creation and `TABLEDIT` cell-text editing on the existing
//! [`SemanticGeometry::Table`](cad_domain::SemanticGeometry::Table) model.
//!
//! Both commands are **Work-only** and write through the normal drawing path.
//! Creation inserts one real `TableGeometry` through the shared
//! [`Application::commit_new_entity`] funnel (active layer, model space, one
//! transaction / one undo step). `TABLEDIT` is a pre-selection edit: it re-reads
//! the one selected table inside the command, replaces exactly one cell's text
//! with [`cad_domain::TableGeometry::with_cell_text`] (which rebuilds the derived
//! display) and stages one `update_entity` in one transaction / one undo step.
//!
//! The typed creation grammar is the declared v1 form (AutoCAD opens a dialog):
//!
//! `TABLE <rows> <cols> <col_width> <row_height> <x> <y>`
//!
//! The table is built on the world XY plane (`+X` horizontal, `+Z` normal) with
//! empty cells, the `Standard` style, [`cad_domain::DEFAULT_CELL_TEXT_HEIGHT`]
//! and single visible borders on all four edges. There are no merges.
//!
//! `TABLEDIT <row> <col> [text...]` addresses a cell **1-based** (row, column);
//! an empty text remainder clears the cell. An identical string is
//! `tabledit.unchanged` and opens no transaction.
//!
//! Stable refusal codes (every one happens before a transaction, so a refused
//! command writes nothing, bumps no revision and records no history):
//!
//! * `table.invalid_dimensions` — zero rows or columns;
//! * `table.create_too_large` — more than [`MAX_TABLE_ROWS`]/[`MAX_TABLE_COLS`]
//!   rows/columns or more than [`MAX_TABLE_CELLS`] total cells;
//! * `table.invalid_origin` — a non-finite origin;
//! * `table.invalid_size` — a non-finite or non-positive column width / row
//!   height, or a non-finite total extent;
//! * `tabledit.payload` / `.no_selection` / `.multiple` / `.not_table` /
//!   `.out_of_range`.
//!
//! The post-creation table edits live in [`super::table_ops`] (`TABLEDIT
//! MERGE|UNMERGE`, `MATCHCELL`, `TABLESTYLE`) and share this module's selection
//! rule and commit funnel. `TABLEEXPORT` is a typed command-line export in
//! `cad_ui_core::session::table_export` (one CSV file through the host's
//! user-named-file write channel). `TINSERT` (a block inside a cell) and
//! `FIELD`/`UPDATEFIELD` stay named refusals: this build has no block-in-cell
//! and no field model.

use super::*;
use crate::command_payload::{TableCreateCommand, TableEditCommand};
use cad_domain::{
    TableBorder, TableCellBorders, TableCellGeometry, TableColumnGeometry, TableFrame,
    TableGeometry, TableParts, TableRowGeometry, DEFAULT_CELL_TEXT_HEIGHT,
};

/// The maximum number of rows a `TABLE` command creates.
pub const MAX_TABLE_ROWS: u32 = 1000;
/// The maximum number of columns a `TABLE` command creates.
pub const MAX_TABLE_COLS: u32 = 1000;
/// The maximum total cell count (`rows * cols`) a `TABLE` command creates.
pub const MAX_TABLE_CELLS: u32 = 100_000;

impl Application {
    /// Dispatch one table command (Work-only).
    pub(crate) fn table_edit(
        &mut self,
        session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::TableCreate => self.table_create(session, command, false),
            ExtendedCommandId::DashTable => self.table_create(session, command, true),
            ExtendedCommandId::TableEdit => {
                // The cell-text form keeps its own payload; the merge forms carry
                // the shared table payload, so the two never mix.
                match &command.payload {
                    CommandPayload::Table(_) => self.table_ops(session, command, ext),
                    _ => self.table_edit_cell(session, command),
                }
            }
            ExtendedCommandId::MatchCell | ExtendedCommandId::TableStyleList => {
                self.table_ops(session, command, ext)
            }
            other => Err(CadError::Invariant(format!(
                "not a table command: {other:?}"
            ))),
        }
    }

    /// `TABLE <rows> <cols> <col_width> <row_height> <x> <y>`.
    ///
    /// Builds one empty table grid and inserts it through the shared create
    /// funnel, so it lands on the active layer in model space in one transaction
    /// / one undo step. Every named refusal is checked before that transaction.
    fn table_create(
        &mut self,
        session: &SessionState,
        command: &Command,
        dash: bool,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::TableCreate(payload) = &command.payload else {
            return Err(table_refusal(
                "table.payload",
                "TABLE needs a TableCreate payload (rows, cols, col_width, row_height, origin)",
            ));
        };
        let TableCreateCommand {
            rows,
            cols,
            col_width,
            row_height,
            origin,
        } = *payload.as_ref();
        if rows == 0 || cols == 0 {
            return Err(table_refusal(
                "table.invalid_dimensions",
                format!("a table needs at least one row and one column, got {rows} x {cols}"),
            ));
        }
        if rows > MAX_TABLE_ROWS || cols > MAX_TABLE_COLS {
            return Err(table_refusal(
                "table.create_too_large",
                format!(
                    "the table is {rows} x {cols}; this build creates at most \
                     {MAX_TABLE_ROWS} x {MAX_TABLE_COLS} rows/columns"
                ),
            ));
        }
        let cells = u64::from(rows) * u64::from(cols);
        if cells > u64::from(MAX_TABLE_CELLS) {
            return Err(table_refusal(
                "table.create_too_large",
                format!(
                    "the table has {cells} cells; this build creates at most \
                     {MAX_TABLE_CELLS}"
                ),
            ));
        }
        if !is_finite_point(origin) {
            return Err(table_refusal(
                "table.invalid_origin",
                format!("the table origin {origin:?} is not finite"),
            ));
        }
        if !col_width.is_finite() || col_width <= 0.0 {
            return Err(table_refusal(
                "table.invalid_size",
                format!("the column width must be finite and positive, got {col_width}"),
            ));
        }
        if !row_height.is_finite() || row_height <= 0.0 {
            return Err(table_refusal(
                "table.invalid_size",
                format!("the row height must be finite and positive, got {row_height}"),
            ));
        }
        let total_width = f64::from(cols) * col_width;
        let total_height = f64::from(rows) * row_height;
        if !total_width.is_finite() || !total_height.is_finite() {
            return Err(table_refusal(
                "table.invalid_size",
                format!(
                    "the total extent {total_width} x {total_height} is not finite; reduce the \
                     cell size or the table dimensions"
                ),
            ));
        }
        let geometry = TableGeometry::new(table_parts(rows, cols, col_width, row_height, origin))?;
        let label = if dash {
            "create table (-TABLE)"
        } else {
            "create table"
        };
        let mut outcome = self.commit_new_entity(
            session,
            command,
            SemanticGeometry::Table(Box::new(geometry)),
            label,
        )?;
        let code = if dash {
            "-table.created"
        } else {
            "table.created"
        };
        let name = if dash { "-TABLE" } else { "TABLE" };
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: code.into(),
            message: format!(
                "{name}: created a {rows} x {cols} table at ({}, {}, {}) with column width \
                 {col_width} and row height {row_height} (total {total_width} x {total_height}); \
                 all cells are empty with the Standard style and single visible borders, and no \
                 merge spans. Cell text: `TABLEDIT <row> <col> <text>`; merged ranges: \
                 `TABLEDIT MERGE|UNMERGE`; another style: `TABLESTYLE <name> [<field> <value> ...]` \
                 (block content and fields stay named refusals)",
                origin.x, origin.y, origin.z
            ),
        });
        Ok(outcome)
    }

    /// `TABLEDIT <row> <col> [text...]`.
    ///
    /// Pre-selection: exactly one selected, directly addressable `Table` entity.
    /// The cell address is **1-based**; the text is the remainder of the typed
    /// line (leading whitespace already trimmed by the parser). An empty text
    /// clears the cell and an identical string is a no-op.
    fn table_edit_cell(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::TableEdit(payload) = &command.payload else {
            return Err(table_refusal(
                "tabledit.payload",
                "TABLEDIT needs a TableEdit payload (row, col, text)",
            ));
        };
        let TableEditCommand { row, col, text } = payload.as_ref();
        let selection = select_table(self, session, command, "tabledit")?;
        let table = &selection.table;
        // The payload is 1-based; the grid is 0-based. Zero is outside the
        // 1-based address space, so it is the same `out_of_range` refusal.
        let row_index = row.checked_sub(1);
        let column_index = col.checked_sub(1);
        let Some((row_index, column_index)) = row_index
            .zip(column_index)
            .map(|(r, c)| (r as usize, c as usize))
        else {
            return Err(table_out_of_range(row, col, table));
        };
        let Some(cell) = table.cell(row_index, column_index) else {
            return Err(table_out_of_range(row, col, table));
        };
        if cell.text == *text {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: Some(ObjectId(selection.before.id.0)),
                    code: "tabledit.unchanged".into(),
                    message: format!(
                        "TABLEDIT: cell ({row}, {col}) of table {} already holds `{text}`; nothing \
                         was written",
                        selection.before.id.0
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let updated = table.with_cell_text(row_index, column_index, text)?;
        commit_table_geometry(
            self,
            command.document,
            &selection,
            updated,
            "edit table cell",
            "tabledit.cell",
            format!(
                "TABLEDIT: set cell (row {row}, column {col}) of table {} to `{text}`; one \
                 transaction / one undo step",
                selection.before.id.0
            ),
        )
    }
}

/// The one selected table a table edit addresses, already resolved.
///
/// `before` is the stored entity and `table` its definition, both cloned out of
/// the document so the caller can take `&mut self` for the commit without holding
/// a borrow of the workspace.
pub(crate) struct TableSelection {
    pub before: DbEntity,
    pub table: TableGeometry,
}

/// Resolve the pre-selection every table edit shares.
///
/// Exactly one selected reference, directly addressable (no block instance, no
/// sub-element), in the command's document, and existing with a `Table`
/// geometry. `prefix` is the command's stable code prefix (`tabledit`,
/// `matchcell`, `tablestyle`), so the same rule answers the same shape of code
/// for every table command:
///
/// * `{prefix}.no_selection` — nothing selected;
/// * `{prefix}.multiple` — more than one reference;
/// * `{prefix}.not_table` — a block member, a sub-element, a foreign document or
///   a non-table geometry.
pub(crate) fn select_table(
    app: &Application,
    session: &SessionState,
    command: &Command,
    prefix: &str,
) -> CadResult<TableSelection> {
    let references = session.selection.refs();
    let reference = match references {
        [] => {
            return Err(table_refusal(
                format!("{prefix}.no_selection"),
                "this command needs exactly one selected table; nothing is selected",
            ));
        }
        [one] => one,
        many => {
            return Err(table_refusal(
                format!("{prefix}.multiple"),
                format!(
                    "this command edits exactly one table; {} entities are selected",
                    many.len()
                ),
            ));
        }
    };
    if reference.document != command.document
        || !reference.instance.0.is_empty()
        || reference.sub_element.is_some()
    {
        return Err(table_refusal(
            format!("{prefix}.not_table"),
            format!(
                "entity {} is not a directly addressable table in this document",
                reference.entity.0
            ),
        ));
    }
    let document = app
        .workspace
        .documents
        .get(&command.document)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let before = document
        .drawing
        .entity(reference.entity)
        .cloned()
        .ok_or_else(|| {
            table_refusal(
                format!("{prefix}.not_table"),
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
    let table = match &before.geometry {
        SemanticGeometry::Table(table) => table.as_ref().clone(),
        other => {
            return Err(table_refusal(
                format!("{prefix}.not_table"),
                format!(
                    "entity {} is a {}, not a table",
                    before.id.0,
                    class_name(other)
                ),
            ))
        }
    };
    Ok(TableSelection { before, table })
}

/// Stage one table definition change: one `update_entity` in one transaction /
/// one undo step, with the same before/after patches every other edit records.
#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_table_geometry(
    app: &mut Application,
    document_id: DocumentId,
    selection: &TableSelection,
    updated: TableGeometry,
    label: &str,
    code: &str,
    message: String,
) -> CadResult<CommandOutcome> {
    let mut candidate = {
        let document = app
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        document.drawing.as_ref().clone()
    };
    let mut after = selection.before.clone();
    after.geometry = SemanticGeometry::Table(Box::new(updated));
    let transaction = next_transaction_id();
    let change_set = {
        let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
        staged.update_entity(after)?;
        staged.commit()?
    };
    let stored = candidate
        .entity(selection.before.id)
        .cloned()
        .ok_or_else(|| CadError::Invariant(format!("{code}: the table vanished after commit")))?;
    app.publish_candidate(
        document_id,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: label.into(),
            patches: vec![drawing_patch(
                selection.before.id,
                Some(selection.before.clone()),
                Some(stored),
            )],
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            style_patches: Vec::new(),
            dim_style_patches: Vec::new(),
            mleader_style_patches: Vec::new(),
            layout_patches: Vec::new(),
            merge_key: None,
        },
    )?;
    Ok(CommandOutcome {
        objects: vec![ObjectId(selection.before.id.0)],
        changes: Some(Arc::new(change_set)),
        diagnostics: vec![Diagnostic {
            object: Some(ObjectId(selection.before.id.0)),
            code: code.into(),
            message,
        }],
        measurement: None,
        point_report: None,
    })
}

/// The empty grid a `TABLE` command creates.
///
/// Every cell is empty with the default text height, the `Standard` style and
/// single visible borders; no merge spans are set.
fn table_parts(
    rows: u32,
    cols: u32,
    col_width: f64,
    row_height: f64,
    origin: Point3,
) -> TableParts {
    let columns = (0..cols)
        .map(|_| TableColumnGeometry {
            width: col_width,
            name: String::new(),
        })
        .collect();
    let rows = (0..rows)
        .map(|_| TableRowGeometry {
            height: row_height,
            cells: (0..cols).map(|_| empty_cell()).collect(),
        })
        .collect();
    TableParts {
        frame: TableFrame {
            origin,
            horizontal: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        },
        columns,
        rows,
        style_name: "Standard".into(),
        name: String::new(),
        description: String::new(),
    }
}

/// One empty cell with the created table's default appearance.
fn empty_cell() -> TableCellGeometry {
    TableCellGeometry {
        text: String::new(),
        text_height: DEFAULT_CELL_TEXT_HEIGHT,
        style_name: "Standard".into(),
        style: StyleId(0),
        font: None,
        alignment: cad_domain::TableCellAlignment::DEFAULT,
        rotation: 0.0,
        merge_width: 1,
        merge_height: 1,
        borders: TableCellBorders {
            top: TableBorder::VISIBLE,
            right: TableBorder::VISIBLE,
            bottom: TableBorder::VISIBLE,
            left: TableBorder::VISIBLE,
        },
        // No per-cell appearance: every cell draws in the entity's resolved
        // colour with no background, exactly like a table created before the
        // per-cell colours existed.
        text_color: None,
        fill_color: None,
    }
}

/// The `tabledit.out_of_range` refusal for a 1-based address.
fn table_out_of_range(row: &u32, col: &u32, table: &TableGeometry) -> CadError {
    table_refusal(
        "tabledit.out_of_range",
        format!(
            "cell ({row}, {col}) is outside the table's {} rows x {} columns (row and column \
             are 1-based)",
            table.rows().len(),
            table.columns().len()
        ),
    )
}

/// One stable, code-prefixed refusal for a table command.
pub(crate) fn table_refusal(
    code: impl std::fmt::Display,
    detail: impl std::fmt::Display,
) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}
