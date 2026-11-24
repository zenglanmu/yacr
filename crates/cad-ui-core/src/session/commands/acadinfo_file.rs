//! `ACADINFO <file.json>`: write the versioned information file.
//!
//! AutoCAD's `ACADINFO` creates a file describing the installation and current
//! setup. The bare command stays the read-only report; this module owns the
//! file form. The facts come from the **one** `cad-app` snapshot
//! ([`cad_app::acad_info::AcadInfoSnapshot`]), the bytes are written through the
//! host's named-file bridge (`HostResources::write_named_file`, exactly like
//! `SYSVDLG SAVE`/`TABLEEXPORT`), and a host without a sink refuses by name
//! instead of claiming a file.
//!
//! The file is versioned JSON (`{"format":"yacr.acadinfo","version":1,…}`), not
//! AutoCAD's prose text file. Licence, hardware and installed-module rows are
//! not collected anywhere in this build; the file names them in `notCollected`
//! instead of omitting them silently. No timestamp is written: the document
//! header tracks no creation/update clock (see `TIME`/`EDITTIME`).

use std::path::Path;

use cad_app::acad_info::{ACADINFO_FILE_FORMAT, ACADINFO_FILE_VERSION};
use cad_platform::NamedFileError;

use crate::session::CadSession;

/// `ACADINFO <file.json>`: the information-file form of the report.
pub const ACADINFO_SELECTOR: &str = "ACADINFO";

/// Parse `ACADINFO <file.json>`: the whole trimmed remainder is the path.
///
/// Like `SYSVDLG SAVE`, the destination must end in `.json`; this build has no
/// AutoCAD `acadinfo.txt` writer and must not write a file whose extension
/// suggests a format it does not produce. A path may contain spaces.
pub fn parse_acadinfo_argument(argument: &str) -> Option<String> {
    let path = argument.trim();
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    let extension = Path::new(path).extension()?.to_str()?;
    extension
        .eq_ignore_ascii_case("json")
        .then(|| path.to_string())
}

impl CadSession {
    /// Apply one typed `ACADINFO <file.json>` line.
    pub(crate) fn acadinfo_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(path) = parse_acadinfo_argument(argument) else {
            return self.refuse_argument(format!(
                "acadinfo.path: `{}` has no `.json` destination; ACADINFO takes one file path \
                 ending in .json (the bare command is the report)",
                text.trim()
            ));
        };
        let encoded = {
            let controller = self.core.controller.borrow();
            let snapshot = controller
                .application
                .acad_info_snapshot(controller.document_id)
                .ok_or_else(|| {
                    "acadinfo.no_document: ACADINFO needs an open document; the information file \
                     carries its revision and entity count"
                        .to_string()
                })?;
            serde_json::to_string_pretty(&snapshot.to_file_value())
                .map_err(|error| format!("acadinfo.encode_failed: {error}"))?
        };
        if let Err(error) = self
            .core
            .resources
            .write_named_file(Path::new(&path), encoded.as_bytes())
        {
            return self.refuse_argument(write_reason(&path, &error));
        }
        self.core.record_command(ACADINFO_SELECTOR);
        *self.error.borrow_mut() = None;
        let line = format!(
            "ACADINFO: wrote {} bytes of {ACADINFO_FILE_FORMAT} v{ACADINFO_FILE_VERSION} JSON to \
             {path}; licence, hardware and installed-module rows are not collected, and the \
             drawing was not modified",
            encoded.len()
        );
        // A read-only report has no app command to carry the diagnostic, so the
        // status line is the report (`ExternalText` is the platform-free variant
        // for text this build composed itself).
        let _ = self.core.set_external_status(line);
        Ok(())
    }
}

fn write_reason(path: &str, error: &NamedFileError) -> String {
    match error {
        NamedFileError::Unavailable => format!(
            "acadinfo.write_unavailable: unsupported-runtime: acadinfo-file: this host cannot \
             write the user-named file '{path}' ({})",
            error.reason()
        ),
        NamedFileError::Missing(reason) | NamedFileError::Unreadable(reason) => {
            format!("acadinfo.write_failed: {path}: {reason}")
        }
    }
}
