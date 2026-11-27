//! `SECTIONPLANE [ON|OFF|TOGGLE|CLEAR|LIST|SET ...]`: the document section plane.
//!
//! The plane is document state (`tables/sections.json`, minor 32) and the
//! model-space scene build clips against it. Every write is one revision and one
//! undo step; `LIST` is read-only and Viewer-legal. A bare selector toggles the
//! stored plane, so the View > Section Planes menu item has a real action without
//! a second command id.

use super::*;
use crate::SectionCommand;
use cad_domain::section::{markers, SectionPlane};

impl Application {
    /// Apply one typed `SECTIONPLANE` line (or the menu's bare toggle).
    pub(crate) fn section_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let option = match &command.payload {
            CommandPayload::Section(option) => *option,
            // A bare menu dispatch may arrive without a typed payload.
            _ => SectionCommand::Toggle,
        };
        if session.app_mode() == AppMode::Viewer && !matches!(option, SectionCommand::List) {
            return Err(CadError::PermissionDenied);
        }
        let current = self.drawing(command.document)?.section_plane();
        let next = match option {
            SectionCommand::List => {
                let message = match current {
                    Some(plane) => format!("SECTIONPLANE LIST: {plane}"),
                    None => "SECTIONPLANE LIST: no section plane is stored".to_string(),
                };
                return Ok(section_outcome("section.list", message));
            }
            SectionCommand::Set { origin, normal } => {
                Some(SectionPlane::new(origin, normal)?.with_active(true))
            }
            SectionCommand::On => Some(require_plane(current)?.with_active(true)),
            SectionCommand::Off => Some(require_plane(current)?.with_active(false)),
            SectionCommand::Toggle => {
                let plane = require_plane(current)?;
                Some(plane.with_active(!plane.active))
            }
            SectionCommand::Clear => None,
        };
        if next == current {
            return Ok(section_outcome(
                "section.unchanged",
                format!(
                    "SECTIONPLANE: {}; nothing was written",
                    match current {
                        Some(plane) => plane.to_string(),
                        None => "no plane is stored".to_string(),
                    }
                ),
            ));
        }
        self.write_section(command.document, current, next, "SECTIONPLANE")?;
        Ok(section_outcome(
            "section.saved",
            format!(
                "SECTIONPLANE: {}",
                match next {
                    Some(plane) => plane.to_string(),
                    None => "cleared".to_string(),
                }
            ),
        ))
    }

    /// Commit one whole-slot section write and record it as one undo step.
    pub(crate) fn write_section(
        &mut self,
        document: DocumentId,
        before: Option<SectionPlane>,
        after: Option<SectionPlane>,
        label: &str,
    ) -> CadResult<()> {
        let transaction = crate::app_drawing::next_transaction_id();
        {
            let document = self
                .workspace
                .documents
                .get_mut(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            std::sync::Arc::make_mut(&mut document.drawing).apply_section_plane(
                after,
                transaction,
                label,
            )?;
        }
        let patch = cad_db::SectionPatch { before, after };
        let record =
            cad_history::DrawingUndoRecord::entity_only(transaction, label.to_string(), Vec::new());
        if let Err(error) = self
            .history
            .entry(document)
            .or_default()
            .record_drawing_with_sections(record, vec![patch.clone()])
        {
            let document = self
                .workspace
                .documents
                .get_mut(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            std::sync::Arc::make_mut(&mut document.drawing).apply_section_patches(
                std::slice::from_ref(&patch.inverse()),
                crate::app_drawing::next_transaction_id(),
                "SECTIONPLANE rollback",
            )?;
            return Err(error);
        }
        Ok(())
    }
}

fn require_plane(current: Option<SectionPlane>) -> CadResult<SectionPlane> {
    current.ok_or_else(|| {
        CadError::InvalidInput(format!(
            "{}: SECTIONPLANE ON|OFF|TOGGLE needs a stored plane; run SECTIONPLANE SET first",
            markers::SECTION_PLANE_MISSING
        ))
    })
}

fn section_outcome(code: &str, message: String) -> CommandOutcome {
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
