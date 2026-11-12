//! The Wave 27 read-only AutoCAD 2026 report batch: `RECOVER`, `RECOVERALL`,
//! `PURGEAECDATA` and `SHOWURLS`.
//!
//! Every handler here is a **report**, not a repair: it reads the open document
//! through the same authority the editor uses and writes nothing — no
//! transaction, no revision bump, no `ChangeSet`, no undo/history entry. None of
//! them returns an error for a merely unsupported capability; the boundary is a
//! named diagnostic so a caller can print it and carry on.
//!
//! # Honest boundaries
//!
//! * **RECOVER** scans every stored entity with [`cad_db::validate_entity`]
//!   (exactly the detect-only half of `AUDIT`) and reports the count. AutoCAD's
//!   `RECOVER` selects a **damaged DWG file on disk** and rebuilds the drawing;
//!   that is host file I/O plus an importer-level salvage pass, and this command
//!   layer has neither a file handle nor a salvage implementation. That half is
//!   named by `recover.file_level_not_modeled` instead of being faked.
//! * **RECOVERALL** is AutoCAD's "recover every open drawing and its xrefs"
//!   batch. This build has **one** open document and no xref resolution model in
//!   this path, so the single-document scan runs once and
//!   `recoverall.single_document` states the boundary.
//! * **PURGEAECDATA** removes AEC (Architecture/vertical-product) proxy data.
//!   This build imports and models no AEC objects at all, so there is no AEC
//!   data to purge and `purgeaecdata.no_aec_model` says so; nothing is written.
//! * **SHOWURLS** lists the hyperlinks attached to objects. `cad-db` retains no
//!   hyperlink/URL field, so `showurls.not_retained` reports the boundary rather
//!   than inventing URL rows or reporting a fabricated empty list.
//!
//! The per-entity messages are ASCII-only (same rule as `AUDIT`): a diagnostic
//! must never depend on the renderer's handling of a stored non-ASCII byte.

use super::*;

use crate::app_utility::{document_drawing, require_none_payload};
use cad_db::{validate_entity, DrawingDatabase};

impl Application {
    /// Dispatch the Wave 27 read-only report batch.
    pub(crate) fn recovery_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        match command.id {
            CommandId::RecoverDrawing => self.recover_drawing(command),
            CommandId::RecoverAllDrawings => self.recover_all_drawings(command),
            CommandId::PurgeAecData => self.purge_aec_data(command),
            CommandId::ShowUrls => self.show_urls(command),
            other => Err(CadError::Invariant(format!(
                "not a Wave 27 recovery report: {other:?}"
            ))),
        }
    }

    /// `RECOVER`: report the stored-entity scan of the open document.
    ///
    /// The scan is the detect-only half of `AUDIT` — [`cad_db::validate_entity`]
    /// over every stored entity — and nothing is repaired, rewritten or deleted.
    /// The disk-level DWG salvage that AutoCAD's `RECOVER` performs is
    /// host-owned and is named, never imitated.
    fn recover_drawing(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "RECOVER")?;
        let drawing = document_drawing(self, command)?;
        let scan = recovery_scan(&drawing, "RECOVER");
        let mut diagnostics = Vec::with_capacity(scan.invalid_messages.len() + 2);
        diagnostics.push(scan.summary);
        diagnostics.extend(scan.invalid_messages);
        diagnostics.push(Diagnostic {
            object: None,
            code: "recover.file_level_not_modeled".into(),
            message: "RECOVER: opening and repairing a damaged DWG file on disk is host-owned \
                      file I/O plus an importer-level salvage pass; this command reads only the \
                      already-open document and re-implements neither, so no file was opened or \
                      repaired"
                .into(),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `RECOVERALL`: the same single-document scan, with the batch boundary named.
    fn recover_all_drawings(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "RECOVERALL")?;
        let drawing = document_drawing(self, command)?;
        let scan = recovery_scan(&drawing, "RECOVERALL");
        let mut diagnostics = Vec::with_capacity(scan.invalid_messages.len() + 3);
        diagnostics.push(scan.summary);
        diagnostics.extend(scan.invalid_messages);
        diagnostics.push(Diagnostic {
            object: None,
            code: "recover.file_level_not_modeled".into(),
            message: "RECOVERALL: opening and repairing damaged DWG files on disk is host-owned \
                      file I/O plus an importer-level salvage pass; this command reads only the \
                      already-open document and re-implements neither"
                .into(),
        });
        diagnostics.push(Diagnostic {
            object: None,
            code: "recoverall.single_document".into(),
            message: "RECOVERALL: this build has one open document and no xref-batch recovery, so \
                      exactly one document was scanned; no other drawing and no external \
                      reference was opened or recovered"
                .into(),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `PURGEAECDATA`: report that no AEC data is modelled, so nothing is purged.
    fn purge_aec_data(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "PURGEAECDATA")?;
        // Touch the document so the command stays document-scoped; no AEC table is
        // read because none is modelled.
        let _ = document_drawing(self, command)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "purgeaecdata.no_aec_model".into(),
                message: "PURGEAECDATA: this build imports and models no AEC (Architecture/\
                          vertical-product) objects, so there is no AEC data to purge; nothing \
                          was removed or written"
                    .into(),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `SHOWURLS`: report that hyperlink/URL data is not retained.
    fn show_urls(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "SHOWURLS")?;
        // Document-scoped like the other reports, but no URL row exists to read.
        let _ = document_drawing(self, command)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "showurls.not_retained".into(),
                message: "SHOWURLS: hyperlink/URL data is not retained by this database, so no \
                          URL was found and no URL row was invented; the report is empty because \
                          the data does not exist in this build, not because the drawing has no \
                          hyperlinks"
                    .into(),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The shared entity scan: every stored entity through
/// [`cad_db::validate_entity`], reported as a leading summary plus one
/// `recover.invalid_entity` per rejection.
struct RecoveryScan {
    summary: Diagnostic,
    invalid_messages: Vec<Diagnostic>,
}

fn recovery_scan(database: &DrawingDatabase, label: &str) -> RecoveryScan {
    let mut checked = 0usize;
    let mut invalid = 0usize;
    let mut invalid_messages = Vec::new();
    for entity in database.entities() {
        checked += 1;
        if let Err(error) = validate_entity(database, entity.id, entity) {
            invalid += 1;
            // ASCII-only on purpose: `CadError`'s Debug form can carry arbitrary
            // stored strings (layer/block names), so every non-ASCII character is
            // replaced instead of passed through.
            invalid_messages.push(Diagnostic {
                object: Some(entity.object.id),
                code: "recover.invalid_entity".into(),
                message: format!(
                    "entity {}: {}",
                    entity.id.0,
                    ascii_safe(&format!("{error:?}"))
                ),
            });
        }
    }
    let summary = Diagnostic {
        object: None,
        code: "recover.summary".into(),
        message: if invalid == 0 {
            format!(
                "{label}: {checked} entities checked; no invalid entity found; no repair performed"
            )
        } else {
            format!("{label}: {checked} entities checked; {invalid} invalid; no repair performed")
        },
    };
    RecoveryScan {
        summary,
        invalid_messages,
    }
}

/// Keep a diagnostic message ASCII-only (the `AUDIT` rule, reused here).
///
/// `CadError`'s `Debug` form can include values copied out of the drawing (a
/// layer or block name), and a report must not depend on the renderer's handling
/// of a control character or a non-ASCII byte. Printable ASCII and the space are
/// kept; everything else becomes `?`, so the reason itself stays visible.
fn ascii_safe(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .collect()
}
