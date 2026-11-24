//! `SYSVDLG`: report, read one modelled variable, or apply a validated set.
//!
//! AutoCAD's `SYSVDLG` dialog views, edits, saves and restores system-variable
//! settings. This build has no interactive dialog, but the representable
//! behaviour is real:
//!
//! * the bare command reports every modelled variable (the existing read);
//! * `SYSVDLG <name>` reports one modelled variable and refuses an unmodelled
//!   name **by name** (nothing is invented);
//! * `SYSVDLG <name> <value>` applies one value through the same setter the
//!   ordinary command uses (one transaction);
//! * `SYSVDLG RESTORE <file.json>` applies a saved set in **one** command
//!   transaction (the header values together in one revision; session values
//!   after the same pre-validation);
//! * `SYSVDLG SAVE <file.json>` lives in `cad-ui-core`, which owns the host
//!   file bridge; it reads this module's one reader and never writes a second
//!   copy of a value.
//!
//! Every value is validated **before any write**, and an unknown or duplicate
//! name refuses the whole apply. A value outside the closed set is never
//! skipped silently.

use super::*;
use crate::app_extended_sysvar::{
    modelled_system_variable, ModelledValue, MODELLED_SYSTEM_VARIABLES,
};
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::document_drawing;

impl Application {
    /// Dispatch one `SYSVDLG` form.
    pub(crate) fn system_variable_dialog(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let drawing = document_drawing(self, command)?;
        match &command.payload {
            CommandPayload::None => Ok(report_outcome(sysvdlg_report(&drawing, session))),
            CommandPayload::SysVarDialog(SysVarDialogCommand::Read(name)) => {
                let name = name.trim().to_ascii_uppercase();
                match modelled_system_variable(&drawing, session, &name) {
                    Some(ModelledValue::Value(value, kind)) => {
                        Ok(report_outcome(vec![diagnostic(
                            "sysvdlg.value",
                            format!("SYSVDLG {name} = {value} ({kind})"),
                        )]))
                    }
                    Some(ModelledValue::Absent(reason)) => Err(CadError::InvalidInput(format!(
                        "sysvdlg.not_stored: {name}: {reason}"
                    ))),
                    None => Err(unknown_variable(&name)),
                }
            }
            CommandPayload::SysVarDialog(SysVarDialogCommand::Apply(assignments)) => {
                self.sysvdlg_apply(session, command, &drawing, assignments)
            }
            _ => Err(CadError::InvalidInput(
                "sysvdlg.invalid_payload: SYSVDLG takes no payload (report), a variable name, a \
                 `<name> <value>` pair, or a RESTORE assignment list"
                    .into(),
            )),
        }
    }

    /// Apply a validated assignment list. All validation happens before the
    /// first mutation; the header values then go through one revision.
    fn sysvdlg_apply(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        drawing: &cad_db::DrawingDatabase,
        assignments: &[SysVarAssignment],
    ) -> CadResult<CommandOutcome> {
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        if assignments.is_empty() {
            return Err(CadError::InvalidInput(
                "sysvdlg.empty_apply: there is no variable to apply; nothing was written".into(),
            ));
        }
        let parsed = ParsedAssignments::parse(drawing, session, assignments)?;

        // Phase 1: build and apply the header patch. `apply_header_values`
        // validates again, but the parse above already refused everything a
        // user can get wrong, so a failure here is an invariant, not a
        // half-applied restore.
        let mut patch = cad_db::HeaderValuesPatch::default();
        if parsed.point_mode.is_some() || parsed.point_size.is_some() {
            patch.point_display = Some((
                parsed
                    .point_mode
                    .unwrap_or_else(|| drawing.point_display_mode()),
                parsed
                    .point_size
                    .unwrap_or_else(|| drawing.point_display_size()),
            ));
        }
        patch.linetype_scale = parsed.linetype_scale;
        patch.insertion_base = parsed.insertion_base;
        patch.limits_min = parsed.limits_min;
        patch.limits_max = parsed.limits_max;
        let changes = if patch.is_empty() {
            None
        } else {
            let transaction = crate::app_drawing::next_transaction_id();
            let mut candidate = drawing.clone();
            let change = candidate.apply_header_values(patch, transaction, "sysvdlg")?;
            let changed = change.before != change.after;
            if changed {
                self.workspace
                    .documents
                    .get_mut(&command.document)
                    .ok_or_else(|| {
                        CadError::Invariant("document disappeared while publishing".into())
                    })?
                    .drawing = Arc::new(candidate);
                Some(Arc::new(change))
            } else {
                None
            }
        };

        // Phase 2: session-only values. Every setter's validation was already
        // run by `ParsedAssignments::parse`, so these calls cannot fail for a
        // user-supplied value.
        if let Some(spacing) = parsed.snap_unit {
            session.set_snap_spacing(spacing)?;
        }
        if let Some(enabled) = parsed.grid {
            session.set_grid_enabled(enabled);
        }
        if let Some(enabled) = parsed.snap {
            session.set_snap_grid_enabled(enabled);
        }
        if let Some(enabled) = parsed.ortho {
            session.set_ortho_enabled(enabled);
        }
        if let Some(height) = parsed.text_size {
            session.set_tool_parameter(ToolParameter::TextHeight, ToolValue::Number(height))?;
        }
        if let Some(frames) = parsed.wipeout {
            session.drafting_extras.set_wipeout_frames(frames);
        }
        if let Some(index) = parsed.cvport {
            let SpaceId::Paper(layout) = &session.active_space else {
                return Err(CadError::InvalidInput(
                    "sysvdlg.cvport_requires_paper: CVPORT is a paper-space value; switch to a \
                     paper layout before applying it"
                        .into(),
                ));
            };
            session.set_viewport_marker(*layout, index - 1);
        }
        if let Some(elev) = parsed.elev {
            session.drafting_extras.set_elev(elev)?;
        }
        if let Some(enabled) = parsed.fs_mode {
            session.drafting_extras.set_fs_mode(enabled);
        }
        if let Some(enabled) = parsed.explode_mode {
            session.set_explode_mode(enabled);
        }

        let mut rows: Vec<Diagnostic> = assignments
            .iter()
            .map(|assignment| {
                diagnostic(
                    "sysvdlg.applied",
                    format!(
                        "SYSVDLG: {} = {}",
                        assignment.name.trim().to_ascii_uppercase(),
                        assignment.value.trim()
                    ),
                )
            })
            .collect();
        rows.push(diagnostic(
            "sysvdlg.summary",
            format!(
                "SYSVDLG: applied {} modelled variable(s){}; every name and value was validated \
                 before the first write",
                assignments.len(),
                if changes.is_some() {
                    " (header values in one revision)"
                } else {
                    " (session values only; no drawing revision)"
                }
            ),
        ));
        if changes.is_some() {
            rows.push(diagnostic(
                "sysvdlg.no_history",
                "SYSVDLG: the header values were published in one revision and one header-only \
                 ChangeSet; no undo step was recorded (cad-history has no header patch kind)"
                    .into(),
            ));
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes,
            diagnostics: rows,
            measurement: None,
            point_report: None,
        })
    }
}

/// The bare `SYSVDLG` report.
fn sysvdlg_report(drawing: &cad_db::DrawingDatabase, session: &SessionState) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::with_capacity(MODELLED_SYSTEM_VARIABLES.len() + 3);
    for name in MODELLED_SYSTEM_VARIABLES {
        let value = modelled_system_variable(drawing, session, name)
            .expect("MODELLED_SYSTEM_VARIABLES is exactly the reader's closed set");
        match value {
            ModelledValue::Value(value, kind) => diagnostics.push(diagnostic(
                "sysvdlg.value",
                format!("SYSVDLG {name} = {value} ({kind})"),
            )),
            ModelledValue::Absent(reason) => {
                diagnostics.push(diagnostic("sysvdlg.not_stored", reason))
            }
        }
    }
    diagnostics.push(diagnostic(
        "sysvdlg.summary",
        format!(
            "SYSVDLG: {} modelled system variable(s) reported with live values; variables \
             outside the modelled closed set are not tracked and were not invented",
            MODELLED_SYSTEM_VARIABLES.len()
        ),
    ));
    diagnostics.push(diagnostic(
        "sysvdlg.not_tracked",
        "SYSVDLG: every other AutoCAD system variable is not tracked by this build — there is \
         no per-variable table to enumerate, and the write-only shell settings \
         (NAVVCUBEDISPLAY, DISPLAYVIEWCUBEIN2D, DISPLAYVIEWCUBEIN3D) have no readable value"
            .into(),
    ));
    diagnostics.push(diagnostic(
        "sysvdlg.dialog_not_implemented",
        "SYSVDLG: there is no interactive dialog; the representable forms are `SYSVDLG <name>` \
         (read), `SYSVDLG <name> <value>` (edit), `SYSVDLG SAVE <file.json>` and `SYSVDLG \
         RESTORE <file.json>` (session settings file)"
            .into(),
    ));
    diagnostics
}

fn unknown_variable(name: &str) -> CadError {
    CadError::InvalidInput(format!(
        "sysvdlg.unknown_variable: SYSVDLG does not model '{name}'; the closed set is {}",
        MODELLED_SYSTEM_VARIABLES.join(", ")
    ))
}

/// The parsed, validated form of one apply list.
#[derive(Default)]
struct ParsedAssignments {
    point_mode: Option<i16>,
    point_size: Option<f64>,
    linetype_scale: Option<f64>,
    insertion_base: Option<Point3>,
    limits_min: Option<[f64; 2]>,
    limits_max: Option<[f64; 2]>,
    snap_unit: Option<f64>,
    grid: Option<bool>,
    snap: Option<bool>,
    ortho: Option<bool>,
    text_size: Option<f64>,
    wipeout: Option<WipeoutFrames>,
    cvport: Option<usize>,
    elev: Option<f64>,
    fs_mode: Option<bool>,
    explode_mode: Option<bool>,
}

impl ParsedAssignments {
    fn parse(
        drawing: &cad_db::DrawingDatabase,
        session: &SessionState,
        assignments: &[SysVarAssignment],
    ) -> CadResult<ParsedAssignments> {
        let mut parsed = ParsedAssignments::default();
        let mut seen: Vec<String> = Vec::with_capacity(assignments.len());
        for assignment in assignments {
            let name = assignment.name.trim().to_ascii_uppercase();
            if name.is_empty() {
                return Err(CadError::InvalidInput(
                    "sysvdlg.empty_name: an assignment needs a variable name".into(),
                ));
            }
            if !MODELLED_SYSTEM_VARIABLES.contains(&name.as_str()) {
                return Err(unknown_variable(&name));
            }
            if seen.contains(&name) {
                return Err(CadError::InvalidInput(format!(
                    "sysvdlg.duplicate_variable: '{name}' appears more than once; nothing was \
                     written"
                )));
            }
            seen.push(name.clone());
            let value = assignment.value.trim();
            match name.as_str() {
                "PDMODE" => {
                    parsed.point_mode =
                        Some(value.parse::<i16>().map_err(|_| {
                            invalid_value(&name, value, "one integer that fits i16")
                        })?)
                }
                "PDSIZE" => parsed.point_size = Some(finite(&name, value, "one finite decimal")?),
                "SNAPUNIT" => {
                    parsed.snap_unit = Some(positive(&name, value, "one positive finite decimal")?)
                }
                "LTSCALE" => {
                    parsed.linetype_scale =
                        Some(positive(&name, value, "one positive finite decimal")?)
                }
                "GRID" | "SNAP" | "ORTHO" => {
                    let enabled = parse_bool(value)
                        .ok_or_else(|| invalid_value(&name, value, "ON, OFF, 1 or 0"))?;
                    match name.as_str() {
                        "GRID" => parsed.grid = Some(enabled),
                        "SNAP" => parsed.snap = Some(enabled),
                        _ => parsed.ortho = Some(enabled),
                    }
                }
                "TEXTSIZE" => {
                    let height = positive(&name, value, "one positive finite text height")?;
                    ToolParameter::TextHeight
                        .validate(ToolValue::Number(height))
                        .map_err(|error| {
                            CadError::InvalidInput(format!(
                                "sysvdlg.invalid_value: {name}: {error}"
                            ))
                        })?;
                    parsed.text_size = Some(height);
                }
                "WIPEOUTFRAME" => {
                    let raw = value
                        .parse::<i64>()
                        .map_err(|_| invalid_value(&name, value, "0, 1 or 2"))?;
                    parsed.wipeout = Some(
                        WipeoutFrames::from_number(raw)
                            .ok_or_else(|| invalid_value(&name, value, "0, 1 or 2"))?,
                    );
                }
                "CVPORT" => {
                    let index = value
                        .parse::<usize>()
                        .map_err(|_| invalid_value(&name, value, "one positive viewport index"))?;
                    if index == 0 {
                        return Err(invalid_value(&name, value, "one positive viewport index"));
                    }
                    let SpaceId::Paper(layout) = &session.active_space else {
                        return Err(CadError::InvalidInput(
                            "sysvdlg.cvport_requires_paper: CVPORT is a paper-space value; the \
                             current space is model space, so nothing was written"
                                .into(),
                        ));
                    };
                    let viewports = drawing
                        .layout(*layout)
                        .map(|row| row.viewports.len())
                        .ok_or_else(|| {
                            CadError::InvalidInput(format!(
                                "sysvdlg.cvport_layout_missing: the active paper layout {} is not \
                                 in the drawing's layout table",
                                layout.0
                            ))
                        })?;
                    if index > viewports {
                        return Err(CadError::InvalidInput(format!(
                            "sysvdlg.cvport_out_of_range: layout {} has {viewports} viewport(s), \
                             so CVPORT {index} does not exist; nothing was written",
                            layout.0
                        )));
                    }
                    parsed.cvport = Some(index);
                }
                "INSBASE" => {
                    let [x, y, z] = parse_point(value, &name)?;
                    parsed.insertion_base = Some(Point3 { x, y, z });
                }
                "LIMMIN" => parsed.limits_min = Some(parse_corner(value, &name)?),
                "LIMMAX" => parsed.limits_max = Some(parse_corner(value, &name)?),
                "ELEV" => parsed.elev = Some(finite(&name, value, "one finite height")?),
                "FSMODE" => {
                    parsed.fs_mode = Some(
                        parse_bool(value)
                            .ok_or_else(|| invalid_value(&name, value, "ON, OFF, 1 or 0"))?,
                    )
                }
                // `EXPLMODE`/`EXPMODE` is the same session switch `EXPLODE MODE`
                // writes; a restore must not trip the closed-set guard.
                "EXPLMODE" | "EXPMODE" => {
                    parsed.explode_mode = Some(
                        parse_bool(value)
                            .ok_or_else(|| invalid_value(&name, value, "ON, OFF, 1 or 0"))?,
                    )
                }
                _ => unreachable!("the closed set was checked above"),
            }
        }
        // `ELEV`'s setter has the same finite rule; keep the validation here so
        // a bad value refuses before any header write.
        if let Some(elev) = parsed.elev {
            if !elev.is_finite() {
                return Err(invalid_value(
                    "ELEV",
                    &elev.to_string(),
                    "one finite height",
                ));
            }
        }
        Ok(parsed)
    }
}

fn invalid_value(name: &str, value: &str, expected: &str) -> CadError {
    CadError::InvalidInput(format!(
        "sysvdlg.invalid_value: {name} needs {expected}, got '{value}'; nothing was written"
    ))
}

fn finite(name: &str, value: &str, expected: &str) -> CadResult<f64> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| invalid_value(name, value, expected))?;
    if !parsed.is_finite() {
        return Err(invalid_value(name, value, expected));
    }
    Ok(parsed)
}

fn positive(name: &str, value: &str, expected: &str) -> CadResult<f64> {
    let parsed = finite(name, value, expected)?;
    if parsed <= 0.0 {
        return Err(invalid_value(name, value, expected));
    }
    Ok(parsed)
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_uppercase().as_str() {
        "ON" | "TRUE" | "1" => Some(true),
        "OFF" | "FALSE" | "0" => Some(false),
        _ => None,
    }
}

/// Parse `x,y[,z]` (also accepts whitespace-separated components).
fn parse_point(value: &str, name: &str) -> CadResult<[f64; 3]> {
    let parts: Vec<&str> = value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    if !(2..=3).contains(&parts.len()) {
        return Err(invalid_value(name, value, "2 or 3 finite components"));
    }
    let mut components = [0.0f64; 3];
    for (index, part) in parts.iter().enumerate() {
        let parsed = part
            .parse::<f64>()
            .map_err(|_| invalid_value(name, value, "2 or 3 finite components"))?;
        if !parsed.is_finite() {
            return Err(invalid_value(name, value, "2 or 3 finite components"));
        }
        components[index] = parsed;
    }
    Ok(components)
}

fn parse_corner(value: &str, name: &str) -> CadResult<[f64; 2]> {
    let [x, y, _] = parse_point(value, name)?;
    Ok([x, y])
}
