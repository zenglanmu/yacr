//! `SCENE SAVE|RESTORE|LIST|DELETE`: persisted scenes (SketchUp pages).
//!
//! A scene captures **presentation state**: the camera, 2D/3D mode, saved 2D
//! camera, work plane and active space (the same payload a named view stores)
//! plus the visual-style token this build implements (`WIREFRAME`/`SHADED`).
//! Saving and deleting write `tables/scenes.json` as one revision and one undo
//! step; restoring is a session apply (camera/space/style), exactly like
//! `VIEW RESTORE`, and records a `ZOOM PREVIOUS` snapshot so `VIEWBACK` undoes it.
//!
//! Refusals: `scene.name_required`, `scene.unknown`, `scene.invalid_view`,
//! `scene.style_invalid`, plus the database's cap/duplicate markers. `SAVE` and
//! `DELETE` refuse in Viewer mode (`PermissionDenied`); `RESTORE`/`LIST` are
//! Viewer-legal.

use super::*;
use cad_domain::scene::{Scene, SceneVisualStyle};
use cad_history::DrawingUndoRecord;

impl Application {
    /// Apply one typed `SCENE <option>` line.
    pub(crate) fn scene_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Scene(option) = &command.payload else {
            return Err(CadError::InvalidInput(
                "SCENE needs SAVE <name>, RESTORE <name>, LIST or DELETE <name>".into(),
            ));
        };
        // RESTORE/LIST are session view state; SAVE/DELETE write the document
        // table, so they refuse in Viewer instead of a hidden write.
        if session.app_mode() == AppMode::Viewer
            && matches!(option, SceneCommand::Save(_) | SceneCommand::Delete(_))
        {
            return Err(CadError::PermissionDenied);
        }
        match option {
            SceneCommand::Save(name) => {
                let name = checked_scene_name(name)?;
                let viewport = self
                    .workspace
                    .viewports
                    .get(&command.viewport)
                    .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
                let view = ViewSnapshot::capture(viewport);
                let named = crate::view_persist::named_view_from_snapshot(
                    name.clone(),
                    &view,
                    session.active_space.clone(),
                );
                let style = SceneVisualStyle::from_token(session.view_extras.visual_style.name())
                    .unwrap_or_default();
                let planned = self
                    .drawing(command.document)?
                    .plan_save_scene(Scene { view: named, style })?;
                let Some(planned) = planned else {
                    return Ok(scene_outcome(
                        "scene.unchanged",
                        format!("`{name}` already stores this exact scene; nothing was written"),
                    ));
                };
                let replaced = planned.replaced;
                self.write_scene(
                    command.document,
                    planned.patch,
                    &format!("SCENE SAVE {name}"),
                )?;
                let count = self.drawing(command.document)?.scenes().len();
                Ok(scene_outcome(
                    "scene.saved",
                    format!(
                        "SCENE SAVE: {} scene `{name}` with its camera, mode, work plane, space \
                         and style ({count} scene(s) in the drawing)",
                        if replaced { "replaced" } else { "created" }
                    ),
                ))
            }
            SceneCommand::Restore(name) => self.restore_scene(session, command, name),
            SceneCommand::List => {
                let names: Vec<String> = self
                    .drawing(command.document)?
                    .scenes()
                    .scenes()
                    .map(|scene| scene.name().to_string())
                    .collect();
                let message = if names.is_empty() {
                    "SCENE LIST: no scenes are saved in this drawing".to_string()
                } else {
                    format!("SCENE LIST: {} scene(s): {}", names.len(), names.join(", "))
                };
                Ok(scene_outcome("scene.list", message))
            }
            SceneCommand::Delete(name) => {
                let name = checked_scene_name(name)?;
                let planned = self.drawing(command.document)?.plan_delete_scene(&name)?;
                self.write_scene(
                    command.document,
                    planned.patch,
                    &format!("SCENE DELETE {name}"),
                )?;
                let count = self.drawing(command.document)?.scenes().len();
                Ok(scene_outcome(
                    "scene.deleted",
                    format!("SCENE DELETE: removed scene `{name}` ({count} scene(s) left)"),
                ))
            }
        }
    }

    /// Apply one scene's presentation state (camera/space/style).
    fn restore_scene(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        name: &str,
    ) -> CadResult<CommandOutcome> {
        let name = checked_scene_name(name)?;
        let Some(saved) = self
            .drawing(command.document)?
            .scenes()
            .resolve(&name)
            .cloned()
        else {
            return Err(unknown_scene(&name));
        };
        let snapshot = ViewSnapshot::from_named_view(&saved.view)?;
        crate::space::validate_space_id(self.drawing(command.document)?, &saved.view.space)?;
        let viewport = self
            .workspace
            .viewports
            .get(&command.viewport)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let changed =
            ViewSnapshot::capture(viewport) != snapshot || session.active_space != saved.view.space;
        if changed {
            self.push_view_snapshot(&command.viewport)?;
        }
        session.active_space = saved.view.space.clone();
        snapshot.restore(self.viewport_mut(command)?);
        session.view_extras.set_visual_style(saved.style.token())?;
        Ok(scene_outcome(
            "scene.restored",
            format!(
                "SCENE RESTORE: applied scene `{name}` (camera, mode, work plane, space and \
                 style {})",
                saved.style.token()
            ),
        ))
    }

    /// Commit one scene-table patch and record it as one undo step.
    fn write_scene(
        &mut self,
        document: DocumentId,
        patch: cad_db::ScenePatch,
        label: &str,
    ) -> CadResult<()> {
        let transaction = crate::app_drawing::next_transaction_id();
        let changes = {
            let document = self
                .workspace
                .documents
                .get_mut(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            Arc::make_mut(&mut document.drawing).apply_scene_patches(
                std::slice::from_ref(&patch),
                transaction,
                label,
            )?
        };
        debug_assert!(changes.scope.tables.contains(&cad_db::TableKind::Scenes));
        let record = DrawingUndoRecord::entity_only(transaction, label.to_string(), Vec::new());
        if let Err(error) = self
            .history
            .entry(document)
            .or_default()
            .record_drawing_with_scenes(record, vec![patch.clone()])
        {
            // Put the row back: a refused history record must not leave a table
            // edit the user cannot undo.
            let document = self
                .workspace
                .documents
                .get_mut(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            Arc::make_mut(&mut document.drawing).apply_scene_patches(
                std::slice::from_ref(&patch.inverse()),
                crate::app_drawing::next_transaction_id(),
                "SCENE rollback",
            )?;
            return Err(error);
        }
        Ok(())
    }
}

fn checked_scene_name(name: &str) -> CadResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CadError::InvalidInput(
            "scene.name_required: a scene needs a non-empty name".into(),
        ));
    }
    Ok(name.to_string())
}

fn unknown_scene(name: &str) -> CadError {
    CadError::InvalidInput(format!(
        "scene.unknown: there is no saved scene named `{name}` in this drawing"
    ))
}

fn scene_outcome(code: &str, message: String) -> CommandOutcome {
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
