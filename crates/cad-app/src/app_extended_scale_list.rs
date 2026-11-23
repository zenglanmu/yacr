//! `SCALELISTEDIT` / `-SCALELISTEDIT`: the drawing's named annotation scales
//! (`ACAD_SCALELIST`).
//!
//! # The table is real, and the writes are real
//!
//! This build has a genuine scale table: `cad-connector-acadrust` reads every
//! non-temporary `AcDbScale` object into
//! [`DrawingDatabase::scales`](cad_db::DrawingDatabase::scales), and
//! `cad_db::drawing::scale_edit` is the sanctioned post-import writer. So both
//! selectors report the stored rows and both accept the same write grammar —
//! there is **one** write path, not a dialog path and a command-line path.
//!
//! * bare `SCALELISTEDIT` / `-SCALELISTEDIT` / `?` — read-only report of every
//!   stored row plus the page of the first paper layout (a drawing with no paper
//!   layout reports that instead of a fabricated page);
//! * `ADD <name> <paper_units> <drawing_units>` — one new row;
//! * `RENAME <old> <new>` — rename one row, cascading into the `CANNOSCALE`
//!   value and every annotative entity's per-scale placement override;
//! * `DELETE <name>` — remove one row, refused by name while a stored reference
//!   still names it.
//!
//! Every write is one `cad-db` mutation: one revision and one [`ChangeSet`]. A
//! refusal happens before the first mutation, so a refused write changes no row,
//! bumps no revision and publishes nothing.
//!
//! # What is *not* claimed
//!
//! * **No undo step.** `cad-history` has no scale-row patch kind, so these
//!   writes are not undoable; the diagnostic says so instead of implying a step
//!   that `UNDO` could not replay.
//! * **No dialog.** AutoCAD's `SCALELISTEDIT` is a modal list editor; this build
//!   has no such dialog, so the bare selector is the report and the edits are
//!   typed lines. `RESET` (restore the default list) is a named refusal: this
//!   build has no authoritative default-scale template to restore, and inventing
//!   one would silently rewrite the table.
//! * **No viewport/plot by-name reference.** A delete is refused by name where a
//!   *name* is stored (the `CANNOSCALE` value and annotative overrides). The
//!   paper viewports store a model↔paper transform and the plot settings store a
//!   raw numerator/denominator pair, so the command reports that boundary rather
//!   than pretending to check them.
//!
//! [`ChangeSet`]: cad_db::ChangeSet

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::document_drawing;

/// One typed `-SCALELISTEDIT` write.
///
/// The grammar lives in `cad-ui-core` (`session::scale_arguments`); this type is
/// the payload those parsers produce, so a direct IPC caller and a typed command
/// line reach exactly the same handler.
#[derive(Debug, Clone, PartialEq)]
pub enum ScaleListCommand {
    /// `ADD <name> <paper_units> <drawing_units>`.
    Add {
        name: String,
        paper_units: f64,
        drawing_units: f64,
    },
    /// `RENAME <old> <new>`.
    Rename { old: String, new: String },
    /// `DELETE <name>`.
    Delete { name: String },
}

impl Application {
    /// Dispatch one `SCALELISTEDIT` / `-SCALELISTEDIT` invocation.
    ///
    /// `report_command` is the name to quote in the report diagnostics: `-` and
    /// non-`-` spellings share this implementation, so the message matches what
    /// the user typed.
    pub(crate) fn scale_list(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        report_command: &str,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::None => self.scale_list_report(command, report_command),
            CommandPayload::ScaleList(op) => {
                self.scale_list_write(session, command, op, report_command)
            }
            _ => Err(CadError::InvalidInput(format!(
                "{report_command} takes no payload or one ADD|RENAME|DELETE operation"
            ))),
        }
    }

    /// The read-only report: every stored row, then the active scale and the
    /// first paper layout's page.
    fn scale_list_report(
        &self,
        command: &Command,
        report_command: &str,
    ) -> CadResult<CommandOutcome> {
        let drawing = document_drawing(self, command)?;
        let mut rows = Vec::new();
        let mut scales: Vec<&cad_db::Scale> = drawing.scales().collect();
        scales.sort_by_key(|scale| scale.id.0);
        for scale in &scales {
            rows.push(diagnostic(
                "scalelistedit.entry",
                format!(
                    "{report_command}: scale '{}' paper/drawing = {}/{} (factor {})",
                    scale.name,
                    scale.paper_units,
                    scale.drawing_units,
                    scale.factor()
                ),
            ));
        }
        if scales.is_empty() {
            rows.push(diagnostic(
                "scalelistedit.empty",
                format!(
                    "{report_command}: this drawing's ACAD_SCALELIST is empty; use \
                     `{report_command} ADD <name> <paper_units> <drawing_units>` to add a scale"
                ),
            ));
        }
        match drawing.active_annotation_scale() {
            Some(active) => rows.push(diagnostic(
                "scalelistedit.active",
                format!(
                    "{report_command}: the active annotation scale (CANNOSCALE) is '{}' (value \
                     {}{})",
                    active.name,
                    active.value,
                    if active.named {
                        ", resolved in the table"
                    } else {
                        ", NOT in the table — annotative rendering falls back to the header value"
                    }
                ),
            )),
            None => rows.push(diagnostic(
                "scalelistedit.active_unset",
                format!(
                    "{report_command}: this drawing stores no CANNOSCALE value, so no annotation \
                     scale is current"
                ),
            )),
        }
        match drawing.layouts().next() {
            Some(layout) => {
                let record = drawing.plot_settings_for(layout.id);
                rows.push(diagnostic(
                    "scalelistedit.summary",
                    format!(
                        "{report_command}: layout '{}' page {} ({} x {} {}), scale {}:{}",
                        layout.name,
                        record.paper_size_name,
                        record.paper_width,
                        record.paper_height,
                        match record.paper_units {
                            cad_db::PlotPaperUnits::Millimeters => "mm",
                            cad_db::PlotPaperUnits::Inches => "in",
                            cad_db::PlotPaperUnits::Pixels => "px",
                        },
                        record.scale_numerator,
                        record.scale_denominator
                    ),
                ))
            }
            None => rows.push(diagnostic(
                "scalelistedit.summary",
                format!(
                    "{report_command}: this drawing has no paper layout, so there is no page to \
                     report; the scale table above is the drawing's whole ACAD_SCALELIST"
                ),
            )),
        }

        rows.push(diagnostic(
            "scalelistedit.grammar",
            format!(
                "{report_command}: writing is available as `{report_command} ADD <name> \
                 <paper_units> <drawing_units>`, `RENAME <old> <new>` and `DELETE <name>`; the \
                 modal scale-list dialog and `RESET` are NOT IMPLEMENTED"
            ),
        ));
        Ok(report_outcome(rows))
    }

    /// Apply one write and report what the database actually did.
    fn scale_list_write(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        op: &ScaleListCommand,
        report_command: &str,
    ) -> CadResult<CommandOutcome> {
        // The two selectors stay Viewer-legal because they also carry the
        // read-only report; the write itself is refused up front instead of
        // letting a Viewer session edit the drawing (the same rule `COLOR` and
        // the named style tables apply).
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let document_id = command.document;
        let drawing = document_drawing(self, command)?.clone();
        let transaction = crate::app_drawing::next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        // The message is built first: the change set is moved into the outcome
        // below and its revision is part of the text.
        let (changeset, code, message) = match op {
            ScaleListCommand::Add {
                name,
                paper_units,
                drawing_units,
            } => {
                let (changeset, row) = candidate.create_scale(
                    name,
                    *paper_units,
                    *drawing_units,
                    transaction,
                    "scalelistedit add",
                )?;
                let message = format!(
                    "{report_command} ADD: created scale '{}' (paper/drawing = {}/{}, factor \
                     {}); the ACAD_SCALELIST gained one row in revision {}, and this write is not \
                     undoable (cad-history has no scale-row patch kind)",
                    row.name,
                    row.paper_units,
                    row.drawing_units,
                    row.factor(),
                    changeset.after.0
                );
                (changeset, "scalelistedit.added", message)
            }
            ScaleListCommand::Rename { old, new } => {
                let changeset =
                    candidate.rename_scale(old, new, transaction, "scalelistedit rename")?;
                let rebuilt = changeset.changes.len();
                let message = format!(
                    "{report_command} RENAME: renamed scale '{}' to '{}' and retargeted the \
                     CANNOSCALE value plus every annotative override that named it, in one \
                     revision{}; this write is not undoable",
                    old.trim(),
                    new.trim(),
                    if rebuilt == 0 {
                        String::new()
                    } else {
                        format!(" ({rebuilt} annotative entity representation(s) rebuilt)")
                    }
                );
                (changeset, "scalelistedit.renamed", message)
            }
            ScaleListCommand::Delete { name } => {
                let changeset =
                    candidate.delete_scale(name, transaction, "scalelistedit delete")?;
                let message = format!(
                    "{report_command} DELETE: removed scale '{}' from the ACAD_SCALELIST in one \
                     revision; the viewport and plot-setting scales keep their stored transform \
                     and numeric ratio (neither names a scale row), and this write is not undoable",
                    name.trim()
                );
                (changeset, "scalelistedit.deleted", message)
            }
        };
        let changed = changeset.before != changeset.after;
        if changed {
            // Publish the validated candidate exactly like the other post-import
            // table writes do.
            self.workspace
                .documents
                .get_mut(&document_id)
                .ok_or_else(|| CadError::Invariant("document disappeared while publishing".into()))?
                .drawing = Arc::new(candidate);
        }
        let mut diagnostics = vec![diagnostic(
            code,
            if changed {
                message
            } else {
                // A rename to the stored spelling is an authoritative no-op: no
                // revision, no publish, and the text says exactly that.
                format!(
                    "{message} — but the stored name already matched, so no row was rewritten and \
                     no revision was raised"
                )
            },
        )];
        diagnostics.push(diagnostic(
            "scalelistedit.no_history",
            format!(
                "{report_command}: a real write publishes one revision and one ChangeSet and \
                 records NO undo step; cad-history has no scale-row patch kind, so UNDO will not \
                 restore the previous scale table"
            ),
        ));
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(changeset)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}
