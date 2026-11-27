//! The typed `SCENE` payload row.

/// The option of a typed `SCENE <option>` line: `SAVE <name>`, `RESTORE <name>`,
/// `LIST` or `DELETE <name>`.
///
/// The store is the persisted document table `tables/scenes.json` (bounded to 32
/// case-insensitively unique names). A scene captures the camera, 2D/3D mode,
/// work plane, active space and the visual style; the typed grammar lives in
/// `cad-ui-core`.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneCommand {
    /// Capture the current presentation state under a name (or replace it).
    Save(String),
    /// Apply a saved scene (camera, mode, work plane, space and style).
    Restore(String),
    /// Report the saved names; writes nothing.
    List,
    /// Drop one saved scene.
    Delete(String),
}
