//! The two read-only **report** commands (`RESOURCES`, `DIAGNOSTICS`).
//!
//! Split out of `application.rs` (which is at the file-size ceiling) with the
//! bodies unchanged: both are inherent methods on [`Application`], so the
//! dispatch arms in `application::execute` are untouched.

use super::*;

impl Application {
    /// Report the resources the open document references (spec F10/F11).
    pub(crate) fn resources_report(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let keys = if document.resource_keys.is_empty() {
            "无外部资源引用".to_string()
        } else {
            document.resource_keys.join(", ")
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "resources.summary".into(),
                message: format!("资源引用 {}：{keys}", document.resource_keys.len()),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// Structured diagnostics summary for the UI panel and CLI (spec §19).
    pub(crate) fn diagnostics_report(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let bounds = document
            .drawing
            .bounds()
            .map(|(min, max)| format!("范围 {:.3},{:.3} → {:.3},{:.3}", min.x, min.y, max.x, max.y))
            .unwrap_or_else(|| "空图纸".to_string());
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "diagnostics.summary".into(),
                    message: format!(
                        "图元 {}，单位 {:?}",
                        document.drawing.entity_count(),
                        document.units.source
                    ),
                },
                Diagnostic {
                    object: None,
                    code: "diagnostics.bounds".into(),
                    message: bounds,
                },
            ],
            measurement: None,
            point_report: None,
        })
    }
}
