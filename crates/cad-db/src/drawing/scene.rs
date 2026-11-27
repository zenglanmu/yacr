//! Persisted scenes (`tables/scenes.json`, `.ymodel` minor 20; a row with the
//! `tags` visible-layer snapshot makes the writer stamp minor 23, a row with the
//! `display` Shadows/Fog snapshot minor 24).
//!
//! Document content like the named-view table: held by [`DrawingDatabase`],
//! written by `snapshot` when it holds a row, decoded by `from_snapshot`. The
//! two write paths mirror the view table exactly:
//!
//! * [`DrawingDatabase::plan_save_scene`] / [`DrawingDatabase::plan_delete_scene`]
//!   validate against the current table and return the reversible [`ScenePatch`]
//!   before anything mutates (an identical save is `None`: no revision);
//! * [`DrawingDatabase::apply_scene_patches`] applies the patches as **one**
//!   revision and one `tables/scenes.json` dirty mark.
//!
//! Undo/redo swaps the patch sides and replays through the same apply method.
//! A scene is presentation state (camera/space/style), so restoring one is a
//! session apply and never touches this table.

use cad_domain::scene::{markers, Scene, SceneTable, SCENE_CAP};
use cad_domain::{CadError, CadResult, Revision, TransactionId};

use super::DrawingDatabase;
use crate::scope::TableKind;
use crate::{ChangeScope, ChangeSet};

/// A reversible scene row.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenePatch {
    /// The row name (trimmed, exactly as stored).
    pub name: String,
    /// The row before the write; `None` for a create.
    pub before: Option<Scene>,
    /// The row after the write; `None` for a delete.
    pub after: Option<Scene>,
}

impl ScenePatch {
    /// The inverse patch (undo/redo replay).
    pub fn inverse(&self) -> ScenePatch {
        ScenePatch {
            name: self.name.clone(),
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

/// What one planned write will change.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneWrite {
    /// The reversible row.
    pub patch: ScenePatch,
    /// Whether an existing scene of the same name will be replaced.
    pub replaced: bool,
}

impl DrawingDatabase {
    /// The document's scenes (empty when it has none).
    pub fn scenes(&self) -> &SceneTable {
        &self.scenes
    }

    /// Validate a scene save and return the patch it would write.
    ///
    /// `None` means the table already holds exactly this row (no revision, no
    /// history step). An invalid row or a new name at [`SCENE_CAP`] is a named
    /// refusal and nothing changes.
    pub fn plan_save_scene(&self, mut scene: Scene) -> CadResult<Option<SceneWrite>> {
        let name = scene.view.name.trim().to_string();
        scene.view.name = name.clone();
        scene
            .validate()
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        let before = self.scenes.resolve(&name).cloned();
        if before.as_ref() == Some(&scene) {
            return Ok(None);
        }
        if before.is_none() && self.scenes.len() >= SCENE_CAP {
            return Err(CadError::InvalidInput(format!(
                "{}: `{name}` would be scene {}, but the table holds at most {SCENE_CAP} rows",
                markers::SCENE_CAP_EXCEEDED,
                self.scenes.len() + 1
            )));
        }
        Ok(Some(SceneWrite {
            patch: ScenePatch {
                name,
                before: before.clone(),
                after: Some(scene),
            },
            replaced: before.is_some(),
        }))
    }

    /// Validate a scene delete and return the patch it would write.
    pub fn plan_delete_scene(&self, name: &str) -> CadResult<SceneWrite> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "{}: a scene needs a non-empty name",
                markers::SCENE_UNKNOWN
            )));
        }
        let before = self
            .scenes
            .resolve(&name)
            .cloned()
            .ok_or_else(|| unknown_scene(&name))?;
        let name = before.view.name.clone();
        Ok(SceneWrite {
            patch: ScenePatch {
                name,
                before: Some(before),
                after: None,
            },
            replaced: false,
        })
    }

    /// Apply scene patches as **one** revision and one table dirty mark.
    ///
    /// Every patch is validated before anything mutates: a non-empty name, at
    /// least one side, a valid row, and no two patches naming the same scene.
    pub fn apply_scene_patches(
        &mut self,
        patches: &[ScenePatch],
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        if patches.is_empty() {
            return Err(CadError::InvalidInput(
                "a scene write needs at least one patch".into(),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for patch in patches {
            let name = patch.name.trim();
            if name.is_empty() {
                return Err(CadError::InvalidInput(format!(
                    "{}: a scene patch has an empty name",
                    markers::SCENE_UNKNOWN
                )));
            }
            if !seen.insert(name.to_ascii_lowercase()) {
                return Err(CadError::InvalidInput(format!(
                    "{}: `{name}` is patched more than once",
                    markers::SCENE_DUPLICATE_NAME
                )));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a scene patch needs a before or after row".into(),
                ));
            }
            for row in patch.before.iter().chain(patch.after.iter()) {
                row.validate()
                    .map_err(|error| CadError::InvalidInput(error.to_string()))?;
            }
        }

        let mut next = self.scenes.clone();
        for patch in patches {
            match &patch.after {
                Some(after) => {
                    next.save(after.clone())
                        .map_err(|error| CadError::InvalidInput(error.to_string()))?;
                }
                None => {
                    next.remove(&patch.name);
                }
            }
        }
        if next == self.scenes {
            return Err(CadError::InvalidInput(
                "a scene patch would change nothing".into(),
            ));
        }

        let before = self.revision;
        self.scenes = next;
        let after = Revision(before.0 + 1);
        self.revision = after;
        let mut scope = ChangeScope::default();
        scope.table(TableKind::Scenes);
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
            scope,
        })
    }
}

fn unknown_scene(name: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{}: there is no scene named `{name}` in this drawing",
        markers::SCENE_UNKNOWN
    ))
}

impl crate::DrawingDatabaseBuilder {
    /// Install the decoded scene table (a load path).
    pub fn set_scenes(&mut self, table: SceneTable) -> CadResult<()> {
        for scene in table.scenes() {
            scene
                .validate()
                .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        }
        self.database.scenes = table;
        Ok(())
    }
}
