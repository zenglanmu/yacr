//! The TABLE export contract tests: the legacy entity record, the
//! `TABLECONTENT` companion (linked-data name/description, per-cell
//! `GRIDFORMAT` spacing and `CELLMARGIN` margins) and the round trips.
//!
//! Split out of `export/table.rs` for the code-size ceiling; the module is a
//! child of [`super::table`] so every helper it pins stays in one place.
//!
//! The per-cell/row appearance round trip is pinned here too: a stored colour,
//! fill, border colour and border lineweight must be **written as the DXF
//! groups** and read back unchanged, and a cell/row without an override must
//! write the neutral `ByBlock`/no-fill/`ByLayer` values (so a table that never
//! set one keeps its pre-appearance file byte-for-byte).

use super::*;
use crate::tests_multileader_fixtures::{export_one, import_bytes};

const RED: [u8; 3] = [255, 0, 0];
const YELLOW: [u8; 3] = [255, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const GREEN: [u8; 3] = [0, 255, 0];

fn cell(text: &str) -> TableCellGeometry {
    TableCellGeometry {
        text: text.to_string(),
        text_height: 2.5,
        style_name: "Standard".into(),
        style: StyleId(0),
        font: None,
        alignment: TableCellAlignment::DEFAULT,
        rotation: 0.0,
        merge_width: 1,
        merge_height: 1,
        borders: TableCellBorders {
            top: TableBorder::VISIBLE,
            right: TableBorder::VISIBLE,
            bottom: TableBorder::VISIBLE,
            left: TableBorder::VISIBLE,
        },
        text_color: None,
        fill_color: None,
    }
}

/// A 1 x 2 table whose first cell carries every per-cell appearance field and
/// whose style carries per-row appearance too. The second cell stays neutral.
fn styled_table() -> TableGeometry {
    let mut styled = cell("styled");
    styled.text_color = Some(RED);
    styled.fill_color = Some(YELLOW);
    for border in [
        &mut styled.borders.top,
        &mut styled.borders.right,
        &mut styled.borders.bottom,
        &mut styled.borders.left,
    ] {
        border.color = Some(BLUE);
        border.lineweight_hundredths = Some(50);
    }
    let mut style = TableStyleValues::named("Report");
    // Both other row types are suppressed, so every row of this table is a
    // data row: the plain cell inherits the data row's appearance and the
    // title value set is carried only as the style's own record.
    style.title_suppressed = true;
    style.header_suppressed = true;
    style.text_color = Some(GREEN);
    style.fill_color = Some(RED);
    style.borders.top.color = Some(YELLOW);
    style.borders.top.lineweight_hundredths = Some(35);
    let mut title = style.data_row();
    title.text_color = Some(BLUE);
    title.fill_color = None;
    style.set_row_style(TableRowKind::Title, title);
    TableGeometry::from_import_with_style(
        TableParts {
            frame: TableFrame::default(),
            columns: vec![
                TableColumnGeometry {
                    width: 4.0,
                    name: "A".into(),
                },
                TableColumnGeometry {
                    width: 4.0,
                    name: "B".into(),
                },
            ],
            rows: vec![TableRowGeometry {
                height: 3.0,
                cells: vec![styled, cell("plain")],
            }],
            style_name: "Report".into(),
            name: String::new(),
            description: String::new(),
        },
        style,
    )
}

fn written_table(bytes: &[u8]) -> acadrust::entities::Table {
    let document = acadrust::DxfReader::from_reader(std::io::Cursor::new(bytes.to_vec()))
        .expect("reader")
        .read()
        .expect("written DXF parses");
    let table = document
        .entities()
        .find_map(|entity| match entity {
            acadrust::EntityType::Table(table) => Some(table.as_ref().clone()),
            _ => None,
        })
        .expect("one TABLE entity");
    table
}

fn written_style(bytes: &[u8], name: &str) -> acadrust::objects::TableStyle {
    let document = acadrust::DxfReader::from_reader(std::io::Cursor::new(bytes.to_vec()))
        .expect("reader")
        .read()
        .expect("written DXF parses");
    let style = document
        .objects
        .iter()
        .find_map(|(_, object)| match object {
            ObjectType::TableStyle(style) if style.name.eq_ignore_ascii_case(name) => {
                Some(style.clone())
            }
            _ => None,
        })
        .expect("the TABLESTYLE record");
    style
}

#[test]
fn the_per_cell_appearance_is_written_as_its_dxf_groups() {
    let (bytes, report) = export_one(SemanticGeometry::Table(Box::new(styled_table())));
    let written = written_table(&bytes);
    let style = written.rows[0].cells[0]
        .style
        .as_ref()
        .expect("the styled cell keeps a CellStyle record");
    // Group 64 / 63 / 283: content colour, background colour, fill enabled.
    assert_eq!(style.content_color, acadrust::Color::Index(1), "red");
    assert_eq!(style.background_color, acadrust::Color::Index(2), "yellow");
    assert!(style.fill_enabled);
    // Groups 69/275 (top), 65/279 (right), 66/276 (bottom), 68/278 (left).
    for border in [
        &style.top_border,
        &style.right_border,
        &style.bottom_border,
        &style.left_border,
    ] {
        assert_eq!(border.color, acadrust::Color::Index(5), "blue");
        assert_eq!(border.line_weight, acadrust::LineWeight::Value(50));
    }

    // The neutral cell writes the neutral values, so nothing is implied.
    let plain = written.rows[0].cells[1]
        .style
        .as_ref()
        .expect("every cell gets a CellStyle record");
    assert_eq!(plain.content_color, acadrust::Color::ByBlock);
    assert_eq!(plain.background_color, acadrust::Color::ByBlock);
    assert!(!plain.fill_enabled);
    assert_eq!(plain.top_border.color, acadrust::Color::ByBlock);
    assert_eq!(plain.top_border.line_weight, acadrust::LineWeight::ByLayer);

    // The style object carries its row value sets (groups 62/63/283 and
    // 64-69/274-279 of each row record).
    let record = written_style(&bytes, "Report");
    assert_eq!(record.data_row_style.text_color, acadrust::Color::Index(3));
    assert_eq!(record.data_row_style.fill_color, acadrust::Color::Index(1));
    assert!(record.data_row_style.fill_enabled);
    assert_eq!(
        record.data_row_style.top_border.color,
        acadrust::Color::Index(2)
    );
    assert_eq!(
        record.data_row_style.top_border.line_weight,
        acadrust::LineWeight::Value(35)
    );
    assert_eq!(record.title_row_style.text_color, acadrust::Color::Index(5));
    assert!(!record.title_row_style.fill_enabled);

    // The conversion report no longer claims the fields are dropped.
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.message.contains("nearest ACI index")),
        "{report:?}"
    );
}

#[test]
fn the_per_cell_and_per_row_appearance_round_trips() {
    let source = styled_table();
    let (bytes, _report) = export_one(SemanticGeometry::Table(Box::new(source.clone())));
    let reimported = import_bytes(&bytes);
    let entity = reimported
        .model_space()
        .into_iter()
        .find(|entity| entity.object.type_key == "AcDbTable")
        .expect("AcDbTable re-imported");
    let SemanticGeometry::Table(table) = &entity.geometry else {
        panic!("expected a Table variant");
    };

    // Per cell: colour, fill and every edge's colour/lineweight.
    let styled = table.cell(0, 0).expect("styled cell");
    assert_eq!(styled.text_color, Some(RED));
    assert_eq!(styled.fill_color, Some(YELLOW));
    for border in [
        styled.borders.top,
        styled.borders.right,
        styled.borders.bottom,
        styled.borders.left,
    ] {
        assert_eq!(border.color, Some(BLUE));
        assert_eq!(border.lineweight_hundredths, Some(50));
    }
    // The plain cell inherited the data row's appearance (it never had an
    // override of its own), and the inherited values are real values.
    let plain = table.cell(0, 1).expect("plain cell");
    assert_eq!(plain.text_color, Some(GREEN));
    assert_eq!(plain.fill_color, Some(RED));
    assert_eq!(plain.borders.top.color, Some(YELLOW));
    assert_eq!(plain.borders.top.lineweight_hundredths, Some(35));

    // Per style row: the data row and the distinguished title row.
    let style = table.style();
    assert_eq!(style.text_color, Some(GREEN));
    assert_eq!(style.fill_color, Some(RED));
    assert_eq!(style.borders.top.color, Some(YELLOW));
    assert_eq!(style.borders.top.lineweight_hundredths, Some(35));
    let title = style.row_style(TableRowKind::Title);
    assert_eq!(title.text_color, Some(BLUE));
    assert_eq!(title.fill_color, None);
    // The header row never differed, so it stays linked to the data row.
    assert_eq!(style.header_row, None);

    // The derived display follows the re-imported appearance, so the colour
    // is not merely stored.
    let appearances = table.display_appearances();
    assert!(
        appearances
            .iter()
            .any(|appearance| appearance.color == Some(YELLOW)),
        "the fill colour reaches the display: {appearances:?}"
    );
    assert!(appearances
        .iter()
        .any(|appearance| appearance.color == Some(RED)));
}

/// A 1 x 2 table whose first cell stores a `GRIDFORMAT` margin override and a
/// doubled border, and whose second cell stores neither.
fn grid_format_table() -> TableGeometry {
    let mut styled = cell("styled");
    styled.borders.top = TableBorder {
        visible: true,
        double_spacing: 0.5,
        ..TableBorder::VISIBLE
    };
    let table = TableGeometry::from_import(TableParts {
        frame: TableFrame::default(),
        columns: vec![
            TableColumnGeometry {
                width: 4.0,
                name: "A".into(),
            },
            TableColumnGeometry {
                width: 4.0,
                name: "B".into(),
            },
        ],
        rows: vec![TableRowGeometry {
            height: 3.0,
            cells: vec![styled, cell("plain")],
        }],
        style_name: "Standard".into(),
        name: "Report".into(),
        description: "linked data".into(),
    });
    table
        .with_cell_margins(
            0,
            0,
            cad_domain::TableCellMarginsEdit {
                left: TableFieldEdit::Set(0.2),
                top: TableFieldEdit::Set(0.3),
                right: TableFieldEdit::Set(0.4),
                bottom: TableFieldEdit::Set(0.5),
            },
        )
        .expect("in grid")
}

fn written_table_content(bytes: &[u8]) -> acadrust::entities::Table {
    let document = acadrust::DxfReader::from_reader(std::io::Cursor::new(bytes.to_vec()))
        .expect("reader")
        .read()
        .expect("written DXF parses");
    document
        .objects
        .iter()
        .find_map(|(_, object)| match object {
            ObjectType::TableContent(content) => Some(content.clone()),
            _ => None,
        })
        .expect("one TABLECONTENT object")
}

#[test]
fn the_grid_format_margins_and_border_type_are_written_as_their_dxf_groups() {
    let (bytes, report) = export_one(SemanticGeometry::Table(Box::new(grid_format_table())));
    let content = written_table_content(&bytes);
    // The linked-data pair rides the object (the entity record has no group).
    assert_eq!(content.name, "Report");
    assert_eq!(content.description, "linked data");

    // Cell (0, 0) carries a CELLMARGIN block with the four flags set and the
    // values in top/left/bottom/right order, plus a GRIDFORMAT block whose
    // spacing is the doubled top edge.
    let styled = content.rows[0].cells[0]
        .style
        .as_ref()
        .expect("the styled cell keeps its style record");
    assert_ne!(styled.margin_override_flags, 0, "the CELLMARGIN block");
    assert!((styled.margin_left - 0.2).abs() < 1e-9);
    assert!((styled.margin_top - 0.3).abs() < 1e-9);
    assert!((styled.margin_right - 0.4).abs() < 1e-9);
    assert!((styled.margin_bottom - 0.5).abs() < 1e-9);
    assert!(styled.applied_border_edges.contains(CellEdgeFlags::TOP));
    assert!((styled.top_border.double_spacing - 0.5).abs() < 1e-9);
    assert_eq!(
        styled.top_border.border_type,
        acadrust::entities::BorderType::Double,
        "a spacing above zero writes the Double border type"
    );

    // The neutral cell writes no CELLMARGIN block at all.
    let plain = content.rows[0].cells[1]
        .style
        .as_ref()
        .expect("every cell gets a style record");
    assert_eq!(plain.margin_override_flags, 0);
    assert!(plain.top_border.double_spacing <= 0.0);

    assert!(
        report.entries.iter().any(|entry| entry
            .message
            .contains("ride the TABLECONTENT object written beside the ACAD_TABLE entity")),
        "{report:?}"
    );
}

#[test]
fn the_grid_format_margins_and_border_type_round_trip() {
    let source = grid_format_table();
    let before = source.cell_margins(0, 0);
    let (bytes, _report) = export_one(SemanticGeometry::Table(Box::new(source)));
    let reimported = import_bytes(&bytes);
    let entity = reimported
        .model_space()
        .into_iter()
        .find(|entity| entity.object.type_key == "AcDbTable")
        .expect("AcDbTable re-imported");
    let SemanticGeometry::Table(table) = &entity.geometry else {
        panic!("expected a Table variant");
    };
    assert_eq!(table.cell_margins(0, 0), before);
    assert_eq!(table.cell_margins(0, 1), Some(TableCellMargins::INHERIT));
    assert!((table.cell(0, 0).expect("cell").borders.top.double_spacing - 0.5).abs() < 1e-9);
    // The name/description come back from the object.
    assert_eq!(table.name(), "Report");
    assert_eq!(table.description(), "linked data");
    // The display follows the re-imported margins: the text anchor is inset.
    let mut found = false;
    for child in table.display() {
        if let SemanticGeometry::Text { text, position, .. } = child {
            if text == "styled" {
                assert!(position.x > 0.0 && position.y > 0.0, "{position:?}");
                found = true;
            }
        }
    }
    assert!(found, "the styled cell's text must be drawn");
}

#[test]
fn a_table_without_the_new_fields_writes_no_table_content_object() {
    // No margin, no doubled border, no name/description: the file must keep
    // exactly the pre-channel shape, so nothing is written beside the entity.
    let source = TableGeometry::from_import(TableParts {
        frame: TableFrame::default(),
        columns: vec![TableColumnGeometry {
            width: 4.0,
            name: "A".into(),
        }],
        rows: vec![TableRowGeometry {
            height: 3.0,
            cells: vec![cell("only")],
        }],
        style_name: "Standard".into(),
        name: String::new(),
        description: String::new(),
    });
    let (bytes, _report) = export_one(SemanticGeometry::Table(Box::new(source)));
    let document = acadrust::DxfReader::from_reader(std::io::Cursor::new(bytes.clone()))
        .expect("reader")
        .read()
        .expect("written DXF parses");
    assert!(
        !document
            .objects
            .iter()
            .any(|(_, object)| matches!(object, ObjectType::TableContent(_))),
        "an unset table writes no TABLECONTENT object"
    );
    let text = String::from_utf8_lossy(&bytes).to_string();
    assert!(
        !text.contains("CELLMARGIN_BEGIN") && !text.contains("GRIDFORMAT_BEGIN"),
        "an unset table writes no CELLMARGIN/GRIDFORMAT block"
    );
}
