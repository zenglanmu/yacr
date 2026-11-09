//! The session-only half of the 2026-11-07 layout / viewport batch.
//!
//! `VPMAX` / `VPMIN` set and clear one session display marker naming the active
//! paper layout's **first** viewport (this build has no per-viewport selection).
//! They never write drawing data, bump a revision or record history, so they are
//! split out of [`crate::app_layout`] to keep that file inside the code-size gate.

use super::*;

impl Application {
    /// `VPMAX`: set the session-only maximized-viewport marker.
    pub(crate) fn vp_max(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "vpmax.invalid_payload: VPMAX takes no payload".into(),
            ));
        }
        let SpaceId::Paper(layout_id) = &session.active_space else {
            return Err(CadError::Unsupported(
                "vpmax.paper_space_required: VPMAX needs an active paper layout".into(),
            ));
        };
        let layout_id = *layout_id;
        let viewports = self
            .drawing(command.document)?
            .layout(layout_id)
            .ok_or_else(|| {
                CadError::InvalidInput(format!("vpmax.unknown_layout: layout {}", layout_id.0))
            })?
            .viewports
            .len();
        if viewports == 0 {
            return Err(CadError::Unsupported(format!(
                "vpmax.no_viewports: layout {} has no viewports to maximize",
                layout_id.0
            )));
        }
        let target = LayoutViewportRef {
            layout: layout_id,
            index: 0,
        };
        if session.maximized_viewport() == Some(target) {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "vpmax.already_maximized".into(),
                    message: format!(
                        "VPMAX: layout {} viewport {} is already maximized",
                        layout_id.0, target.index
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        session.set_maximized_viewport(target);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "vpmax.maximized".into(),
                message: format!(
                    "VPMAX: maximized layout {} viewport {} (session display state only; the \
                     first viewport is used because this build has no viewport selection)",
                    layout_id.0, target.index
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `VPMIN`: clear the session-only maximized-viewport marker.
    pub(crate) fn vp_min(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "vpmin.invalid_payload: VPMIN takes no payload".into(),
            ));
        }
        match session.maximized_viewport() {
            Some(viewport) => {
                session.clear_maximized_viewport();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "vpmin.restored".into(),
                        message: format!(
                            "VPMIN: restored the layout viewport (layout {} viewport {})",
                            viewport.layout.0, viewport.index
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            None => Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "vpmin.nothing_to_restore".into(),
                    message: "VPMIN: no viewport is maximized; nothing changed".into(),
                }],
                measurement: None,
                point_report: None,
            }),
        }
    }
}
