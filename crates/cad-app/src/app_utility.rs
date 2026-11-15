//! The 2026-11-07 utility/named-object batch: `PURGE`, `RENAME`, `SETVAR`,
//! `TIME`, `DBLIST`, `TREESTAT`, `MREDO` and `MULTIPLE`.
//!
//! # Honest boundaries
//!
//! * **PURGE** removes unused rows from four tables. Layers ride the existing
//!   `DELETELAYER` command path (which is also why a layer in use is refused: the
//!   database never re-points an entity's layer) and, as before, stay outside the
//!   undo lane. Block definitions, text styles and linetypes go through the
//!   database's single-purpose delete APIs
//!   ([`cad_db::DrawingDatabase::delete_block`] / `delete_style` /
//!   `delete_linetype`) and are staged as **one batch / one transaction / one undo
//!   step** per invocation, so undo restores every purged row and every purged
//!   block member. Each in-use object is a named per-object refusal
//!   (`block.in_use`, `style.in_use`, `style.standard_undeletable`,
//!   `linetype.in_use`, `linetype.continuous_undeletable`) reported as
//!   `purge.<kind>_refused`; a refusal removes nothing.
//! * **RENAME** addresses every kind that has a post-import write API: `LAYER`
//!   rides the existing `RENAMELAYER` command, the three style tables use the
//!   shared style-table batch, `BLOCK` renames the block definition, and
//!   `LINETYPE` renames the linetype row plus every stored name reference to it
//!   (see [`Self::rename_linetype_command`]). No `RENAME` kind is silently
//!   ignored; an unknown/duplicate/illegal name is a stable named refusal.
//! * **SETVAR** only *reports* here. The typed write form is parsed in
//!   `cad-ui-core` and dispatched to the existing `PDMODE`/`PDSIZE`/`LTSCALE`/
//!   `SNAPUNIT`/`GRID`/`SNAP`/`ORTHO` handlers, so this module never becomes a
//!   second write path. `TEXTSIZE` writes use the existing session
//!   `set_tool_parameter` API.
//! * **TIME** reports the document revision, entity count and the current UTC
//!   clock. Drawing creation/edit timestamps are **NOT TRACKED** (the database
//!   stores no timestamps) and session uptime is **NOT TRACKED**; both are named
//!   diagnostics, never invented dates. Local-time rendering is also NOT TRACKED
//!   (this build has no local-offset dependency); only UTC is printed.
//! * **DBLIST** lists top-level (model/paper) entities. Block-definition members
//!   are not top-level and are counted separately as skipped, never silently
//!   mixed into the list.
//! * **TREESTAT** reports the spatial index as **NOT TRACKED**: the application
//!   session does not retain a persistent index, and `cad-spatial`'s
//!   `GridSpatialIndex` is built per pick query. No fake entry count is emitted.
//! * **MREDO** applies independent existing redo records (`CommandId::Redo`), one
//!   history move per step. It refuses an empty redo stack and reports a partial
//!   run when fewer steps are available than requested.
//! * **MULTIPLE** records the session repeat flag. This host has no command
//!   repeat loop, so the arming succeeds and the actual repetition is reported as
//!   `multiple.repeat_not_implemented`; the command never claims it repeated
//!   anything.
//!
//! `RENAME LAYER` reuses the ordinary `RENAMELAYER` command, so it records the
//! same one-step layer patch; `PURGE` still publishes its layer deletes without
//! history (out of scope for the layer-undo lane), while its block/style/linetype
//! deletes are one undo step. Every report path writes nothing, bumps no revision
//! and records no history.

use super::*;

use crate::app_drawing::next_transaction_id;
use std::time::{SystemTime, UNIX_EPOCH};

impl Application {
    /// Dispatch the write/session half of the utility batch.
    pub(crate) fn utility_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.id {
            CommandId::Purge => self.purge(session, command),
            CommandId::Rename => self.rename(session, command),
            CommandId::MRedo => self.mredo(session, command),
            CommandId::Multiple => self.multiple(session, command),
            other => Err(CadError::Invariant(format!(
                "not a utility command: {other:?}"
            ))),
        }
    }

    /// Dispatch the read-only half of the utility batch.
    pub(crate) fn utility_report(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.id {
            CommandId::SetVar => self.setvar_report(session, command),
            CommandId::Time => self.time_report(command),
            CommandId::DbList => self.dblist_report(command),
            CommandId::TreeStat => self.treestat_report(command),
            other => Err(CadError::Invariant(format!(
                "not a utility report: {other:?}"
            ))),
        }
    }

    /// `RENAME LAYER` reuses the ordinary `RENAMELAYER` command (same database
    /// write, same refusals for unknown, duplicate and illegal names). The three
    /// style tables use the shared style-table batch and `BLOCK` the block-row
    /// patch; `LINETYPE` renames the table row and cascades every stored name
    /// reference in the same revision.
    fn rename(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Rename(rename) = &command.payload else {
            return Err(CadError::InvalidInput(
                "RENAME needs `RENAME <kind> <old> <new>`".into(),
            ));
        };
        match rename.kind {
            NamedObjectKind::Layer => {
                let inner = with_payload(
                    command,
                    CommandId::RenameLayer,
                    CommandPayload::RenameLayer {
                        layer: rename.old.clone(),
                        name: rename.new.clone(),
                    },
                );
                self.layer_command(session, &inner)
            }
            // The three style tables now have a post-import write API, so their
            // rename is the same one-transaction/one-undo path the commands use.
            NamedObjectKind::Style => self.apply_style_table_op(
                session,
                command,
                crate::command_payload::StyleTableKind::TextStyle,
                crate::command_payload::StyleTableOp::Rename {
                    old: rename.old.clone(),
                    new: rename.new.clone(),
                },
            ),
            NamedObjectKind::DimStyle => self.apply_style_table_op(
                session,
                command,
                crate::command_payload::StyleTableKind::DimStyle,
                crate::command_payload::StyleTableOp::Rename {
                    old: rename.old.clone(),
                    new: rename.new.clone(),
                },
            ),
            NamedObjectKind::MLeaderStyle => self.apply_style_table_op(
                session,
                command,
                crate::command_payload::StyleTableKind::MLeaderStyle,
                crate::command_payload::StyleTableOp::Rename {
                    old: rename.old.clone(),
                    new: rename.new.clone(),
                },
            ),
            NamedObjectKind::Block => {
                self.rename_block_command(session, command, &rename.old, &rename.new)
            }
            NamedObjectKind::Linetype => {
                self.rename_linetype_command(session, command, &rename.old, &rename.new)
            }
        }
    }

    /// `RENAME LINETYPE <old> <new>`: rename one linetype table row.
    ///
    /// The table write and its cascade are one [`cad_db`] batch
    /// ([`cad_db::DrawingDatabase::apply_linetype_rename`]): the row's name, every
    /// layer attribute that names the old linetype and every entity whose
    /// explicit linetype keeps the name are retargeted in the same revision, so a
    /// later save/export can never contain a dangling linetype name.
    ///
    /// Refusals (nothing written, no revision, no history): unknown and ambiguous
    /// names (`linetype.unknown`/`linetype.ambiguous`), an empty/illegal new name
    /// (`linetype.name_empty`/`linetype.name_illegal_char`), a duplicate
    /// (`linetype.duplicate_name`), the drawing default `Continuous`
    /// (`linetype.continuous_unrenameable` — it is the implicit value of every
    /// layer row without stored attributes) and Viewer mode
    /// ([`CadError::PermissionDenied`]). Renaming to the stored spelling is an
    /// authoritative no-op (`rename.unchanged`) that records nothing.
    fn rename_linetype_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        old: &str,
        new: &str,
    ) -> CadResult<CommandOutcome> {
        use crate::app_reports_ext::{diagnostic, report_outcome};

        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let drawing = document_drawing(self, command)?;
        let id = crate::app_drawing::resolve_unique(
            "linetype",
            "linetype",
            old,
            drawing
                .linetypes()
                .map(|row| (row.id, row.name.clone()))
                .collect(),
        )?;
        let before = drawing.linetype(id).cloned().ok_or_else(|| {
            CadError::InvalidInput(format!(
                "linetype.unknown: this drawing has no linetype named '{}'",
                old.trim()
            ))
        })?;
        let trimmed = new.trim();
        if before.name == trimmed {
            return Ok(report_outcome(vec![diagnostic(
                "rename.unchanged",
                format!(
                    "RENAME LINETYPE: '{}' already has that name; nothing changed",
                    before.name
                ),
            )]));
        }
        // Count the cascade from the current state so the report describes what
        // the batch really retargeted, never a guess.
        let layer_refs = drawing
            .layers()
            .filter(|layer| {
                drawing
                    .layer_attributes(layer.id)
                    .linetype
                    .eq_ignore_ascii_case(&before.name)
            })
            .count();
        let entity_refs = drawing
            .entities()
            .filter(|entity| {
                matches!(
                    &drawing.entity_render_attributes(entity.id).linetype,
                    cad_db::EntityLineType::Explicit { name, .. }
                        if name.eq_ignore_ascii_case(&before.name)
                )
            })
            .count();

        let mut after = before.clone();
        after.name = trimmed.to_string();
        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = candidate.apply_linetype_rename("rename linetype", transaction, &after)?;
        let stored = candidate.linetype(id).cloned().ok_or_else(|| {
            CadError::Invariant("the renamed linetype row vanished after the write".into())
        })?;
        self.publish_candidate_with_linetypes(
            command.document,
            candidate,
            cad_history::DrawingUndoRecord {
                transaction,
                label: "rename linetype".to_string(),
                patches: Vec::new(),
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                merge_key: None,
            },
            vec![cad_history::linetype_patch(
                id,
                Some(before.clone()),
                Some(stored.clone()),
            )],
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![diagnostic(
                "rename.linetype",
                format!(
                    "RENAME LINETYPE: renamed '{}' to '{}' and retargeted {layer_refs} layer \
                     row(s) and {entity_refs} entity attribute reference(s) in the same \
                     transaction (one undo step)",
                    before.name, stored.name
                ),
            )],
            measurement: None,
            point_report: None,
        })
    }

    /// `MREDO [n]`: apply up to `n` existing redo records.
    fn mredo(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let requested = match &command.payload {
            CommandPayload::None => 1,
            CommandPayload::MRedo(count) => *count,
            _ => {
                return Err(CadError::InvalidInput(
                    "MREDO takes one positive integer or no argument".into(),
                ))
            }
        };
        if requested == 0 {
            return Err(CadError::InvalidInput(
                "mredo.count: MREDO needs a positive number of steps".into(),
            ));
        }
        if !self.can_redo(&command.document) {
            return Err(CadError::InvalidInput(
                "mredo.empty: there is no redo record to apply in this document".into(),
            ));
        }

        let mut done = 0u32;
        let mut changes = None;
        while done < requested && self.can_redo(&command.document) {
            let outcome = self.redo(command)?;
            self.reconcile_selection(session);
            changes = outcome.changes;
            done += 1;
        }
        let code = if done == requested {
            "mredo.applied"
        } else {
            "mredo.partial"
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes,
            diagnostics: vec![Diagnostic {
                object: None,
                code: code.into(),
                message: format!(
                    "MREDO: applied {done} existing redo record(s) of {requested} requested; \
                     each step is one ordinary redo history move"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `MULTIPLE [command]`: record the session repeat flag.
    ///
    /// The flag is real session state, but this host has no command repeat loop.
    /// The command therefore reports both the arming and the named boundary; it
    /// never claims that anything was repeated.
    fn multiple(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let target = match &command.payload {
            CommandPayload::None => None,
            CommandPayload::Multiple(target) => target.clone(),
            _ => {
                return Err(CadError::InvalidInput(
                    "MULTIPLE takes one command name or no argument".into(),
                ))
            }
        };
        session.arm_multiple(target.clone());
        let label = target.as_deref().unwrap_or("<the next command>");
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "multiple.armed".into(),
                    message: format!(
                        "MULTIPLE: session repeat flag armed for {label}; this is session state \
                         only"
                    ),
                },
                Diagnostic {
                    object: None,
                    code: "multiple.repeat_not_implemented".into(),
                    message: "MULTIPLE: this host has no command repeat loop, so no command was \
                              repeated; a caller must implement the loop before repetition is \
                              available"
                        .into(),
                },
            ],
            measurement: None,
            point_report: None,
        })
    }

    /// `TIME`: read-only document/clock report.
    ///
    /// One line per AutoCAD `TIME` field. Only the fields this build really has a
    /// source for are valued: the current UTC clock and the document's stored
    /// revision/entity/table state. Creation time, last update, the elapsed
    /// command time and the elapsed timer are not tracked anywhere in `cad-db` or
    /// in the session, so each names itself as `time.not_tracked` instead of
    /// printing a fabricated date, a zero or a plausible duration.
    fn time_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "TIME")?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = vec![
            Diagnostic {
                object: None,
                code: "time.document".into(),
                message: format!(
                    "TIME: document {} database {} revision {} entities {}",
                    command.document.0,
                    drawing.id().0,
                    drawing.revision().0,
                    drawing.entity_count()
                ),
            },
            Diagnostic {
                object: None,
                code: "time.tracked".into(),
                message: "TIME: tracked by this build: the current UTC clock (no local-offset \
                           source), the document id, its revision and its entity/table counts"
                    .into(),
            },
            // One line per unmodelled AutoCAD TIME field, each named.
            Diagnostic {
                object: None,
                code: "time.not_tracked".into(),
                message: "time.not_tracked: drawing creation time (this database stores no \
                          timestamps)"
                    .into(),
            },
            Diagnostic {
                object: None,
                code: "time.not_tracked".into(),
                message: "time.not_tracked: drawing last-update time (this database stores no \
                          timestamps; the revision counter is the only change evidence)"
                    .into(),
            },
            Diagnostic {
                object: None,
                code: "time.not_tracked".into(),
                message:
                    "time.not_tracked: elapsed command time (no per-command timer exists; the \
                          session records no start/stop instants)"
                        .into(),
            },
            Diagnostic {
                object: None,
                code: "time.not_tracked".into(),
                message: "time.not_tracked: elapsed timer and timer on/off state (no elapsed-time \
                          accumulator exists in this build)"
                    .into(),
            },
            Diagnostic {
                object: None,
                code: "time.uptime_not_tracked".into(),
                message: "TIME: session uptime is not tracked by this build".into(),
            },
            Diagnostic {
                object: None,
                code: "time.local_not_tracked".into(),
                message: "TIME: local-time rendering is not tracked by this build; only UTC is \
                          reported"
                    .into(),
            },
        ];
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => {
                let seconds = elapsed.as_secs();
                diagnostics.push(Diagnostic {
                    object: None,
                    code: "time.utc".into(),
                    message: format!("TIME: current UTC {} (unix {seconds})", format_utc(seconds)),
                });
            }
            Err(_) => diagnostics.push(Diagnostic {
                object: None,
                code: "time.utc_unavailable".into(),
                message: "TIME: the system clock is before the Unix epoch; no UTC time was \
                          fabricated"
                    .into(),
            }),
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `DBLIST`: list every top-level entity in the open document.
    fn dblist_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "DBLIST")?;
        let drawing = document_drawing(self, command)?;
        let mut rows: Vec<&cad_db::DbEntity> = Vec::new();
        let mut block_members = 0usize;
        for entity in drawing.entities() {
            if matches!(entity.space, SpaceId::Block(_)) {
                block_members += 1;
            } else {
                rows.push(entity);
            }
        }
        rows.sort_by_key(|entity| (space_rank(&entity.space), entity.draw_order, entity.id.0));
        let mut diagnostics = Vec::with_capacity(rows.len() + 2);
        for entity in &rows {
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "dblist.object".into(),
                message: format!(
                    "DBLIST id={} handle={} type={} layer={} space={} draw_order={}",
                    entity.id.0,
                    entity
                        .object
                        .source_handle
                        .as_deref()
                        .map(str::trim)
                        .filter(|handle| !handle.is_empty())
                        .unwrap_or("-"),
                    entity.object.type_key,
                    layer_label(&drawing, entity.layer),
                    space_label(&entity.space),
                    entity.draw_order
                ),
            });
        }
        diagnostics.push(Diagnostic {
            object: None,
            code: "dblist.summary".into(),
            message: format!(
                "DBLIST: {} top-level entity(ies) in drawing revision {}",
                rows.len(),
                drawing.revision().0
            ),
        });
        if block_members > 0 {
            diagnostics.push(Diagnostic {
                object: None,
                code: "dblist.scope".into(),
                message: format!(
                    "DBLIST: {block_members} block-definition member(s) are not top-level and were \
                     not listed"
                ),
            });
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `TREESTAT`: report the spatial-index state honestly.
    fn treestat_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "TREESTAT")?;
        // Touch the document so the command is still document-scoped; no index
        // state is read because none is retained.
        let _ = document_drawing(self, command)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "treestat.not_tracked".into(),
                message: "TREESTAT: the spatial index is not tracked by this build. The \
                          application session retains no persistent index; cad-spatial's \
                          GridSpatialIndex is built per pick query, so no tree size or depth is \
                          reported (no fake count)."
                    .into(),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The named-object kinds one `PURGE` target covers.
///
/// `PURGE LAYER` touches no table; the bare `PURGE` covers all three tables. An
/// unsupported kind can no longer be requested: `PurgeTarget` has no DIMSTYLE /
/// MLEADERSTYLE variant, and `NamedObjectKind`'s other arms are refused by the
/// `RENAME`/`-PURGE` grammar rather than silently ignored.
pub(crate) fn purge_table_kinds(target: PurgeTarget) -> Vec<NamedObjectKind> {
    match target {
        PurgeTarget::All => vec![
            NamedObjectKind::Block,
            NamedObjectKind::Style,
            NamedObjectKind::Linetype,
        ],
        PurgeTarget::Block => vec![NamedObjectKind::Block],
        PurgeTarget::Style => vec![NamedObjectKind::Style],
        PurgeTarget::Linetype => vec![NamedObjectKind::Linetype],
        PurgeTarget::Layer => Vec::new(),
    }
}

pub(crate) fn with_payload(command: &Command, id: CommandId, payload: CommandPayload) -> Command {
    Command {
        schema_version: command.schema_version,
        id,
        document: command.document,
        viewport: command.viewport,
        payload,
    }
}

pub(crate) fn document_drawing(
    app: &Application,
    command: &Command,
) -> CadResult<Arc<cad_db::DrawingDatabase>> {
    let document = app
        .workspace
        .documents
        .get(&command.document)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    if document.id != command.document {
        return Err(CadError::StaleResult);
    }
    Ok(document.drawing.clone())
}

pub(crate) fn require_none_payload(command: &Command, name: &str) -> CadResult<()> {
    if matches!(command.payload, CommandPayload::None) {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!("{name} needs no payload")))
    }
}

fn space_rank(space: &SpaceId) -> u8 {
    match space {
        SpaceId::Model => 0,
        SpaceId::Paper(_) => 1,
        SpaceId::Block(_) => 2,
    }
}

fn space_label(space: &SpaceId) -> String {
    match space {
        SpaceId::Model => "Model".to_string(),
        SpaceId::Paper(layout) => format!("Paper({})", layout.0),
        SpaceId::Block(block) => format!("Block({})", block.0),
    }
}

fn layer_label(database: &cad_db::DrawingDatabase, layer: LayerId) -> String {
    match database.layer(layer) {
        Some(row) => row.name.clone(),
        None => format!("unresolved({})", layer.0),
    }
}

/// Format Unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (UTC), without a calendar
/// dependency. Uses Howard Hinnant's civil-from-days algorithm.
pub(crate) fn format_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = rem / 3_600;
    let minute = (rem % 3_600) / 60;
    let second = rem % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}
