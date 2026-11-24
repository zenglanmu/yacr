//! Wave 6 **file / plot / export** handlers for the extended batch.
//!
//! `DWGLOG`, `LOGFILEON` and `LOGFILEOFF` read or write one session token
//! ([`SessionState::drafting_extras`]). The token is session state only: this
//! build writes **no** log file, so the outcome says that plainly instead of
//! implying that logging started.
//!
//! `DWGPROPS` is a document summary report **and** the `document.json`
//! metadata write: the bare form reads the document id/revision/entity counts
//! plus every stored title/author/subject/keywords/comments/custom property; a
//! `CommandPayload::DwgProps` write changes one field in one header revision.
//! `DWFFORMAT` and `EXPORTPDF` are **read-only capability reports**, not errors:
//! they state that DWF is not an export format in this build and that PDF export
//! is host-owned through `PLOT`. No host file I/O is re-implemented in `cad-app`.
//!
//! The log tokens and the two capability reports open no transaction, bump no
//! revision, record no `cad-history` entry and publish no `ChangeSet`, so their
//! forms are Viewer-legal. A `DWGPROPS` write records a header-only change and
//! refuses in Viewer mode.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, require_none_payload};

impl Application {
    /// Dispatch one Wave 6 file/plot/export token or report.
    pub(crate) fn extended_file6(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::DwgLog => self.dwglog(session, command),
            ExtendedCommandId::LogFileOn => self.logfile(session, command, true),
            ExtendedCommandId::LogFileOff => self.logfile(session, command, false),
            ExtendedCommandId::DwgProps => self.dwgprops(session, command),
            ExtendedCommandId::DwfFormat => self.dwfformat(command),
            ExtendedCommandId::ExportPdf => self.exportpdf(command),
            other => Err(CadError::Invariant(format!(
                "not a Wave 6 file/plot command: {other:?}"
            ))),
        }
    }

    /// `DWGLOG`: flip the session drawing-log token and report it.
    ///
    /// The token is session state only. This build creates no per-drawing log
    /// file, so nothing is written and no drawing row changes.
    fn dwglog(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "DWGLOG")?;
        let enabled = session.drafting_extras.toggle_dwglog();
        Ok(report_outcome(vec![diagnostic(
            "dwglog.toggled",
            format!(
                "DWGLOG: session drawing-log token is now {} (session state only); this build \
                 writes no log file and the drawing was not written",
                state(enabled)
            ),
        )]))
    }

    /// `LOGFILEON` / `LOGFILEOFF`: set the same session token explicitly.
    fn logfile(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        enabled: bool,
    ) -> CadResult<CommandOutcome> {
        let name = if enabled { "LOGFILEON" } else { "LOGFILEOFF" };
        require_none_payload(command, name)?;
        session.drafting_extras.set_dwglog(enabled);
        let (code, verb) = if enabled {
            ("logfileon.toggled", "ON")
        } else {
            ("logfileoff.toggled", "OFF")
        };
        Ok(report_outcome(vec![diagnostic(
            code,
            format!(
                "{name}: session drawing-log token is now {verb} (session state only); this build \
                 writes no log file"
            ),
        )]))
    }

    /// `DWGPROPS`: report the document's summary metadata, or write one field.
    ///
    /// The bare form reports the document id/revision/entity counts plus every
    /// stored summary field and custom property. The typed form writes exactly
    /// one field through [`cad_db::DrawingDatabase::set_document_metadata`]:
    /// one revision, one header-only `ChangeSet` and **no** undo step (`cad-history`
    /// has no document-metadata patch kind). Nothing is invented: absent fields
    /// are named, and a clear is only what the caller asked for.
    fn dwgprops(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let drawing = document_drawing(self, command)?;
        let action = match &command.payload {
            CommandPayload::None => {
                return Ok(report_outcome(dwgprops_report(command, &drawing)));
            }
            CommandPayload::DwgProps(action) => action.clone(),
            _ => {
                return Err(CadError::InvalidInput(
                    "dwgprops.invalid_payload: DWGPROPS takes no payload (report) or one \
                     `<field> <text>` / `CUSTOM <name> <text>` write"
                        .into(),
                ));
            }
        };
        // A metadata write is document content, so a Viewer session may not do
        // it. The bare report above stays Viewer-legal.
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let mut metadata = drawing.document_metadata().clone();
        let (code, message) = match action {
            DwgPropsCommand::SetField { field, value } => {
                let stored = metadata_value(field, &value)?;
                let row = match stored.as_deref() {
                    Some(value) => ("dwgprops.set", format!("{} is now {value:?}", field.key())),
                    None => ("dwgprops.cleared", format!("{} was cleared", field.key())),
                };
                metadata.set_field(field, stored);
                row
            }
            DwgPropsCommand::SetCustom { name, value } => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Err(CadError::InvalidInput(
                        "dwgprops.custom_name: a custom property needs a non-empty name".into(),
                    ));
                }
                if name.chars().count() > MAX_CUSTOM_NAME_CHARS {
                    return Err(CadError::InvalidInput(format!(
                        "dwgprops.custom_name: the custom property name is {} characters (limit \
                         {MAX_CUSTOM_NAME_CHARS}); nothing was written",
                        name.chars().count()
                    )));
                }
                if value.is_empty() {
                    metadata.custom.remove(&name);
                    (
                        "dwgprops.cleared",
                        format!("custom property {name:?} was removed"),
                    )
                } else {
                    validate_metadata_text("dwgprops.custom_value", &value)?;
                    if !metadata.custom.contains_key(&name)
                        && metadata.custom.len() >= MAX_CUSTOM_PROPERTIES
                    {
                        return Err(CadError::InvalidInput(format!(
                            "dwgprops.custom_limit: this document already stores \
                             {MAX_CUSTOM_PROPERTIES} custom properties; nothing was written"
                        )));
                    }
                    metadata.custom.insert(name.clone(), value);
                    ("dwgprops.set", format!("custom property {name:?} was set"))
                }
            }
        };
        if &metadata == drawing.document_metadata() {
            return Ok(report_outcome(vec![diagnostic(
                "dwgprops.unchanged",
                format!(
                    "DWGPROPS: {message}; the stored value was already identical, so no \
                         revision was raised"
                ),
            )]));
        }
        let document_id = command.document;
        let transaction = crate::app_drawing::next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let changeset = candidate.set_document_metadata(metadata, transaction, "dwgprops");
        let after = changeset.after.0;
        self.workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::Invariant("document disappeared while publishing".into()))?
            .drawing = Arc::new(candidate);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(changeset)),
            diagnostics: vec![
                diagnostic(
                    code,
                    format!(
                        "DWGPROPS: {message}; stored in document.json metadata (revision {after})"
                    ),
                ),
                diagnostic(
                    "dwgprops.no_history",
                    "DWGPROPS: this write published one revision and one ChangeSet and recorded \
                     NO undo step; cad-history has no document-metadata patch kind"
                        .into(),
                ),
            ],
            measurement: None,
            point_report: None,
        })
    }

    /// `DWFFORMAT`: read-only report — DWF/DWFx is not an export format here.
    fn dwfformat(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "DWFFORMAT")?;
        let _drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![diagnostic(
            "dwfformat.not_supported",
            "DWFFORMAT: this build does not export DWF or DWFx and there is no default-format \
             setting; use SVG/PDF via PLOT; nothing was changed"
                .into(),
        )]))
    }

    /// `EXPORTPDF`: read-only report — PDF export is host-owned through `PLOT`.
    fn exportpdf(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "EXPORTPDF")?;
        let _drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![diagnostic(
            "exportpdf.use_plot",
            "EXPORTPDF: PDF export is host-owned through PLOT (the platform save picker); this \
             command writes nothing itself; nothing was changed"
                .into(),
        )]))
    }
}

/// The longest accepted summary/custom text, in Unicode scalar values.
///
/// The bound is an engineering limit on a `document.json` scalar, not an
/// AutoCAD value: refusing a longer write is explicit and changes nothing. It
/// is deliberately generous (AutoCAD's own summary fields are short).
const MAX_METADATA_CHARS: usize = 4096;
/// The longest accepted custom-property **name**, in Unicode scalar values.
const MAX_CUSTOM_NAME_CHARS: usize = 255;
/// How many custom properties one document may store.
const MAX_CUSTOM_PROPERTIES: usize = 256;

/// Read-only `DWGPROPS` report: identity plus every stored summary field.
fn dwgprops_report(command: &Command, drawing: &cad_db::DrawingDatabase) -> Vec<Diagnostic> {
    let metadata = drawing.document_metadata();
    let mut rows = Vec::with_capacity(12);
    rows.push(diagnostic(
        "dwgprops.report",
        format!(
            "DWGPROPS: document {} revision {} entities {}",
            command.document.0,
            drawing.revision().0,
            drawing.entity_count()
        ),
    ));
    let mut stored = 0usize;
    for field in [
        cad_db::DocumentMetadataField::Title,
        cad_db::DocumentMetadataField::Author,
        cad_db::DocumentMetadataField::Subject,
        cad_db::DocumentMetadataField::Keywords,
        cad_db::DocumentMetadataField::Comments,
    ] {
        if let Some(value) = metadata.field(field) {
            rows.push(diagnostic(
                "dwgprops.field",
                format!("DWGPROPS {} = {value:?}", field.key()),
            ));
            stored += 1;
        }
    }
    for (name, value) in &metadata.custom {
        rows.push(diagnostic(
            "dwgprops.custom",
            format!("DWGPROPS custom {name:?} = {value:?}"),
        ));
        stored += 1;
    }
    if stored == 0 {
        rows.push(diagnostic(
            "dwgprops.empty",
            "DWGPROPS: this document stores no summary metadata; set it with `DWGPROPS TITLE \
             <text>`, AUTHOR, SUBJECT, KEYWORDS, COMMENTS or `DWGPROPS CUSTOM <name> <text>`"
                .into(),
        ));
    } else {
        rows.push(diagnostic(
            "dwgprops.summary",
            format!(
                "DWGPROPS: {stored} metadata value(s) stored in document.json; an empty write \
                 clears a named field or removes a custom property"
            ),
        ));
    }
    rows
}

/// Validate one named field's text and map an empty write to "clear".
fn metadata_value(field: cad_db::DocumentMetadataField, value: &str) -> CadResult<Option<String>> {
    if value.is_empty() {
        return Ok(None);
    }
    validate_metadata_text(&format!("dwgprops.{}", field.key()), value)?;
    Ok(Some(value.to_string()))
}

/// Refuse a metadata value the document header cannot honestly store.
fn validate_metadata_text(code: &str, value: &str) -> CadResult<()> {
    if value.contains('\0') {
        return Err(CadError::InvalidInput(format!(
            "{code}: the value contains a NUL character; nothing was written"
        )));
    }
    let chars = value.chars().count();
    if chars > MAX_METADATA_CHARS {
        return Err(CadError::InvalidInput(format!(
            "{code}: the value is {chars} characters (limit {MAX_METADATA_CHARS}); nothing was \
             written"
        )));
    }
    Ok(())
}

fn state(enabled: bool) -> &'static str {
    if enabled {
        "ON"
    } else {
        "OFF"
    }
}
