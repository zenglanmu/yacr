//! `AUDIT`: report invalid stored entities, and (on `Fix = Yes`) repair the
//! provably repairable ones.
//!
//! ## The prompt and the two modes
//!
//! AutoCAD asks **`Fix any errors detected? [Yes/No]`**, default **No**. The
//! answer is the typed [`AuditRequest`] payload; `CommandPayload::None` is the
//! default **No**.
//!
//! * **No** — the detect-only report, byte-compatible with the original v1: one
//!   `audit.clean` diagnostic on a clean drawing, or an `audit.summary` followed
//!   by one `audit.invalid_entity` per stored entity that fails
//!   [`cad_db::validate_entity`]. Nothing is repaired, rewritten or deleted; the
//!   command is read-only (no transaction, revision bump or undo entry).
//! * **Yes** — the documented report structure: a header-variable pass, a block
//!   definition pass, object pass 1 (identity/references) and object pass 2
//!   (geometry), the repairs, and a final
//!   `Total errors found N fixed M` diagnostic. Each repair is reported with the
//!   entity id and the action taken; the batch is one transaction.
//!
//! ## What is actually repairable
//!
//! A repair is written only when the database's own write validator
//! ([`cad_db::validate_entity`]) **accepts the repaired row** — the authority
//! proves repairability, so there is no second error catalogue here. Two
//! normalizations meet that bar:
//!
//! * an `EntityId`/`ObjectId` disagreement is normalized to the documented shared
//!   value (the importer allocates both from one counter);
//! * a `TEXT`/`MTEXT` run whose stored wrap width is non-finite or `<= 0` gets
//!   `wrap_width: None`, the documented "no wrap" form.
//!
//! Everything else the validator rejects is **not repairable** here and is
//! reported only: degenerate geometry (a zero-length LINE, a zero radius, an
//! empty polyline — there is no canonical value to write back), a missing
//! layer/layout/block reference, a missing block name, an inconsistent block
//! member list, and a non-finite or non-positive header variable. Recreating a
//! missing layer or renaming a block would invent data; RECOVER is out of scope.
//!
//! ### Why a repair is not an undo step
//!
//! The project's undo path restores the recorded *before* row through the same
//! validated write path (`cad-db`'s `apply_drawing_changes…`). A repaired row's
//! before-state is invalid **by definition** (that is why AUDIT found it), so an
//! undo step could never be applied: it would be refused by the validator midway
//! through the history. The repair is therefore applied atomically in one
//! transaction and **not** placed on the undo stack, and the command says so
//! (`audit.repairs_not_undoable`). Deleting invalid entities as a "purge" would
//! hit the exact same undo wall, which is why this build repairs fields instead.
//!
//! ## Previous selection set
//!
//! AutoCAD places the objects with errors in their **current space** into the
//! `Previous` selection set. This build has no `Previous` set, so the mapped
//! behaviour is: when at least one error object exists in the current space, the
//! session selection is replaced by those objects (the still-existing ones after
//! a repair). A clean audit leaves the selection untouched.
//!
//! ## `AUDITCTL`
//!
//! AutoCAD's `AUDITCTL=1` writes the `.adt` audit log. That is host file I/O and
//! this command layer has no file handle for it, so
//! [`AuditRequest::auditctl_log`] is the named `audit.auditctl_log` refusal.
//! Neither mode writes a log here.

use cad_db::{validate_entity, validate_geometry, DbEntity, DrawingDatabase};
use cad_domain::{CadError, CadResult, Diagnostic, EntityId, InstancePath, ObjectId, SelectionRef};

use crate::app_drawing::next_transaction_id;
use crate::{
    AppMode, Application, Command, CommandOutcome, CommandPayload, SelectionSet,
    SelectionToolPayload, SessionState,
};

/// The answers of the AUDIT prompt(s).
///
/// `fix` is `Fix any errors detected? [Yes/No]`; the default is **No**.
/// `auditctl_log` asks for the AutoCAD `AUDITCTL` `.adt` log, which is host file
/// I/O and therefore a named refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuditRequest {
    /// `Yes` = attempt the provable repairs; `No` (default) = detect only.
    pub fix: bool,
    /// `AUDITCTL=1`: request the `.adt` log (refused by name).
    pub auditctl_log: bool,
}

impl Application {
    pub(crate) fn audit_drawing(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let request = audit_request(command)?;
        if request.auditctl_log {
            return Err(CadError::Unsupported(
                "audit.auditctl_log: the AUDITCTL .adt audit log is host file I/O; this command \
                 layer has no file handle for it"
                    .into(),
            ));
        }
        if request.fix {
            if session.app_mode() == AppMode::Viewer {
                return Err(CadError::PermissionDenied);
            }
            return self.audit_and_repair(session, command);
        }
        self.audit_report_only(session, command)
    }

    /// `Fix = No`: the byte-compatible detect-only report.
    fn audit_report_only(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let document = self.audit_document(command)?;
        let database = document.drawing.as_ref();
        let mut checked = 0usize;
        let mut invalid = 0usize;
        let mut diagnostics = Vec::new();
        let mut error_entities: Vec<EntityId> = Vec::new();
        for entity in database.entities() {
            checked += 1;
            if let Err(error) = validate_entity(database, entity.id, entity) {
                invalid += 1;
                error_entities.push(entity.id);
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
        self.publish_error_selection(session, command.document, &error_entities);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `Fix = Yes`: the sectioned report plus the provable repairs.
    fn audit_and_repair(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let document = self.audit_document(command)?;
        let database = document.drawing.as_ref();
        let mut diagnostics = Vec::new();
        let mut errors = 0usize;

        // --- header variables -------------------------------------------------
        let header_findings = header_findings(database);
        diagnostics.push(Diagnostic {
            object: None,
            code: "audit.header".into(),
            message: if header_findings.is_empty() {
                "AUDIT: header variables: 3 checked; no invalid value".to_string()
            } else {
                format!(
                    "AUDIT: header variables: 3 checked; {} invalid",
                    header_findings.len()
                )
            },
        });
        for finding in &header_findings {
            errors += 1;
            diagnostics.push(Diagnostic {
                object: None,
                code: "audit.invalid_header".into(),
                message: finding.clone(),
            });
        }

        // --- block definitions ------------------------------------------------
        let block_findings = block_findings(database);
        let block_count = database.blocks().count();
        diagnostics.push(Diagnostic {
            object: None,
            code: "audit.blocks".into(),
            message: if block_findings.is_empty() {
                format!("AUDIT: {block_count} block definition(s) checked; no invalid definition")
            } else {
                format!(
                    "AUDIT: {block_count} block definition(s) checked; {} invalid",
                    block_findings.len()
                )
            },
        });
        for finding in &block_findings {
            errors += 1;
            diagnostics.push(Diagnostic {
                object: None,
                code: "audit.invalid_block".into(),
                message: finding.clone(),
            });
        }

        // --- object passes ----------------------------------------------------
        let mut checked = 0usize;
        let mut pass1: Vec<(EntityId, String)> = Vec::new();
        let mut pass2: Vec<(EntityId, String)> = Vec::new();
        let mut planned: Vec<(EntityId, DbEntity, Vec<String>)> = Vec::new();
        for entity in database.entities() {
            checked += 1;
            if let Some(error) = reference_error(database, entity.id, entity) {
                pass1.push((entity.id, format!("{error:?}")));
            }
            if let Err(error) = validate_geometry(&entity.geometry) {
                pass2.push((entity.id, format!("{error:?}")));
            }
            if let Some(repair) = planned_repair(database, entity) {
                planned.push(repair);
            }
        }
        diagnostics.push(Diagnostic {
            object: None,
            code: "audit.pass1".into(),
            message: format!(
                "AUDIT: object pass 1 (references): {checked} entities checked; {} invalid",
                pass1.len()
            ),
        });
        for (id, text) in &pass1 {
            errors += 1;
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(id.0)),
                code: "audit.invalid_entity".into(),
                message: format!("entity {}: {}", id.0, ascii_safe(text)),
            });
        }
        diagnostics.push(Diagnostic {
            object: None,
            code: "audit.pass2".into(),
            message: format!(
                "AUDIT: object pass 2 (geometry): {checked} entities checked; {} invalid",
                pass2.len()
            ),
        });
        for (id, text) in &pass2 {
            errors += 1;
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(id.0)),
                code: "audit.invalid_entity".into(),
                message: format!("entity {}: {}", id.0, ascii_safe(text)),
            });
        }

        // --- repairs ----------------------------------------------------------
        //
        // Only a normalization that the *validator then accepts* is written, so
        // "provably repairable" is checked by the authority itself rather than
        // by a second, drifting rule set. The write is one transaction. It is
        // deliberately **not** placed on the undo stack: undo restores the
        // previous row through the same validated write path, and that row is by
        // definition still invalid, so the undo step could never apply. See the
        // module docs.
        let repaired = planned.len();
        let mut changes = None;
        if !planned.is_empty() {
            let mut candidate = (*database).clone();
            let transaction = next_transaction_id();
            let change_set = {
                let mut staged =
                    candidate.begin_drawing_transaction("audit repair", transaction)?;
                for (_, fixed, _) in &planned {
                    staged.update_entity(fixed.clone())?;
                }
                staged.commit()?
            };
            let document = self
                .workspace
                .documents
                .get_mut(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            document.drawing = std::sync::Arc::new(candidate);
            changes = Some(std::sync::Arc::new(change_set));
        }
        for (id, _, actions) in &planned {
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(id.0)),
                code: "audit.repaired".into(),
                message: format!("entity {}: {}", id.0, actions.join("; ")),
            });
        }
        if repaired > 0 {
            diagnostics.push(Diagnostic {
                object: None,
                code: "audit.repairs_not_undoable".into(),
                message: "AUDIT: repairs are applied atomically but are not placed on the undo \
                          stack; undo would have to restore a row the write validator rejects"
                    .into(),
            });
        }

        // --- Previous-selection mapping --------------------------------------
        let mut error_entities: Vec<EntityId> = pass1.iter().map(|(id, _)| *id).collect();
        error_entities.extend(pass2.iter().map(|(id, _)| *id));
        error_entities.sort_by_key(|id| id.0);
        error_entities.dedup();
        self.publish_error_selection(session, command.document, &error_entities);

        diagnostics.push(Diagnostic {
            object: None,
            code: "audit.total".into(),
            message: format!("Total errors found {errors} fixed {repaired}"),
        });
        Ok(CommandOutcome {
            objects: planned.iter().map(|(id, _, _)| ObjectId(id.0)).collect(),
            changes,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    fn audit_document(&self, command: &Command) -> CadResult<&crate::Document> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        if document.id != command.document {
            return Err(CadError::StaleResult);
        }
        Ok(document)
    }

    /// The mapped `Previous` selection set: the still-existing error objects of
    /// the active space. An empty set leaves the session selection untouched.
    fn publish_error_selection(
        &self,
        session: &mut SessionState,
        document: cad_domain::DocumentId,
        error_entities: &[EntityId],
    ) {
        let Some(document_ref) = self.workspace.documents.get(&document) else {
            return;
        };
        let mut set = SelectionSet::new();
        for id in error_entities {
            let Some(entity) = document_ref.drawing.entity(*id) else {
                continue;
            };
            if entity.space != session.active_space {
                continue;
            }
            set.insert(SelectionRef {
                document,
                entity: *id,
                instance: InstancePath::default(),
                sub_element: None,
            });
        }
        if !set.is_empty() {
            session.selection = set;
            session.begin_selecting();
        }
    }
}

/// Read the typed answers; `None` is the documented default **No**.
fn audit_request(command: &Command) -> CadResult<AuditRequest> {
    match &command.payload {
        CommandPayload::None => Ok(AuditRequest::default()),
        CommandPayload::SelectionTool(payload) => match payload.as_ref() {
            SelectionToolPayload::Audit(request) => Ok(*request),
            _ => Err(CadError::InvalidInput("AUDIT needs no payload".into())),
        },
        _ => Err(CadError::InvalidInput("AUDIT needs no payload".into())),
    }
}

/// The normalizations this build can prove: the repaired row is accepted by
/// `cad_db::validate_entity`, which is the authority (no second catalogue).
///
/// Two classes are written:
///
/// * an `ObjectId`/`EntityId` disagreement is normalized to the documented
///   shared value (the importer allocates both from one counter);
/// * a `TEXT`/`MTEXT` run whose stored wrap width is non-finite or `<= 0` gets
///   `wrap_width: None` (the documented "no wrap" form).
///
/// Everything else the validator rejects — degenerate geometry, a missing
/// layer/layout/block — has no canonical fixed value here and is reported only.
fn planned_repair(
    database: &DrawingDatabase,
    entity: &DbEntity,
) -> Option<(EntityId, DbEntity, Vec<String>)> {
    let mut fixed = entity.clone();
    let mut actions: Vec<String> = Vec::new();
    if fixed.object.id.0 != fixed.id.0 {
        fixed.object.id = ObjectId(fixed.id.0);
        actions.push(format!("normalized object id to {}", fixed.id.0));
    }
    if let cad_domain::SemanticGeometry::Text { wrap_width, .. } = &mut fixed.geometry {
        if let Some(width) = *wrap_width {
            if !width.is_finite() || width <= 0.0 {
                *wrap_width = None;
                actions.push("cleared the invalid text wrap width".to_string());
            }
        }
    }
    if actions.is_empty() {
        return None;
    }
    // The validator is the proof of repairability.
    validate_entity(database, fixed.id, &fixed).ok()?;
    Some((entity.id, fixed, actions))
}

/// The identity/reference half of `cad_db::validate_entity`.
///
/// The messages are the validator's own, so a finding reported by pass 1 reads
/// exactly like the detect-only report. Geometry is deliberately **not** checked
/// here: that is object pass 2.
fn reference_error(
    database: &DrawingDatabase,
    id: EntityId,
    entity: &DbEntity,
) -> Option<CadError> {
    if entity.id != id {
        return Some(CadError::Invariant(format!(
            "change key {id:?} does not match entity id {:?}",
            entity.id
        )));
    }
    if entity.object.id.0 != id.0 {
        return Some(CadError::Invariant(format!(
            "entity {id:?} has object id {:?}; the object and entity id must share the same value",
            entity.object.id
        )));
    }
    if database.layer(entity.layer).is_none() {
        return Some(CadError::Invariant(format!(
            "entity {id:?} references missing layer {:?}",
            entity.layer
        )));
    }
    match &entity.space {
        cad_domain::SpaceId::Model => {}
        cad_domain::SpaceId::Paper(layout) => {
            if database.layout(*layout).is_none() {
                return Some(CadError::Invariant(format!(
                    "entity {id:?} references missing layout {layout:?}"
                )));
            }
        }
        cad_domain::SpaceId::Block(block) => {
            if database.block(*block).is_none() {
                return Some(CadError::Invariant(format!(
                    "entity {id:?} references missing block {block:?}"
                )));
            }
        }
    }
    if let cad_domain::SemanticGeometry::Insert { block, .. } = &entity.geometry {
        if database.block(*block).is_none() {
            return Some(CadError::Invariant(format!(
                "entity {id:?} references missing block {block:?}"
            )));
        }
    }
    None
}

/// The three header variables this build can judge.
///
/// `$LTSCALE` must be finite and positive; `$PDSIZE` must be finite (`0` is the
/// documented viewport-relative form); `$DWGCODEPAGE`, when present, must not be
/// blank. All three are stored raw by `cad-db`, so a corrupt header stays
/// observable and this pass reports it instead of normalising it.
fn header_findings(database: &DrawingDatabase) -> Vec<String> {
    let mut findings = Vec::new();
    let scale = database.linetype_scale();
    if !scale.is_finite() || scale <= 0.0 {
        findings.push(format!(
            "header $LTSCALE: {} must be finite and positive",
            ascii_safe(&format!("{scale}"))
        ));
    }
    let size = database.point_display_size();
    if !size.is_finite() {
        findings.push(format!(
            "header $PDSIZE: {} must be finite",
            ascii_safe(&format!("{size}"))
        ));
    }
    if let Some(code_page) = database.code_page() {
        if code_page.trim().is_empty() {
            findings.push("header $DWGCODEPAGE: empty code page".to_string());
        }
    }
    findings
}

/// Block-definition findings: a missing/blank name, a missing member, a member
/// listed twice, or a member stored in another space.
fn block_findings(database: &DrawingDatabase) -> Vec<String> {
    let mut findings = Vec::new();
    for block in database.blocks() {
        let name = database.block_name(block.id).unwrap_or("");
        if name.trim().is_empty() {
            findings.push(format!("block {}: no registered name", block.id.0));
        }
        let mut seen: Vec<EntityId> = Vec::new();
        for member in &block.entities {
            if seen.contains(member) {
                findings.push(format!(
                    "block {}: member {} is listed more than once",
                    block.id.0, member.0
                ));
                continue;
            }
            seen.push(*member);
            match database.entity(*member) {
                None => findings.push(format!(
                    "block {}: member {} does not exist",
                    block.id.0, member.0
                )),
                Some(entity) if entity.space != cad_domain::SpaceId::Block(block.id) => findings
                    .push(format!(
                        "block {}: member {} is stored in another space",
                        block.id.0, member.0
                    )),
                Some(_) => {}
            }
        }
    }
    findings
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
