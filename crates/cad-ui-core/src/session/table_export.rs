//! `TABLEEXPORT <path.csv>`: write one table's grid to a CSV file.
//!
//! AutoCAD opens a dialog; this build has no dialog, so the destination is a
//! typed argument. The exporter is the same shape as `HATCHEDIT PATTERNFILE` in
//! the opposite direction:
//!
//! * the **session** owns the grammar (`cad_ui_core::session::commands::table`)
//!   and stays filesystem-free (it also runs in the browser);
//! * the **host strategy** owns the file
//!   ([`cad_host::HostResources::write_named_file`], the write counterpart of the
//!   `read_named_file` `PATTERNFILE` uses), so every native carrier writes
//!   through `cad_platform::write_named_file` (temp file + rename) and a runtime
//!   with no filesystem refuses instead of claiming a write;
//! * the **bytes** are a pure derivation of the stored grid
//!   ([`cad_domain::TableGeometry::to_csv`]): one record per row, one field per
//!   column, RFC 4180 quoting, the anchor text in a merged range and empty fields
//!   for its covered cells.
//!
//! Selection rule: exactly one selected, directly addressable `Table` in the
//! command's document — the same rule `TABLEDIT`/`MATCHCELL`/`TABLESTYLE` use.
//!
//! Stable refusals (every one happens **before** the write, so a refusal creates
//! no file and does not touch one that exists):
//!
//! * `tableexport.path` — no argument, or a path without the `.csv` extension;
//! * `tableexport.no_selection` / `.multiple` / `.not_table` — the shared
//!   pre-selection rule;
//! * `tableexport.empty` — the table has no grid to write;
//! * `unsupported-runtime: table-export` — this host has no filesystem for a
//!   user-named file (the stable marker the browser carrier answers);
//! * `tableexport.write_failed: <reason>` — the platform refused the write
//!   (missing directory, permission, size bound); the reason is the platform's
//!   own text, never a fabricated success.
//!
//! A successful export writes one file per invocation (`TABLEEXPORT a.csv` then
//! `TABLEEXPORT b.csv` writes two independent files), reports the path and the
//! written byte count on the status line, and **never** modifies the drawing: it
//! is a read-only command, so it opens no transaction, bumps no revision and
//! records no history. It is therefore Viewer-legal, which is exactly how the
//! command layer authorizes it here.

use std::path::Path;

use cad_app::{CommandId, ExtendedCommandId};
use cad_domain::SemanticGeometry;
use cad_platform::NamedFileError;

use super::diagnostics::cad_error_text;
use super::CadSession;

/// Parse `TABLEEXPORT <path.csv>`: the whole trimmed remainder is the path.
///
/// The path is one argument — it may contain spaces (a Windows/desktop path
/// often does) because there is no second argument to confuse it with. The
/// extension is required: a destination without `.csv` would make the writer
/// guess a format, so it is refused by name instead.
pub fn parse_tableexport_argument(argument: &str) -> Option<String> {
    let path = argument.trim();
    if path.is_empty() {
        return None;
    }
    if path.contains('\0') {
        return None;
    }
    let extension = Path::new(path).extension()?.to_str()?;
    extension
        .eq_ignore_ascii_case("csv")
        .then(|| path.to_string())
}

/// The stable refusal for an argument outside the `TABLEEXPORT` grammar.
pub fn tableexport_path_reason(text: &str) -> String {
    format!(
        "tableexport.path: `{}` has no destination path; TABLEEXPORT needs one path ending in \
         `.csv` (exactly one selected table is the object input)",
        text.trim()
    )
}

impl CadSession {
    /// Apply one typed `TABLEEXPORT <path.csv>` line.
    pub(super) fn table_export_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(path) = parse_tableexport_argument(argument) else {
            return self.refuse_argument(tableexport_path_reason(text));
        };
        // Authorize **first**, through the same gate the command layer uses, so a
        // refused mode never performs host I/O.
        if let Err(error) = self
            .core
            .controller
            .borrow()
            .session
            .authorize(CommandId::Extended(ExtendedCommandId::TableExport))
        {
            return Err(cad_error_text(&error));
        }
        let csv = match self.table_export_csv() {
            Ok(csv) => csv,
            Err(reason) => return self.refuse_argument(reason),
        };
        if let Err(error) = self
            .core
            .resources
            .write_named_file(Path::new(&path), csv.as_bytes())
        {
            return self.refuse_argument(write_reason(&path, &error));
        }
        self.core
            .record_command(super::commands::TABLEEXPORT_SELECTOR);
        *self.error.borrow_mut() = None;
        let line = format!(
            "TABLEEXPORT: wrote {} bytes of CSV to {path} (one record per table row, one field per \
             column); the drawing was not modified",
            csv.len()
        );
        // A read-only export has no app command to carry the diagnostic, so the
        // status line is the report. `ExternalText` is the platform-free variant
        // for text this build composed itself (no catalog key, no localization).
        let _ = self.core.set_external_status(line);
        Ok(())
    }

    /// The CSV text of the one selected table, or a stable refusal reason.
    fn table_export_csv(&self) -> Result<String, String> {
        let controller = self.core.controller.borrow();
        let references = controller.session.selection.refs();
        let reference = match references {
            [] => {
                return Err(
                    "tableexport.no_selection: TABLEEXPORT needs exactly one selected table; \
                     nothing is selected"
                        .to_string(),
                )
            }
            [one] => one,
            many => {
                return Err(format!(
                    "tableexport.multiple: TABLEEXPORT exports exactly one table; {} entities are \
                     selected",
                    many.len()
                ))
            }
        };
        let document = controller
            .drawing()
            .ok_or_else(|| "tableexport.not_table: no document is open".to_string())?;
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(format!(
                "tableexport.not_table: entity {} is not a directly addressable table in this \
                 document",
                reference.entity.0
            ));
        }
        let entity = document.entity(reference.entity).ok_or_else(|| {
            format!(
                "tableexport.not_table: entity {} does not exist in this drawing",
                reference.entity.0
            )
        })?;
        let SemanticGeometry::Table(table) = &entity.geometry else {
            return Err(format!(
                "tableexport.not_table: entity {} is not a table",
                entity.id.0
            ));
        };
        let csv = table.to_csv();
        if csv.is_empty() {
            return Err(format!(
                "tableexport.empty: table {} has no row x column grid to export",
                entity.id.0
            ));
        }
        Ok(csv)
    }
}

/// The stable refusal for a failed write, or the host-less runtime marker.
///
/// [`NamedFileError::Unavailable`] is the one case that is **not** a property of
/// the path: this runtime has no filesystem at all (a browser carrier), so it
/// answers the stable `unsupported-runtime:` marker the rest of the build uses
/// for a missing capability, never "missing path".
fn write_reason(path: &str, error: &NamedFileError) -> String {
    match error {
        NamedFileError::Unavailable => {
            "unsupported-runtime: table-export (this host has no filesystem for a user-named file; \
             TABLEEXPORT writes nothing)"
                .to_string()
        }
        NamedFileError::Missing(reason) | NamedFileError::Unreadable(reason) => {
            format!("tableexport.write_failed: {path}: {reason}")
        }
    }
}
