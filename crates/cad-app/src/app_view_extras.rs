//! The 2026-11-07 extras batch view-only commands: `VIEWRES` and `VSCURRENT`.
//!
//! Both mutate [`SessionState::view_extras`] only: no database write, revision
//! bump, `cad-history` entry, `ChangeSet` or `ZOOM PREVIOUS` snapshot. `-PAN` is
//! not implemented here: it rides the existing `CommandId::Pan` camera path with
//! a `Points` delta, so it has no second camera authority.
//!
//! `VSCURRENT` stores a token for a host/renderer to consume; this lane wires no
//! renderer change, so no pixel/GPU effect is claimed or verified.

use super::*;

fn view_outcome(code: &str, message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: code.into(),
            message,
        }],
        measurement: None,
        point_report: None,
    }
}

impl Application {
    /// `VIEWRES <1..=1000>`: store the session circle-tessellation percentage.
    ///
    /// The range is re-validated here even though the typed parser already
    /// enforces it: a direct IPC caller gets the same named refusal, never a
    /// clamp.
    pub(crate) fn viewres(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ViewRes(percent) = &command.payload else {
            return Err(CadError::InvalidInput(
                "VIEWRES needs one integer percentage in 1..=1000".into(),
            ));
        };
        let percent = *percent;
        session.view_extras.set_viewres(percent)?;
        Ok(view_outcome(
            "viewres",
            format!(
                "VIEWRES: circle tessellation set to {percent}% (session state only; the drawing \
                 was not written)"
            ),
        ))
    }

    /// `VSCURRENT WIREFRAME`: store the session visual-style token.
    ///
    /// Every non-wireframe style is refused by name
    /// (`vscurrent.style_not_implemented`), never silently mapped.
    pub(crate) fn vscurrent(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::VisualStyle(style) = &command.payload else {
            return Err(CadError::InvalidInput(
                "VSCURRENT needs one style name (WIREFRAME)".into(),
            ));
        };
        session.view_extras.set_visual_style(style)?;
        Ok(view_outcome(
            "vscurrent",
            format!(
                "VSCURRENT: visual style set to {} (session token only; no renderer change is \
                 wired in this build and no pixel effect is verified)",
                session.view_extras.visual_style.name()
            ),
        ))
    }
}
