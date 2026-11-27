//! `SHADOWS`/`FOG`: SketchUp's display settings on the open document.
//!
//! One per-document record (`tables/display.json`, `.ymodel` minor 22) carries
//! both panels:
//!
//! * **Shadows** is the sun used for **shading** — on/off plus its azimuth and
//!   elevation. The renderer replaces its camera-relative key light with the
//!   resulting direction. Cast shadows (a shadow map, softness, ground-plane
//!   falloff) are **NOT IMPLEMENTED**; the panel points at
//!   `docs/concept-sketchup.md` for that boundary.
//! * **Fog** is distance fog: geometry blends toward the fog colour between
//!   `start` and `end` model units from the camera.
//!
//! A write replaces the whole record as one revision and one undo step (the
//! whole before/after record travels on the history patch). A bare selector is a
//! report and writes nothing; a write that would not change the record answers
//! `display.unchanged` and writes nothing either.
//!
//! Refusals: the domain's `display.invalid_value` (an angle or fog band outside
//! its range), `display.no_change` (a caller applied the same record directly)
//! plus the patch/staleness markers of the replay path. Both selectors are
//! Work-only: they write the document.

use super::*;
use cad_db::DisplayPatch;
use cad_domain::display::{FogSettings, ShadowSettings};
use cad_history::DrawingUndoRecord;

impl Application {
    /// Apply one typed `SHADOWS`/`FOG` line.
    pub(crate) fn display_command(
        &mut self,
        _session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        // A bare selector is the report form: the name-only funnel reaches this
        // arm with `CommandPayload::None` and writes nothing.
        let option = match (&command.payload, command.id) {
            (CommandPayload::Display(option), _) => option.clone(),
            (CommandPayload::None, CommandId::Shadows) => DisplayCommand::ReportShadows,
            (CommandPayload::None, CommandId::Fog) => DisplayCommand::ReportFog,
            _ => {
                return Err(CadError::InvalidInput(
                    "SHADOWS takes ON|OFF|SUN <azimuth> <elevation>; FOG takes \
                     ON|OFF|RANGE <start> <end>|COLOR <r> <g> <b>"
                        .into(),
                ))
            }
        };
        let current = self.drawing(command.document)?.display_settings();
        let (next, label) = match &option {
            DisplayCommand::ReportShadows => {
                return Ok(display_outcome("display.shadows", report_shadows(&current)))
            }
            DisplayCommand::ReportFog => {
                return Ok(display_outcome("display.fog", report_fog(&current)))
            }
            DisplayCommand::Shadows(enabled) => (
                cad_domain::display::DisplaySettings {
                    shadows: ShadowSettings {
                        enabled: *enabled,
                        ..current.shadows
                    },
                    ..current
                },
                format!("SHADOWS {}", if *enabled { "ON" } else { "OFF" }),
            ),
            DisplayCommand::Sun { azimuth, elevation } => (
                cad_domain::display::DisplaySettings {
                    shadows: ShadowSettings {
                        enabled: current.shadows.enabled,
                        azimuth: *azimuth,
                        elevation: *elevation,
                    },
                    ..current
                },
                "SHADOWS SUN".to_string(),
            ),
            DisplayCommand::Fog(enabled) => (
                cad_domain::display::DisplaySettings {
                    fog: FogSettings {
                        enabled: *enabled,
                        ..current.fog
                    },
                    ..current
                },
                format!("FOG {}", if *enabled { "ON" } else { "OFF" }),
            ),
            DisplayCommand::FogRange { start, end } => (
                cad_domain::display::DisplaySettings {
                    fog: FogSettings {
                        enabled: current.fog.enabled,
                        start: *start,
                        end: *end,
                        color: current.fog.color,
                    },
                    ..current
                },
                "FOG RANGE".to_string(),
            ),
            DisplayCommand::FogColor(color) => (
                cad_domain::display::DisplaySettings {
                    fog: FogSettings {
                        enabled: current.fog.enabled,
                        start: current.fog.start,
                        end: current.fog.end,
                        color: *color,
                    },
                    ..current
                },
                "FOG COLOR".to_string(),
            ),
        };
        // The domain owns the ranges; a direct payload caller gets the same
        // stable marker as the typed line.
        next.validate()?;
        if next == current {
            return Ok(display_outcome(
                "display.unchanged",
                format!(
                    "{label}: the display settings already hold this value; nothing was written"
                ),
            ));
        }
        self.write_display(
            command.document,
            DisplayPatch {
                before: current,
                after: next,
            },
            &label,
        )?;
        Ok(display_outcome(
            "display.saved",
            format!("{label}: {}", describe(&next)),
        ))
    }

    /// Commit one whole-record display patch and record it as one undo step.
    pub(crate) fn write_display(
        &mut self,
        document: DocumentId,
        patch: DisplayPatch,
        label: &str,
    ) -> CadResult<()> {
        let transaction = crate::app_drawing::next_transaction_id();
        let changes = {
            let document = self
                .workspace
                .documents
                .get_mut(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            Arc::make_mut(&mut document.drawing).apply_display_settings(
                patch.after,
                transaction,
                label,
            )?
        };
        debug_assert!(changes.scope.tables.contains(&cad_db::TableKind::Display));
        let record = DrawingUndoRecord::entity_only(transaction, label.to_string(), Vec::new());
        if let Err(error) = self
            .history
            .entry(document)
            .or_default()
            .record_drawing_with_display(record, vec![patch.clone()])
        {
            // Put the record back: a refused history record must not leave a
            // write the user cannot undo.
            let document = self
                .workspace
                .documents
                .get_mut(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            Arc::make_mut(&mut document.drawing).apply_display_patches(
                std::slice::from_ref(&patch.inverse()),
                crate::app_drawing::next_transaction_id(),
                "display rollback",
            )?;
            return Err(error);
        }
        Ok(())
    }
}

/// One human-readable summary of the record (the report and write messages).
fn describe(settings: &cad_domain::display::DisplaySettings) -> String {
    format!("{}, {}", describe_shadows(settings), describe_fog(settings))
}

fn describe_shadows(settings: &cad_domain::display::DisplaySettings) -> String {
    let shadows = settings.shadows;
    if shadows.enabled {
        format!(
            "shadows on (sun azimuth {:.1}°, elevation {:.1}°)",
            shadows.azimuth, shadows.elevation
        )
    } else {
        format!(
            "shadows off (sun azimuth {:.1}°, elevation {:.1}°)",
            shadows.azimuth, shadows.elevation
        )
    }
}

fn describe_fog(settings: &cad_domain::display::DisplaySettings) -> String {
    let fog = settings.fog;
    let [r, g, b] = fog.color;
    if fog.enabled {
        format!(
            "fog on ({:.3}..{:.3} model units, colour {r},{g},{b})",
            fog.start, fog.end
        )
    } else {
        format!(
            "fog off ({:.3}..{:.3} model units, colour {r},{g},{b})",
            fog.start, fog.end
        )
    }
}

fn report_shadows(settings: &cad_domain::display::DisplaySettings) -> String {
    format!(
        "SHADOWS: {} (cast shadows are NOT IMPLEMENTED; this is the shading sun)",
        describe_shadows(settings)
    )
}

fn report_fog(settings: &cad_domain::display::DisplaySettings) -> String {
    format!("FOG: {}", describe_fog(settings))
}

fn display_outcome(code: &str, message: String) -> CommandOutcome {
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
