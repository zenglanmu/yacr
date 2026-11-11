//! The session-only `SYSVARMONITOR` watch set.
//!
//! `SYSVARMONITOR ADD <name>` starts watching one modelled system variable and
//! records its current value as the snapshot. `REMOVE <name>` stops watching it.
//! `LIST [<name>]` (and the bare selector) reads the live value through the same
//! `SETVAR` reader and reports `sysvarmonitor.changed` when the value differs
//! from the snapshot, then updates the snapshot. There is **no background
//! notification**: a change is discovered only when the monitor is queried, and
//! nothing is invented between queries.
//!
//! The monitor is session state only: no transaction, no revision bump, no
//! `cad-history` entry, no `ChangeSet` and no persisted preference. The watch set
//! is bounded by the `SETVAR` closed set; an unknown name is refused by name.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, modelled_system_variable, MODELLED_SYSTEM_VARIABLES};

impl Application {
    /// Dispatch one `SYSVARMONITOR` action.
    pub(crate) fn sysvar_monitor(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let action =
            match &command.payload {
                CommandPayload::None => SysVarMonitorAction::List(None),
                CommandPayload::SysVarMonitor(action) => action.clone(),
                _ => return Err(CadError::InvalidInput(
                    "sysvarmonitor.invalid_payload: SYSVARMONITOR takes ADD|REMOVE|LIST <name> or \
                     no payload"
                        .into(),
                )),
            };
        let drawing = document_drawing(self, command)?;
        match action {
            SysVarMonitorAction::Add(name) => {
                let name = watched_name(&name)?;
                let (value, kind) = current_value(&drawing, session, &name)?;
                let added = session.sysvar_monitor.add(name.clone(), value.clone());
                let (code, message) = if added {
                    (
                        "sysvarmonitor.added",
                        format!(
                            "SYSVARMONITOR: monitoring {name} = {value} ({kind}); the monitor is \
                             session state only and nothing was written to the drawing"
                        ),
                    )
                } else {
                    (
                        "sysvarmonitor.already_monitored",
                        format!(
                            "SYSVARMONITOR: {name} is already monitored and its earlier snapshot \
                             was kept; LIST reports a change against that snapshot"
                        ),
                    )
                };
                Ok(report_outcome(vec![diagnostic(code, message)]))
            }
            SysVarMonitorAction::Remove(name) => {
                let name = watched_name(&name)?;
                let _ = current_value(&drawing, session, &name)?;
                let removed = session.sysvar_monitor.remove(&name);
                Ok(report_outcome(vec![diagnostic(
                    if removed {
                        "sysvarmonitor.removed"
                    } else {
                        "sysvarmonitor.not_monitored"
                    },
                    if removed {
                        format!(
                            "SYSVARMONITOR: {name} is no longer monitored; the drawing and the \
                             session value were not changed"
                        )
                    } else {
                        format!("SYSVARMONITOR: {name} was not monitored; nothing changed")
                    },
                )]))
            }
            SysVarMonitorAction::List(Some(name)) => {
                let name = watched_name(&name)?;
                let (value, kind) = current_value(&drawing, session, &name)?;
                if !session.sysvar_monitor.contains(&name) {
                    return Ok(report_outcome(vec![diagnostic(
                        "sysvarmonitor.not_monitored",
                        format!(
                            "SYSVARMONITOR LIST: {name} is not monitored; use `SYSVARMONITOR ADD \
                             {name}` to start watching it"
                        ),
                    )]));
                }
                let snapshot = session
                    .sysvar_monitor
                    .snapshot(&name)
                    .map(str::to_string)
                    .unwrap_or_default();
                let (code, message) = monitor_row(&name, &value, kind, &snapshot);
                session.sysvar_monitor.record(&name, value);
                Ok(report_outcome(vec![
                    diagnostic(code, message),
                    summary(1, "SYSVARMONITOR LIST"),
                ]))
            }
            SysVarMonitorAction::List(None) => {
                let names: Vec<String> =
                    session.sysvar_monitor.names().map(str::to_string).collect();
                if names.is_empty() {
                    return Ok(report_outcome(vec![
                        diagnostic(
                            "sysvarmonitor.no_variables",
                            format!(
                                "SYSVARMONITOR: no variables are monitored; use `SYSVARMONITOR ADD \
                                 <name>` with one of {}",
                                MODELLED_SYSTEM_VARIABLES.join(", ")
                            ),
                        ),
                        summary(0, "SYSVARMONITOR"),
                    ]));
                }
                let mut diagnostics = Vec::with_capacity(names.len() + 1);
                let mut changes = 0usize;
                for name in names {
                    let (value, kind) = current_value(&drawing, session, &name)?;
                    let snapshot = session
                        .sysvar_monitor
                        .snapshot(&name)
                        .map(str::to_string)
                        .unwrap_or_default();
                    let (code, message) = monitor_row(&name, &value, kind, &snapshot);
                    if code == "sysvarmonitor.changed" {
                        changes += 1;
                    }
                    diagnostics.push(diagnostic(code, message));
                    session.sysvar_monitor.record(&name, value);
                }
                diagnostics.push(summary(
                    changes,
                    &format!(
                        "SYSVARMONITOR: {} variable(s) monitored",
                        session.sysvar_monitor.len()
                    ),
                ));
                Ok(report_outcome(diagnostics))
            }
        }
    }
}

/// One monitor row: a change against the snapshot, or the current value.
fn monitor_row(name: &str, value: &str, kind: &str, snapshot: &str) -> (&'static str, String) {
    if snapshot == value {
        (
            "sysvarmonitor.value",
            format!("SYSVARMONITOR LIST: {name} = {value} ({kind}); unchanged since the snapshot"),
        )
    } else {
        (
            "sysvarmonitor.changed",
            format!(
                "SYSVARMONITOR: {name} changed from {snapshot} to {value} ({kind}); the snapshot \
                 was updated"
            ),
        )
    }
}

fn summary(changes: usize, prefix: &str) -> Diagnostic {
    diagnostic(
        "sysvarmonitor.summary",
        format!(
            "{prefix}: {changes} change(s) reported; detection runs only when the monitor is \
             queried, so no background notification is claimed"
        ),
    )
}

/// Normalize and validate the name shape (the modelled closed set is checked by
/// [`current_value`], which carries its own stable refusal).
fn watched_name(name: &str) -> CadResult<String> {
    let name = name.trim().to_ascii_uppercase();
    if name.is_empty() {
        return Err(CadError::InvalidInput(
            "sysvarmonitor.invalid_name: SYSVARMONITOR needs a variable name".into(),
        ));
    }
    Ok(name)
}

/// Read one modelled value, refusing an unmodelled name by name.
fn current_value(
    drawing: &cad_db::DrawingDatabase,
    session: &SessionState,
    name: &str,
) -> CadResult<(String, &'static str)> {
    modelled_system_variable(drawing, session, name).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "sysvarmonitor.unknown_variable: SYSVARMONITOR does not model '{name}'; the closed set \
             is {}",
            MODELLED_SYSTEM_VARIABLES.join(", ")
        ))
    })
}
