//! The post-creation table grammars and `TABLEEXPORT`'s host write half.
//!
//! Two groups:
//!
//! * the **grammars** (`TABLEDIT MERGE|UNMERGE`, `MATCHCELL`, `TABLESTYLE`) are
//!   pure parsers, so they are pinned without a host at all — a malformed line is
//!   refused before IPC and changes nothing;
//! * `TABLEEXPORT <path.csv>` is the **host write**: the session resolves the one
//!   selected table, serializes the grid and asks the resource strategy to write
//!   it, exactly as `HATCHEDIT PATTERNFILE` asks it to read. These tests pin a
//!   real write through the platform carrier, the browser's
//!   `unsupported-runtime: table-export` marker, the "a refusal writes nothing"
//!   rule, and the Viewer gate that runs before any write.

use super::super::super::*;
use super::super::test_support::*;

use cad_app::AppMode;
use cad_host::HostResources;
use cad_platform::NamedFileError;
use std::path::{Path, PathBuf};

use crate::session::commands::{
    parse_matchcell_argument, parse_tabledit_option_argument, parse_tablestyle_argument,
    ARGUMENT_SELECTORS, MATCHCELL_SELECTOR, TABLEDIT_SELECTOR, TABLEEXPORT_SELECTOR,
    TABLESTYLE_SELECTOR,
};
use cad_app::{TableCellAddress, TablePayload, TableStyleCommand, TableStyleEdge};

// ---------------------------------------------------------------------------
// Grammars.
// ---------------------------------------------------------------------------

#[test]
fn tabledit_merge_and_unmerge_parse_a_1_based_span() {
    assert_eq!(
        parse_tabledit_option_argument("MERGE 2 1 2 3"),
        Some(TablePayload::Merge {
            anchor: TableCellAddress { row: 2, col: 1 },
            height: 2,
            width: 3,
        })
    );
    assert_eq!(
        parse_tabledit_option_argument("unmerge 4 5"),
        Some(TablePayload::Unmerge {
            cell: TableCellAddress { row: 4, col: 5 },
        })
    );
    // `TABLEDIT STYLE <name>` assigns the table's style snapshot.
    assert_eq!(
        parse_tabledit_option_argument("STYLE Report"),
        Some(TablePayload::Style(Box::new(TableStyleCommand {
            name: Some("Report".into()),
            ..TableStyleCommand::default()
        })))
    );
    assert_eq!(parse_tabledit_option_argument("STYLE"), None);
    assert_eq!(parse_tabledit_option_argument("STYLE a b"), None);
    // The cell-text form is not one of those words, so it falls through.
    assert_eq!(parse_tabledit_option_argument("2 1 hello world"), None);
}

#[test]
fn a_malformed_merge_line_is_not_a_payload() {
    // A zero span, a fraction, a missing token and a negative value are refused
    // here (the caller then refuses the whole line by name).
    assert_eq!(parse_tabledit_option_argument("MERGE 1 1 0 2"), None);
    assert_eq!(parse_tabledit_option_argument("MERGE 1 1 2"), None);
    assert_eq!(parse_tabledit_option_argument("MERGE 1 1 2 2.5"), None);
    assert_eq!(parse_tabledit_option_argument("MERGE 1 1 2 -2"), None);
    assert_eq!(parse_tabledit_option_argument("UNMERGE 0 1"), None);
    assert_eq!(parse_tabledit_option_argument("UNMERGE 1"), None);
}

#[test]
fn matchcell_needs_a_source_and_at_least_one_destination_pair() {
    assert_eq!(
        parse_matchcell_argument("1 1 2 2"),
        Some(TablePayload::MatchCell {
            source: TableCellAddress { row: 1, col: 1 },
            targets: vec![TableCellAddress { row: 2, col: 2 }],
        })
    );
    assert_eq!(
        parse_matchcell_argument("1 1 2 2 3 4"),
        Some(TablePayload::MatchCell {
            source: TableCellAddress { row: 1, col: 1 },
            targets: vec![
                TableCellAddress { row: 2, col: 2 },
                TableCellAddress { row: 3, col: 4 },
            ],
        })
    );
    // A lone source, an odd token count and a zero address are refused.
    assert_eq!(parse_matchcell_argument("1 1"), None);
    assert_eq!(parse_matchcell_argument("1 1 2 2 3"), None);
    assert_eq!(parse_matchcell_argument("0 1 2 2"), None);
}

#[test]
fn tablestyle_parses_a_name_and_its_fields() {
    let command = parse_tablestyle_argument(
        "Project HEIGHT 3.5 MARGIN 0.2 0.3 TEXTSTYLE RomanS BORDER LEFT DOUBLE 0.25 BORDER TOP OFF \
         TITLE OFF HEADER ON",
    )
    .expect("a valid field list");
    assert_eq!(command.name.as_deref(), Some("Project"));
    assert_eq!(command.text_height, Some(3.5));
    assert_eq!(command.horizontal_margin, Some(0.2));
    assert_eq!(command.vertical_margin, Some(0.3));
    assert_eq!(command.text_style_name.as_deref(), Some("RomanS"));
    assert_eq!(
        command.borders,
        vec![
            (TableStyleEdge::Left, true, Some(0.25)),
            (TableStyleEdge::Top, false, None),
        ]
    );
    assert_eq!(command.title_suppressed, Some(true));
    assert_eq!(command.header_suppressed, Some(false));
}

#[test]
fn tablestyle_keeps_the_current_name_when_the_line_starts_with_a_field() {
    let command = parse_tablestyle_argument("HEIGHT 4").expect("a field list");
    assert_eq!(command.name, None);
    assert_eq!(command.text_height, Some(4.0));
}

#[test]
fn tablestyle_description_takes_the_rest_of_the_line() {
    let command =
        parse_tablestyle_argument("Report DESC bolts per assembly").expect("a field list");
    assert_eq!(command.name.as_deref(), Some("Report"));
    assert_eq!(command.description.as_deref(), Some("bolts per assembly"));
}

#[test]
fn tablestyle_refuses_unknown_words_and_bad_values() {
    assert_eq!(parse_tablestyle_argument("NAME x"), None);
    assert_eq!(parse_tablestyle_argument("HEIGHT 0"), None);
    assert_eq!(parse_tablestyle_argument("HEIGHT"), None);
    assert_eq!(parse_tablestyle_argument("MARGIN -1 2"), None);
    assert_eq!(parse_tablestyle_argument("BORDER SIDEWAYS ON"), None);
    assert_eq!(parse_tablestyle_argument("BORDER TOP MAYBE"), None);
    assert_eq!(parse_tablestyle_argument("TITLE 7"), None);
    assert_eq!(parse_tablestyle_argument("BORDER LEFT DOUBLE -1"), None);
}

#[test]
fn the_post_creation_table_selectors_are_argument_forms() {
    for selector in [
        TABLEDIT_SELECTOR,
        MATCHCELL_SELECTOR,
        TABLESTYLE_SELECTOR,
        TABLEEXPORT_SELECTOR,
    ] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be an admitted argument form"
        );
    }
}

// ---------------------------------------------------------------------------
// TABLEEXPORT: the host write.
// ---------------------------------------------------------------------------

/// A strategy that serves a **real** filesystem through the platform carrier.
struct NativeFiles;

impl HostResources for NativeFiles {
    fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
    fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
    fn write_named_file(&self, path: &Path, bytes: &[u8]) -> Result<(), NamedFileError> {
        cad_platform::write_named_file(path, bytes)
    }
}

/// A strategy with no filesystem at all (the browser carrier's behaviour).
struct BrowserFiles;

impl HostResources for BrowserFiles {
    fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
    fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
}

fn session_with(resources: Rc<dyn HostResources>) -> CadSession {
    let mut host_platform = platform(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    host_platform.resources = resources;
    session_with_platform(host_platform)
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "yacr-tableexport-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A 2x2 table with text in every cell, created and then selected.
fn session_with_selected_table(resources: Rc<dyn HostResources>) -> CadSession {
    let session = session_with(resources);
    session
        .command("TABLE 2 2 4 3 0 0")
        .expect("a table is created");
    select_only_table(&session);
    for row in 1..=2u32 {
        for column in 1..=2u32 {
            session
                .command(&format!("TABLEDIT {row} {column} r{row}c{column}"))
                .expect("cell text");
        }
    }
    session
}

/// Select the one table the session's drawing holds (the pre-selection every
/// table command reads).
fn select_only_table(session: &CadSession) {
    let mut controller = session.core.controller.borrow_mut();
    let document = controller.document_id;
    let id = controller
        .drawing()
        .expect("a drawing is open")
        .model_space()
        .iter()
        .find(|entity| matches!(&entity.geometry, SemanticGeometry::Table(_)))
        .expect("the created table")
        .id;
    controller
        .session
        .selection
        .replace([cad_domain::SelectionRef {
            document,
            entity: id,
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        }]);
}

fn drawing_revision(session: &CadSession) -> cad_domain::Revision {
    session
        .core
        .controller
        .borrow()
        .drawing()
        .expect("a drawing is open")
        .revision()
}

#[test]
fn a_table_export_writes_the_csv_through_the_host_channel() {
    let dir = temp_dir("ok");
    let path = dir.join("cells.csv");
    let session = session_with_selected_table(Rc::new(NativeFiles));
    let revision = drawing_revision(&session);
    session
        .command(&format!("TABLEEXPORT {}", path.display()))
        .expect("the export runs");
    let text = std::fs::read_to_string(&path).expect("the file exists");
    assert_eq!(text, "r1c1,r1c2\r\nr2c1,r2c2\r\n");
    // A read-only export writes no drawing state.
    assert_eq!(drawing_revision(&session), revision, "no revision bump");
}

#[test]
fn a_table_export_without_a_csv_extension_is_refused_and_writes_nothing() {
    let dir = temp_dir("ext");
    let path = dir.join("cells.txt");
    let session = session_with_selected_table(Rc::new(NativeFiles));
    let error = session
        .command(&format!("TABLEEXPORT {}", path.display()))
        .expect_err("not a .csv path");
    assert!(error.contains("tableexport.path"), "{error}");
    assert!(!path.exists(), "a refused export creates no file");
}

#[test]
fn a_table_export_to_a_missing_directory_is_refused_by_name() {
    let dir = temp_dir("missing");
    let path = dir.join("absent").join("cells.csv");
    let session = session_with_selected_table(Rc::new(NativeFiles));
    let error = session
        .command(&format!("TABLEEXPORT {}", path.display()))
        .expect_err("no directory");
    assert!(error.contains("tableexport.write_failed"), "{error}");
    assert!(!path.exists());
}

#[test]
fn a_browser_runtime_answers_the_stable_unsupported_marker() {
    let dir = temp_dir("browser");
    let path = dir.join("cells.csv");
    let session = session_with_selected_table(Rc::new(BrowserFiles));
    let error = session
        .command(&format!("TABLEEXPORT {}", path.display()))
        .expect_err("no filesystem");
    assert!(
        error.contains("unsupported-runtime: table-export"),
        "{error}"
    );
    assert!(!path.exists(), "the browser writes nothing");
}

#[test]
fn a_table_export_with_nothing_selected_is_refused_by_name() {
    let dir = temp_dir("noselection");
    let path = dir.join("cells.csv");
    let session = session_with(Rc::new(NativeFiles));
    let error = session
        .command(&format!("TABLEEXPORT {}", path.display()))
        .expect_err("nothing selected");
    assert!(error.contains("tableexport.no_selection"), "{error}");
    assert!(!path.exists());
}

#[test]
fn a_table_export_of_a_non_table_is_refused_by_name() {
    let dir = temp_dir("nontable");
    let path = dir.join("cells.csv");
    let session = session_with(Rc::new(NativeFiles));
    session.command("DONUT 0 0 1 2").expect("a donut");
    {
        let mut controller = session.core.controller.borrow_mut();
        let document = controller.document_id;
        let id = controller
            .drawing()
            .expect("a drawing is open")
            .model_space()
            .iter()
            .find(|entity| entity.object.type_key != "AcDbTable")
            .expect("the line")
            .id;
        controller
            .session
            .selection
            .replace([cad_domain::SelectionRef {
                document,
                entity: id,
                instance: cad_domain::InstancePath::default(),
                sub_element: None,
            }]);
    }
    let error = session
        .command(&format!("TABLEEXPORT {}", path.display()))
        .expect_err("a line is not a table");
    assert!(error.contains("tableexport.not_table"), "{error}");
    assert!(!path.exists());
}

#[test]
fn a_viewer_can_export_because_the_drawing_is_not_written() {
    let dir = temp_dir("viewer");
    let path = dir.join("cells.csv");
    let session = session_with_selected_table(Rc::new(NativeFiles));
    let revision = drawing_revision(&session);
    session
        .core
        .controller
        .borrow_mut()
        .session
        .switch_mode(AppMode::Viewer)
        .expect("switch to Viewer");
    session
        .command(&format!("TABLEEXPORT {}", path.display()))
        .expect("a read-only export is Viewer-legal, like SAVE/PLOT");
    assert_eq!(
        std::fs::read_to_string(&path).expect("the file exists"),
        "r1c1,r1c2\r\nr2c1,r2c2\r\n"
    );
    assert_eq!(drawing_revision(&session), revision, "no drawing write");
}

#[test]
fn a_viewer_cannot_write_table_style_or_match_a_cell() {
    let session = session_with_selected_table(Rc::new(NativeFiles));
    session
        .core
        .controller
        .borrow_mut()
        .session
        .switch_mode(AppMode::Viewer)
        .expect("switch to Viewer");
    for line in [
        "TABLESTYLE HEIGHT 4",
        "MATCHCELL 1 1 2 2",
        "TABLEDIT MERGE 1 1 1 2",
    ] {
        let error = session.command(line).expect_err("Viewer cannot write");
        assert!(
            error.contains("PermissionDenied") || error.to_lowercase().contains("denied"),
            "{line}: {error}"
        );
    }
}
