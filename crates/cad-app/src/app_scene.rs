//! `SCENE SAVE|RESTORE|LIST|DELETE`: persisted scenes (SketchUp pages).
//!
//! A scene captures **presentation state**: the camera, 2D/3D mode, saved 2D
//! camera, work plane and active space (the same payload a named view stores),
//! the visual-style token this build implements (`WIREFRAME`/`SHADED`) and the
//! **visible tags/layers** at save time. Saving and deleting write
//! `tables/scenes.json` as one revision and one undo step; restoring is a session
//! apply (camera/space/style/tag visibility), exactly like `VIEW RESTORE`, and
//! records a `ZOOM PREVIOUS` snapshot so `VIEWBACK` undoes the view part. Tag
//! visibility lives in the same session-only override set as `LAYEROFF`, so a
//! restore never rewrites the layer table.
//!
//! Refusals: `scene.name_required`, `scene.unknown`, `scene.invalid_view`,
//! `scene.style_invalid`, plus the database's cap/duplicate markers. `SAVE` and
//! `DELETE` refuse in Viewer mode (`PermissionDenied`); `RESTORE`/`LIST` are
//! Viewer-legal.

use super::*;
use cad_domain::scene::{Scene, SceneTag, SceneVisualStyle};
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
                let tags = capture_visible_tags(self.drawing(command.document)?, session);
                let planned = self.drawing(command.document)?.plan_save_scene(Scene {
                    view: named,
                    style,
                    tags: Some(tags),
                })?;
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
                        "SCENE SAVE: {} scene `{name}` with its camera, mode, work plane, space, \
                         style and {} visible tag(s) ({count} scene(s) in the drawing)",
                        if replaced { "replaced" } else { "created" },
                        capture_visible_tags(self.drawing(command.document)?, session).len()
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
        let tag_count = saved.tags.as_ref().map_or(0, Vec::len);
        if let Some(tags) = &saved.tags {
            apply_visible_tags(self.drawing(command.document)?, session, tags);
        }
        Ok(scene_outcome(
            "scene.restored",
            format!(
                "SCENE RESTORE: applied scene `{name}` (camera, mode, work plane, space, \
                 style {} and {tag_count} tag(s))",
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

/// Capture every layer's effective visibility (database flag plus the session
/// override) as a scene's tag snapshot, in layer-table order.
fn capture_visible_tags(
    drawing: &cad_db::DrawingDatabase,
    session: &SessionState,
) -> Vec<SceneTag> {
    drawing
        .layers()
        .map(|layer| SceneTag {
            name: layer.name.clone(),
            visible: session.layer_overrides.effective(layer.id, layer.visible),
        })
        .collect()
}

/// Apply a saved tag snapshot to the session's layer overrides.
///
/// A tag whose name no longer resolves is skipped (renamed/deleted layer), and a
/// tag matching the database's own flag drops the override instead of pinning a
/// second copy of the same value. The layer table is never written.
fn apply_visible_tags(
    drawing: &cad_db::DrawingDatabase,
    session: &mut SessionState,
    tags: &[SceneTag],
) {
    for tag in tags {
        let Some(layer) = resolve_layer_by_name(drawing, &tag.name) else {
            continue;
        };
        if tag.visible == layer.visible {
            session.layer_overrides.remove(layer.id);
        } else {
            session.layer_overrides.set(layer.id, tag.visible);
        }
    }
}

/// Resolve a layer by exact name, then by a unique case-insensitive match.
fn resolve_layer_by_name<'a>(
    drawing: &'a cad_db::DrawingDatabase,
    name: &str,
) -> Option<&'a cad_db::Layer> {
    let name = name.trim();
    if let Some(layer) = drawing.layers().find(|layer| layer.name == name) {
        return Some(layer);
    }
    let mut matches = drawing
        .layers()
        .filter(|layer| layer.name.eq_ignore_ascii_case(name));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
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
