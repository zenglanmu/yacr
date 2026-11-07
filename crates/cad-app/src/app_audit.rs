//! `AUDIT`: report invalid stored entities — **detect only, never repair**.
//!
//! This is the honest v1 of AutoCAD 2026's `AUDIT`. It walks every stored entity
//! and runs the database's existing [`cad_db::validate_entity`] check on it,
//! reporting one diagnostic per invalid entity. The import builder is
//! deliberately permissive (it records what a file holds), so an invalid entity
//! can exist in a real drawing; this command only **reports** it.
//!
//! AutoCAD's `AUDIT` also "corrects some errors". That half is **NOT
//! IMPLEMENTED**: nothing is repaired, rewritten, deleted or re-derived here, and
//! every clean/summary diagnostic says so. The command is read-only — no database
//! write, no transaction, no revision bump, no `cad-history` entry and no
//! `ChangeSet` — so it is allowed in Viewer mode. An invalid entity is left
//! exactly as it was stored.

use cad_db::validate_entity;
use cad_domain::{CadError, CadResult, Diagnostic};

use crate::{Application, Command, CommandOutcome, CommandPayload, SessionState};

impl Application {
    pub(crate) fn audit_drawing(
        &self,
        _session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        // AUDIT is payload-free: a caller that supplies one is refused rather
        // than silently ignored (the `LAYERP`/SELECTALL precedent).
        if !matches!(&command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput("AUDIT needs no payload".into()));
        }
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        if document.id != command.document {
            return Err(CadError::StaleResult);
        }
        let database = &document.drawing;
        let mut checked = 0usize;
        let mut invalid = 0usize;
        let mut diagnostics = Vec::new();
        for entity in database.entities() {
            checked += 1;
            if let Err(error) = validate_entity(database, entity.id, entity) {
                invalid += 1;
                // The report is ASCII-only on purpose: `CadError`'s Debug form can
                // carry arbitrary stored strings (layer names etc.), so every
                // non-ASCII character is replaced instead of passed through.
                diagnostics.push(Diagnostic {
                    object: Some(entity.object.id),
                    code: "audit.invalid_entity".into(),
                    message: format!(
                        "entity {}: {}",
                        entity.id.0,
                        ascii_safe(&format!("{error:?}"))
                    ),
                });
            }
        }
        if invalid == 0 {
            diagnostics.push(Diagnostic {
                object: None,
                code: "audit.clean".into(),
                message: format!(
                    "AUDIT: {checked} entities checked; no invalid entity found; no repair performed"
                ),
            });
        } else {
            // The summary leads; the per-entity findings follow in draw order.
            diagnostics.insert(
                0,
                Diagnostic {
                    object: None,
                    code: "audit.summary".into(),
                    message: format!(
                        "AUDIT: {checked} entities checked; {invalid} invalid; no repair performed"
                    ),
                },
            );
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// Keep a diagnostic message ASCII-only.
///
/// `CadError`'s `Display`/`Debug` can include values copied out of the drawing
/// (a layer or block name), and this report must never depend on the renderer's
/// handling of a control character or a non-ASCII byte. Printable ASCII and the
/// space are kept; everything else becomes `?`, so the message is always a plain
/// ASCII sentence while the reason itself stays visible.
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
