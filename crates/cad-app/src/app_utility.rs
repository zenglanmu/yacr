//! The 2026-11-07 utility/named-object batch: `PURGE`, `RENAME`, `SETVAR`,
//! `TIME`, `DBLIST`, `TREESTAT`, `MREDO` and `MULTIPLE`.
//!
//! # Honest boundaries
//!
//! * **PURGE** can only remove what the database exposes a delete API for. This
//!   build has `DrawingDatabase::delete_layer`; block definitions, linetypes and
//!   text styles have **no delete API**, so an explicit `PURGE BLOCK|STYLE|
//!   LINETYPE` is a named `Unsupported` refusal and the bare `PURGE` reports each
//!   unsupported kind as a diagnostic. Layer rows are removed through the
//!   existing `DELETELAYER` command path, which is also why a layer in use is
//!   refused (the database never re-points an entity's layer).
//! * **RENAME** has a database API for layers only (`rename_layer`). `RENAME
//!   LAYER` is routed to the existing `RENAMELAYER` command; the other kinds are
//!   explicit `Unsupported` refusals.
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
//! All layer-table writes are **not undoable** (`cad-history` stores entity
//! patches only); every report path writes nothing, bumps no revision and records
//! no history.

use super::*;

use cad_db::DEFAULT_LAYER_NAME;
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

    /// `PURGE` / `-PURGE`: remove unused named objects through the sanctioned
    /// delete paths.
    ///
    /// Layer rows go through [`Self::layer_command`] with the ordinary
    /// `DELETELAYER` payload, so the active-layer fallback and override cleanup
    /// are the existing ones. Each non-default layer is reported as removed or
    /// refused (an in-use layer is refused by the database). Block/linetype/style
    /// have no delete API: an explicit kind is a named `Unsupported`; the bare
    /// `PURGE` reports each unsupported kind and still purges what it can.
    fn purge(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let target = match &command.payload {
            CommandPayload::None => PurgeTarget::All,
            CommandPayload::Purge(target) => *target,
            _ => {
                return Err(CadError::InvalidInput(
                    "PURGE takes no payload or one named kind".into(),
                ))
            }
        };
        if let Some(kind) = unsupported_purge_kind(target) {
            return Err(CadError::Unsupported(format!(
                "purge.{}.unsupported: this build has no delete API for {}; nothing was removed",
                kind_key(kind),
                kind_label(kind)
            )));
        }

        let drawing = document_drawing(self, command)?;
        let names: Vec<String> = drawing
            .layers()
            .filter(|layer| layer.name != DEFAULT_LAYER_NAME)
            .map(|layer| layer.name.clone())
            .collect();

        let mut diagnostics = Vec::new();
        let mut last_changes: Option<Arc<ChangeSet>> = None;
        let mut removed = 0usize;
        for name in names {
            let inner = with_payload(
                command,
                CommandId::DeleteLayer,
                CommandPayload::DeleteLayer {
                    layer: name.clone(),
                },
            );
            match self.layer_command(session, &inner) {
                Ok(outcome) => {
                    removed += 1;
                    if outcome.changes.is_some() {
                        last_changes = outcome.changes;
                    }
                    diagnostics.extend(outcome.diagnostics);
                    diagnostics.push(Diagnostic {
                        object: None,
                        code: "purge.layer_removed".into(),
                        message: format!("PURGE: removed unused layer '{name}'"),
                    });
                }
                Err(error) => diagnostics.push(Diagnostic {
                    object: None,
                    code: "purge.layer_refused".into(),
                    message: format!("PURGE: layer '{name}' was not removed: {error}"),
                }),
            }
        }

        if target == PurgeTarget::All {
            for kind in [
                NamedObjectKind::Block,
                NamedObjectKind::Style,
                NamedObjectKind::Linetype,
            ] {
                diagnostics.push(Diagnostic {
                    object: None,
                    code: "purge.kind_unsupported".into(),
                    message: format!(
                        "PURGE: {} has no delete API in this build; unused {} entries were not \
                         removed",
                        kind_label(kind),
                        kind_label(kind)
                    ),
                });
            }
        }

        diagnostics.push(Diagnostic {
            object: None,
            code: if removed == 0 {
                "purge.none".into()
            } else {
                "purge.summary".into()
            },
            message: format!(
                "PURGE: {removed} unused layer row(s) removed; layer-table writes are not \
                 undoable"
            ),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: last_changes,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `RENAME` / `-RENAME`: rename one named object.
    ///
    /// `RENAME LAYER` reuses the ordinary `RENAMELAYER` command (same database
    /// write, same refusals for unknown, duplicate and illegal names). The other
    /// three kinds have no rename API and are explicit refusals.
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
            kind => Err(CadError::Unsupported(format!(
                "rename.{}.unsupported: this build has no rename API for {}; nothing changed",
                kind_key(kind),
                kind_label(kind)
            ))),
        }
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

    /// `SETVAR <name>`: report one modelled variable's current value.
    fn setvar_report(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::SetVar(name) = &command.payload else {
            return Err(CadError::InvalidInput(
                "setvar.needs_name: SETVAR needs a variable name, e.g. `SETVAR PDMODE`".into(),
            ));
        };
        let drawing = document_drawing(self, command)?;
        let name = name.trim().to_ascii_uppercase();
        let (value, kind) =
            modelled_system_variable(&drawing, session, &name).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "setvar.unknown_variable: SETVAR does not model '{name}'; the closed set is {}",
                    MODELLED_SYSTEM_VARIABLES.join(", ")
                ))
            })?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "setvar.value".into(),
                message: format!("SETVAR {name} = {value} ({kind})"),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `TIME`: read-only document/clock report.
    fn time_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "TIME")?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = vec![
            Diagnostic {
                object: None,
                code: "time.document".into(),
                message: format!(
                    "TIME: document {} revision {} entities {}",
                    command.document.0,
                    drawing.revision().0,
                    drawing.entity_count()
                ),
            },
            Diagnostic {
                object: None,
                code: "time.not_tracked".into(),
                message: "TIME: drawing creation/edit timestamps are not tracked by this database"
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

fn unsupported_purge_kind(target: PurgeTarget) -> Option<NamedObjectKind> {
    match target {
        PurgeTarget::Block => Some(NamedObjectKind::Block),
        PurgeTarget::Style => Some(NamedObjectKind::Style),
        PurgeTarget::Linetype => Some(NamedObjectKind::Linetype),
        PurgeTarget::All | PurgeTarget::Layer => None,
    }
}

fn kind_label(kind: NamedObjectKind) -> &'static str {
    match kind {
        NamedObjectKind::Layer => "LAYER",
        NamedObjectKind::Block => "BLOCK",
        NamedObjectKind::Style => "STYLE",
        NamedObjectKind::Linetype => "LINETYPE",
    }
}

fn kind_key(kind: NamedObjectKind) -> &'static str {
    match kind {
        NamedObjectKind::Layer => "layer",
        NamedObjectKind::Block => "block",
        NamedObjectKind::Style => "style",
        NamedObjectKind::Linetype => "linetype",
    }
}

fn with_payload(command: &Command, id: CommandId, payload: CommandPayload) -> Command {
    Command {
        schema_version: command.schema_version,
        id,
        document: command.document,
        viewport: command.viewport,
        payload,
    }
}

/// The `SETVAR` closed set: the system variables this build really models, in
/// report order. `SETVAR` and the `SYSVARMONITOR`/`SYSVDLG` reports read exactly
/// this list, so a report can never advertise a variable the router does not
/// model.
pub(crate) const MODELLED_SYSTEM_VARIABLES: [&str; 8] = [
    "PDMODE", "PDSIZE", "SNAPUNIT", "LTSCALE", "GRID", "SNAP", "ORTHO", "TEXTSIZE",
];

/// Read one modelled system variable as `(value, kind)`, or `None` when the name
/// is outside [`MODELLED_SYSTEM_VARIABLES`].
pub(crate) fn modelled_system_variable(
    drawing: &cad_db::DrawingDatabase,
    session: &SessionState,
    name: &str,
) -> Option<(String, &'static str)> {
    let value = match name {
        "PDMODE" => (drawing.point_display_mode().to_string(), "integer"),
        "PDSIZE" => (drawing.point_display_size().to_string(), "real"),
        "SNAPUNIT" => (session.snap_spacing().to_string(), "real"),
        "LTSCALE" => (drawing.linetype_scale().to_string(), "real"),
        "GRID" => (session.grid_enabled().to_string(), "boolean"),
        "SNAP" => (session.snap_grid_enabled().to_string(), "boolean"),
        "ORTHO" => (session.ortho_enabled().to_string(), "boolean"),
        "TEXTSIZE" => match session.tool_parameter(ToolParameter::TextHeight) {
            ToolValue::Number(height) => (height.to_string(), "real"),
            ToolValue::Flag(flag) => (flag.to_string(), "boolean"),
        },
        _ => return None,
    };
    Some(value)
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
