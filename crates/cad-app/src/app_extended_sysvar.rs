//! The modelled system-variable table, plus the session-only `SYSVARMONITOR`
//! watch set.
//!
//! # One closed set, one reader
//!
//! [`MODELLED_SYSTEM_VARIABLES`] is the **only** list of variables this build
//! models. `SETVAR` (report + write), `SYSVDLG` and `SYSVARMONITOR` all read it,
//! so no report can advertise a name the router cannot handle and no name can be
//! watched that `SETVAR` cannot read.
//!
//! Membership is not the same question as having a value. `INSBASE`, `LIMMIN`,
//! `LIMMAX` and `CVPORT` are modelled but a particular drawing/session may hold
//! no value for them (the importer does not read the three header points, and
//! `CVPORT` has no value outside a paper layout with a viewport). Those cases are
//! [`ModelledValue::Absent`] with a stable reason — never a fabricated `0`, `0,0`
//! or `1`.
//!
//! # Where each value lives (one owner per variable)
//!
//! | variable | owner | write path |
//! |---|---|---|
//! | `PDMODE`, `PDSIZE` | drawing header (`cad-db`) | `PDMODE`/`PDSIZE` commands |
//! | `LTSCALE` | drawing header (`cad-db`) | `LTSCALE` command |
//! | `INSBASE` | drawing header (`cad-db`) | `BASE` command |
//! | `LIMMIN`, `LIMMAX` | drawing header (`cad-db`) | `LIMITS` command |
//! | `CVPORT` | session (`SessionState`) | `CVPORT` command — **nothing is written to the drawing** |
//! | `ELEV` | session (`SessionState::drafting_extras`) | `ELEV` command — **nothing is written to the drawing** |
//! | `FSMODE` | session (`SessionState::drafting_extras`) | `FSMODE` command — **nothing is written to the drawing** |
//! | `GRID`, `SNAP`, `ORTHO` | session flags | the `GRID`/`SNAP`/`ORTHO` toggles |
//! | `TEXTSIZE` | session tool parameter | the shared `set_tool_parameter` |
//! | `WIPEOUTFRAME` | session display token | the `WIPEOUTFRAME` command |
//!
//! Every one of them is readable **and** writable through `SETVAR`, and the write
//! reuses the ordinary command so there is exactly one setter per value.
//!
//! # `SYSVARMONITOR`
//!
//! `SYSVARMONITOR ADD <name>` starts watching one modelled variable and records
//! its current value as the snapshot. `REMOVE <name>` stops watching it.
//! `LIST [<name>]` (and the bare selector) reads the live value through the same
//! reader and reports `sysvarmonitor.changed` when the value differs from the
//! snapshot, then updates the snapshot. There is **no background notification**: a
//! change is discovered only when the monitor is queried, and nothing is invented
//! between queries.
//!
//! The monitor is session state only: no transaction, no revision bump, no
//! `cad-history` entry, no `ChangeSet` and no persisted preference.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::document_drawing;

/// The `SETVAR` closed set: the system variables this build really models, in
/// report order. `SETVAR`, `SYSVDLG` and the `SYSVARMONITOR` watch set read
/// exactly this list, so a report can never advertise a variable the router does
/// not model.
pub(crate) const MODELLED_SYSTEM_VARIABLES: [&str; 15] = [
    "PDMODE",
    "PDSIZE",
    "SNAPUNIT",
    "LTSCALE",
    "GRID",
    "SNAP",
    "ORTHO",
    "TEXTSIZE",
    "WIPEOUTFRAME",
    // Wave 31: the four values whose storage already existed (or was added with
    // the same command) but that `SETVAR` did not list.
    "CVPORT",
    "INSBASE",
    "LIMMIN",
    "LIMMAX",
    // Wave 32: the two session tokens the create funnel and `FS` read. Both are
    // session-only (their writers write nothing to the drawing), and both are
    // reported here so `SETVAR`/`SYSVDLG`/`SYSVARMONITOR` cannot advertise a name
    // the router does not model.
    "ELEV",
    "FSMODE",
];

/// What a modelled variable reads as right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModelledValue {
    /// The current value and its kind word (`integer`, `real`, `boolean`,
    /// `point`).
    Value(String, &'static str),
    /// The name **is** modelled, but this drawing/session holds no value for it.
    /// The string is the stable, user-visible reason; nothing is fabricated.
    Absent(String),
}

/// Read one modelled system variable, or `None` when the name is outside
/// [`MODELLED_SYSTEM_VARIABLES`].
pub(crate) fn modelled_system_variable(
    drawing: &cad_db::DrawingDatabase,
    session: &SessionState,
    name: &str,
) -> Option<ModelledValue> {
    let value = match name {
        "PDMODE" => ModelledValue::Value(drawing.point_display_mode().to_string(), "integer"),
        "PDSIZE" => ModelledValue::Value(drawing.point_display_size().to_string(), "real"),
        "SNAPUNIT" => ModelledValue::Value(session.snap_spacing().to_string(), "real"),
        "LTSCALE" => ModelledValue::Value(drawing.linetype_scale().to_string(), "real"),
        "GRID" => ModelledValue::Value(session.grid_enabled().to_string(), "boolean"),
        "SNAP" => ModelledValue::Value(session.snap_grid_enabled().to_string(), "boolean"),
        "ORTHO" => ModelledValue::Value(session.ortho_enabled().to_string(), "boolean"),
        // `ELEV` is the session elevation the create funnel applies to every new
        // entity; `FSMODE` is the `FS` chain mode. Neither is drawing state: the
        // values live in `SessionState::drafting_extras`.
        "ELEV" => ModelledValue::Value(session.drafting_extras.elev().to_string(), "real"),
        "FSMODE" => ModelledValue::Value(session.drafting_extras.fs_mode().to_string(), "boolean"),
        "TEXTSIZE" => match session.tool_parameter(ToolParameter::TextHeight) {
            ToolValue::Number(height) => ModelledValue::Value(height.to_string(), "real"),
            ToolValue::Flag(flag) => ModelledValue::Value(flag.to_string(), "boolean"),
        },
        // `WIPEOUTFRAME` is session display state (`0`/`1`/`2`), not a drawing
        // header value: the report reads the same token the
        // `WIPEOUTFRAME`/`WIPEOUT Frames` commands write.
        "WIPEOUTFRAME" => ModelledValue::Value(
            session.drafting_extras.wipeout_frames().value().to_string(),
            "integer",
        ),
        // `CVPORT` is the session's current-viewport marker of the **active paper
        // layout**, reported one-based like the command line. Model space (and a
        // layout with no viewport) has no such value, so it is `Absent` with the
        // reason instead of a fabricated `1`.
        "CVPORT" => match &session.active_space {
            SpaceId::Paper(layout) => match drawing.layout(*layout) {
                Some(row) if !row.viewports.is_empty() => {
                    let marker = session.viewport_marker(*layout);
                    ModelledValue::Value((marker + 1).to_string(), "integer")
                }
                Some(row) => ModelledValue::Absent(format!(
                    "setvar.not_stored: CVPORT is modelled but layout {} ('{}') has no viewport, so \
                     there is no current viewport to report (create one with VPORTS or MVIEW); \
                     nothing was fabricated",
                    row.id.0, row.name
                )),
                None => ModelledValue::Absent(format!(
                    "setvar.not_stored: CVPORT is modelled but the session's active paper layout \
                     {} is not in the drawing's layout table, so there is no current viewport to \
                     report",
                    layout.0
                )),
            },
            _ => ModelledValue::Absent(
                "setvar.not_stored: CVPORT is modelled but it is a paper-space value; the current \
                 space is model space, which has no current viewport (switch with PSPACE first)"
                    .into(),
            ),
        },
        // The three vector header values are stored by `BASE`/`LIMITS`
        // (`cad_db::drawing::header_geometry`). `None` means this database holds
        // no value at all: the importer does not read these variables, so a
        // default would be indistinguishable from a real one.
        "INSBASE" => match drawing.insertion_base() {
            Some(base) => ModelledValue::Value(
                format!("{},{},{}", base.x, base.y, base.z),
                "point",
            ),
            None => ModelledValue::Absent(
                "setvar.not_stored: INSBASE ($INSBASE) is modelled but this drawing stores no \
                 insertion base point — the importer of this build does not read $INSBASE, so no \
                 origin value is reported as if it were the file's (set it with `BASE <x> <y> \
                 [<z>]`)"
                    .into(),
            ),
        },
        "LIMMIN" => match drawing.limits_min() {
            Some([x, y]) => ModelledValue::Value(format!("{x},{y}"), "point"),
            None => ModelledValue::Absent(
                "setvar.not_stored: LIMMIN ($LIMMIN) is modelled but this drawing stores no minimum \
                 limit corner — the importer of this build does not read it and no command has \
                 written it (set both corners with `LIMITS <min_x> <min_y> <max_x> <max_y>`)"
                    .into(),
            ),
        },
        "LIMMAX" => match drawing.limits_max() {
            Some([x, y]) => ModelledValue::Value(format!("{x},{y}"), "point"),
            None => ModelledValue::Absent(
                "setvar.not_stored: LIMMAX ($LIMMAX) is modelled but this drawing stores no maximum \
                 limit corner — the importer of this build does not read it and no command has \
                 written it (set both corners with `LIMITS <min_x> <min_y> <max_x> <max_y>`)"
                    .into(),
            ),
        },
        _ => return None,
    };
    Some(value)
}

impl Application {
    /// `SETVAR <name>`: report one modelled variable's current value.
    ///
    /// A name outside the closed set is the stable `setvar.unknown_variable`
    /// refusal (unchanged); a modelled name with no current value is
    /// `setvar.not_stored` with the reason. Nothing is ever printed as a default.
    pub(crate) fn setvar_report(
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
        let value = modelled_system_variable(&drawing, session, &name).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "setvar.unknown_variable: SETVAR does not model '{name}'; the closed set is {}",
                MODELLED_SYSTEM_VARIABLES.join(", ")
            ))
        })?;
        let (value, kind) = match value {
            ModelledValue::Value(value, kind) => (value, kind),
            ModelledValue::Absent(reason) => return Err(CadError::InvalidInput(reason)),
        };
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

/// Read one modelled value, refusing an unmodelled name by name and a modelled
/// name with no current value by its own reason.
fn current_value(
    drawing: &cad_db::DrawingDatabase,
    session: &SessionState,
    name: &str,
) -> CadResult<(String, &'static str)> {
    match modelled_system_variable(drawing, session, name) {
        Some(ModelledValue::Value(value, kind)) => Ok((value, kind)),
        Some(ModelledValue::Absent(reason)) => Err(CadError::InvalidInput(reason)),
        None => Err(CadError::InvalidInput(format!(
            "sysvarmonitor.unknown_variable: SYSVARMONITOR does not model '{name}'; the closed set \
             is {}",
            MODELLED_SYSTEM_VARIABLES.join(", ")
        ))),
    }
}
