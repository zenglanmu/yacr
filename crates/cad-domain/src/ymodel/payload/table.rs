//! The `cad:Table` payload (ADR-0037 D8).
//!
//! A table stores its plane frame, the column/row grid with every cell's content
//! and resolved appearance, the `TABLESTYLE` snapshot, the sparse per-cell
//! margin overrides, the linked-data name/description **and the derived display
//! cache**, primitives and their per-primitive appearance in lockstep.
//!
//! Phase 1 persists that cache verbatim: `TableGeometry::new` validates the grid
//! and rebuilds the cache, and `from_import` clamps merge spans and rebuilds it.
//! Restoring through either would replace stored display data on the first save
//! (ADR-0037 D8) and could reject a legacy grid, so the reader calls
//! `TableGeometry::from_persisted`.
//!
//! The margin overrides are stored as a flat list of addressed cells — the same
//! sparse shape the model keeps (a cell with no override has no entry), not a
//! dense grid.

use serde_json::Value;

use super::{fields, object, PayloadError};
use crate::ymodel::json::{self, ObjectReader};
use crate::{
    SemanticGeometry, TableBorder, TableCellAlignH, TableCellAlignV, TableCellAlignment,
    TableCellBorders, TableCellGeometry, TableCellMargins, TableColumnGeometry,
    TableDisplayAppearance, TableFrame, TableGeometry, TableRowGeometry, TableRowStyle,
    TableStyleValues,
};

// ---------------------------------------------------------------------------
// The entity.
// ---------------------------------------------------------------------------

pub(super) fn encode_table(table: &TableGeometry) -> Value {
    let (geometry, appearance) = table.display_parts();
    object(
        "cad:Table",
        vec![
            ("frame", encode_frame(table.frame())),
            (
                "columns",
                Value::Array(table.columns().iter().map(encode_column).collect()),
            ),
            (
                "rows",
                Value::Array(table.rows().iter().map(encode_row).collect()),
            ),
            ("style", encode_style(table.style())),
            (
                "margins",
                Value::Array(
                    table
                        .margin_overrides()
                        .map(|(address, margins)| encode_margin_entry(address, margins))
                        .collect(),
                ),
            ),
            ("name", Value::String(table.name().to_string())),
            (
                "description",
                Value::String(table.description().to_string()),
            ),
            (
                "display",
                fields(vec![
                    (
                        "geometry",
                        Value::Array(geometry.iter().map(super::encode).collect()),
                    ),
                    (
                        "appearance",
                        Value::Array(appearance.iter().map(encode_appearance).collect()),
                    ),
                ]),
            ),
        ],
    )
}

pub(super) fn decode_table(value: &Value) -> Result<SemanticGeometry, PayloadError> {
    let mut reader = ObjectReader::payload(value, "cad:Table")?;
    let frame = decode_frame(&reader.take("frame")?)?;
    let raw_columns = reader.take("columns")?;
    let columns = match &raw_columns {
        Value::Array(items) => items.iter().map(decode_column).collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("cad:Table.columns: expected an array"));
        }
    };
    let raw_rows = reader.take("rows")?;
    let rows = match &raw_rows {
        Value::Array(items) => items.iter().map(decode_row).collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("cad:Table.rows: expected an array"));
        }
    };
    let style = decode_style(&reader.take("style")?)?;
    let raw_margins = reader.take("margins")?;
    let margins = match &raw_margins {
        Value::Array(items) => items
            .iter()
            .map(decode_margin_entry)
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(PayloadError::new(
                "cad:Table.margins: expected an array of addressed cells",
            ));
        }
    };
    let name = reader.string("name")?;
    let description = reader.string("description")?;
    let display = decode_display(&reader.take("display")?)?;
    reader.finish()?;
    Ok(SemanticGeometry::Table(Box::new(
        TableGeometry::from_persisted(
            frame,
            columns,
            rows,
            style,
            margins,
            name,
            description,
            display.0,
            display.1,
        ),
    )))
}

// ---------------------------------------------------------------------------
// Frame, columns, rows and cells.
// ---------------------------------------------------------------------------

fn encode_frame(frame: TableFrame) -> Value {
    fields(vec![
        ("origin", json::encode_point3(frame.origin)),
        ("horizontal", json::encode_point3(frame.horizontal)),
        ("normal", json::encode_point3(frame.normal)),
    ])
}

fn decode_frame(value: &Value) -> Result<TableFrame, PayloadError> {
    let mut reader = ObjectReader::new(value, "frame")?;
    let frame = TableFrame {
        origin: reader.point3("origin")?,
        horizontal: reader.point3("horizontal")?,
        normal: reader.point3("normal")?,
    };
    reader.finish()?;
    Ok(frame)
}

fn encode_column(column: &TableColumnGeometry) -> Value {
    fields(vec![
        ("width", json::encode_f64(column.width)),
        ("name", Value::String(column.name.clone())),
    ])
}

fn decode_column(value: &Value) -> Result<TableColumnGeometry, PayloadError> {
    let mut reader = ObjectReader::new(value, "column")?;
    let column = TableColumnGeometry {
        width: reader.f64("width")?,
        name: reader.string("name")?,
    };
    reader.finish()?;
    Ok(column)
}

fn encode_row(row: &TableRowGeometry) -> Value {
    fields(vec![
        ("height", json::encode_f64(row.height)),
        (
            "cells",
            Value::Array(row.cells.iter().map(encode_cell).collect()),
        ),
    ])
}

fn decode_row(value: &Value) -> Result<TableRowGeometry, PayloadError> {
    let mut reader = ObjectReader::new(value, "row")?;
    let height = reader.f64("height")?;
    let raw_cells = reader.take("cells")?;
    let cells = match &raw_cells {
        Value::Array(items) => items.iter().map(decode_cell).collect::<Result<_, _>>()?,
        _ => {
            return Err(PayloadError::new("row.cells: expected an array"));
        }
    };
    reader.finish()?;
    Ok(TableRowGeometry { height, cells })
}

fn encode_cell(cell: &TableCellGeometry) -> Value {
    fields(vec![
        ("text", Value::String(cell.text.clone())),
        ("text_height", json::encode_f64(cell.text_height)),
        ("style_name", Value::String(cell.style_name.clone())),
        (
            "style",
            super::id_ref_value(crate::ymodel::id::IdKind::Style, cell.style.0),
        ),
        (
            "font",
            cell.font
                .as_ref()
                .map_or(Value::Null, |font| Value::String(font.clone())),
        ),
        ("alignment", encode_alignment(cell.alignment)),
        ("rotation", json::encode_f64(cell.rotation)),
        ("merge_width", Value::from(cell.merge_width)),
        ("merge_height", Value::from(cell.merge_height)),
        ("borders", encode_borders(&cell.borders)),
        ("text_color", encode_color(cell.text_color)),
        ("fill_color", encode_color(cell.fill_color)),
    ])
}

fn decode_cell(value: &Value) -> Result<TableCellGeometry, PayloadError> {
    let mut reader = ObjectReader::new(value, "cell")?;
    let cell = TableCellGeometry {
        text: reader.string("text")?,
        text_height: reader.f64("text_height")?,
        style_name: reader.string("style_name")?,
        style: crate::StyleId(reader.id_seq("style", crate::ymodel::id::IdKind::Style)?),
        font: reader.opt_string("font")?,
        alignment: decode_alignment(&reader.take("alignment")?)?,
        rotation: reader.f64("rotation")?,
        merge_width: reader.u32("merge_width")?,
        merge_height: reader.u32("merge_height")?,
        borders: decode_borders(&reader.take("borders")?)?,
        text_color: reader.opt_u8x3("text_color")?,
        fill_color: reader.opt_u8x3("fill_color")?,
    };
    reader.finish()?;
    Ok(cell)
}

// ---------------------------------------------------------------------------
// Alignment, borders and colour.
// ---------------------------------------------------------------------------

fn encode_alignment(alignment: TableCellAlignment) -> Value {
    fields(vec![
        (
            "horizontal",
            Value::String(
                match alignment.horizontal {
                    TableCellAlignH::Left => "left",
                    TableCellAlignH::Center => "center",
                    TableCellAlignH::Right => "right",
                }
                .to_string(),
            ),
        ),
        (
            "vertical",
            Value::String(
                match alignment.vertical {
                    TableCellAlignV::Top => "top",
                    TableCellAlignV::Middle => "middle",
                    TableCellAlignV::Bottom => "bottom",
                }
                .to_string(),
            ),
        ),
    ])
}

fn decode_alignment(value: &Value) -> Result<TableCellAlignment, PayloadError> {
    let mut reader = ObjectReader::new(value, "alignment")?;
    let horizontal = match reader.string("horizontal")?.as_str() {
        "left" => TableCellAlignH::Left,
        "center" => TableCellAlignH::Center,
        "right" => TableCellAlignH::Right,
        other => {
            return Err(PayloadError::new(format!(
                "alignment.horizontal: unknown alignment {other:?}"
            )));
        }
    };
    let vertical = match reader.string("vertical")?.as_str() {
        "top" => TableCellAlignV::Top,
        "middle" => TableCellAlignV::Middle,
        "bottom" => TableCellAlignV::Bottom,
        other => {
            return Err(PayloadError::new(format!(
                "alignment.vertical: unknown alignment {other:?}"
            )));
        }
    };
    reader.finish()?;
    Ok(TableCellAlignment {
        horizontal,
        vertical,
    })
}

fn encode_borders(borders: &TableCellBorders) -> Value {
    fields(vec![
        ("top", encode_border(borders.top)),
        ("right", encode_border(borders.right)),
        ("bottom", encode_border(borders.bottom)),
        ("left", encode_border(borders.left)),
    ])
}

fn decode_borders(value: &Value) -> Result<TableCellBorders, PayloadError> {
    let mut reader = ObjectReader::new(value, "borders")?;
    let borders = TableCellBorders {
        top: decode_border(&reader.take("top")?)?,
        right: decode_border(&reader.take("right")?)?,
        bottom: decode_border(&reader.take("bottom")?)?,
        left: decode_border(&reader.take("left")?)?,
    };
    reader.finish()?;
    Ok(borders)
}

fn encode_border(border: TableBorder) -> Value {
    fields(vec![
        ("visible", Value::Bool(border.visible)),
        ("double_spacing", json::encode_f64(border.double_spacing)),
        ("color", encode_color(border.color)),
        (
            "lineweight_hundredths",
            border
                .lineweight_hundredths
                .map_or(Value::Null, Value::from),
        ),
    ])
}

fn decode_border(value: &Value) -> Result<TableBorder, PayloadError> {
    let mut reader = ObjectReader::new(value, "border")?;
    let border = TableBorder {
        visible: reader.bool("visible")?,
        double_spacing: reader.f64("double_spacing")?,
        color: reader.opt_u8x3("color")?,
        lineweight_hundredths: reader.opt_u16("lineweight_hundredths")?,
    };
    reader.finish()?;
    Ok(border)
}

fn encode_color(color: Option<[u8; 3]>) -> Value {
    color.map_or(Value::Null, |color| {
        Value::Array(color.iter().map(|byte| Value::from(*byte)).collect())
    })
}

// ---------------------------------------------------------------------------
// Style snapshot and row value sets.
// ---------------------------------------------------------------------------

fn encode_style(style: &TableStyleValues) -> Value {
    fields(vec![
        ("name", Value::String(style.name.clone())),
        ("description", Value::String(style.description.clone())),
        (
            "horizontal_margin",
            json::encode_f64(style.horizontal_margin),
        ),
        ("vertical_margin", json::encode_f64(style.vertical_margin)),
        ("text_height", json::encode_f64(style.text_height)),
        (
            "text_style_name",
            Value::String(style.text_style_name.clone()),
        ),
        ("borders", encode_borders(&style.borders)),
        (
            "data_row_alignment",
            encode_alignment(style.data_row_alignment),
        ),
        ("text_color", encode_color(style.text_color)),
        ("fill_color", encode_color(style.fill_color)),
        (
            "title_row",
            style
                .title_row
                .as_ref()
                .map_or(Value::Null, encode_row_style),
        ),
        (
            "header_row",
            style
                .header_row
                .as_ref()
                .map_or(Value::Null, encode_row_style),
        ),
        ("title_suppressed", Value::Bool(style.title_suppressed)),
        ("header_suppressed", Value::Bool(style.header_suppressed)),
    ])
}

fn decode_style(value: &Value) -> Result<TableStyleValues, PayloadError> {
    let mut reader = ObjectReader::new(value, "style")?;
    let name = reader.string("name")?;
    let description = reader.string("description")?;
    let horizontal_margin = reader.f64("horizontal_margin")?;
    let vertical_margin = reader.f64("vertical_margin")?;
    let text_height = reader.f64("text_height")?;
    let text_style_name = reader.string("text_style_name")?;
    let borders = decode_borders(&reader.take("borders")?)?;
    let data_row_alignment = decode_alignment(&reader.take("data_row_alignment")?)?;
    let text_color = reader.opt_u8x3("text_color")?;
    let fill_color = reader.opt_u8x3("fill_color")?;
    let title_row = match reader.take("title_row")? {
        Value::Null => None,
        value => Some(decode_row_style(&value)?),
    };
    let header_row = match reader.take("header_row")? {
        Value::Null => None,
        value => Some(decode_row_style(&value)?),
    };
    let title_suppressed = reader.bool("title_suppressed")?;
    let header_suppressed = reader.bool("header_suppressed")?;
    reader.finish()?;
    Ok(TableStyleValues {
        name,
        description,
        horizontal_margin,
        vertical_margin,
        text_height,
        text_style_name,
        borders,
        data_row_alignment,
        text_color,
        fill_color,
        title_row,
        header_row,
        title_suppressed,
        header_suppressed,
    })
}

fn encode_row_style(row: &TableRowStyle) -> Value {
    fields(vec![
        ("text_height", json::encode_f64(row.text_height)),
        (
            "text_style_name",
            Value::String(row.text_style_name.clone()),
        ),
        ("alignment", encode_alignment(row.alignment)),
        ("borders", encode_borders(&row.borders)),
        ("text_color", encode_color(row.text_color)),
        ("fill_color", encode_color(row.fill_color)),
    ])
}

fn decode_row_style(value: &Value) -> Result<TableRowStyle, PayloadError> {
    let mut reader = ObjectReader::new(value, "row_style")?;
    let row = TableRowStyle {
        text_height: reader.f64("text_height")?,
        text_style_name: reader.string("text_style_name")?,
        alignment: decode_alignment(&reader.take("alignment")?)?,
        borders: decode_borders(&reader.take("borders")?)?,
        text_color: reader.opt_u8x3("text_color")?,
        fill_color: reader.opt_u8x3("fill_color")?,
    };
    reader.finish()?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Margin overrides.
// ---------------------------------------------------------------------------

fn encode_margin_entry(address: (usize, usize), margins: TableCellMargins) -> Value {
    fields(vec![
        ("row", Value::from(address.0 as u64)),
        ("column", Value::from(address.1 as u64)),
        ("left", json::encode_opt_f64(margins.left)),
        ("top", json::encode_opt_f64(margins.top)),
        ("right", json::encode_opt_f64(margins.right)),
        ("bottom", json::encode_opt_f64(margins.bottom)),
    ])
}

fn decode_margin_entry(value: &Value) -> Result<((usize, usize), TableCellMargins), PayloadError> {
    let mut reader = ObjectReader::new(value, "margin")?;
    let address = (reader.usize("row")?, reader.usize("column")?);
    let margins = TableCellMargins {
        left: reader.opt_f64("left")?,
        top: reader.opt_f64("top")?,
        right: reader.opt_f64("right")?,
        bottom: reader.opt_f64("bottom")?,
    };
    reader.finish()?;
    Ok((address, margins))
}

// ---------------------------------------------------------------------------
// Derived display cache.
// ---------------------------------------------------------------------------

fn encode_appearance(appearance: &TableDisplayAppearance) -> Value {
    fields(vec![
        ("color", encode_color(appearance.color)),
        (
            "lineweight_hundredths",
            appearance
                .lineweight_hundredths
                .map_or(Value::Null, Value::from),
        ),
    ])
}

fn decode_appearance(value: &Value) -> Result<TableDisplayAppearance, PayloadError> {
    let mut reader = ObjectReader::new(value, "appearance")?;
    let appearance = TableDisplayAppearance {
        color: reader.opt_u8x3("color")?,
        lineweight_hundredths: reader.opt_u16("lineweight_hundredths")?,
    };
    reader.finish()?;
    Ok(appearance)
}

fn decode_display(
    value: &Value,
) -> Result<(Vec<SemanticGeometry>, Vec<TableDisplayAppearance>), PayloadError> {
    let mut reader = ObjectReader::new(value, "display")?;
    let raw_geometry = reader.take("geometry")?;
    let geometry = match &raw_geometry {
        Value::Array(items) => items
            .iter()
            .map(super::decode)
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(PayloadError::new("display.geometry: expected an array"));
        }
    };
    let raw_appearance = reader.take("appearance")?;
    let appearance = match &raw_appearance {
        Value::Array(items) => items
            .iter()
            .map(decode_appearance)
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(PayloadError::new("display.appearance: expected an array"));
        }
    };
    reader.finish()?;
    if geometry.len() != appearance.len() {
        return Err(PayloadError::new(format!(
            "display: {} primitives but {} appearances (they are stored in lockstep)",
            geometry.len(),
            appearance.len()
        )));
    }
    Ok((geometry, appearance))
}
