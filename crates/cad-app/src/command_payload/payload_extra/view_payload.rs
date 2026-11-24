//! The typed `VIEW`/`NEWVIEW` payload rows.

use cad_domain::Point3;

/// The option of a typed `VIEW <option>` / `-VIEW <option>` line:
/// `SAVE <name>`, `RESTORE <name>`, `LIST` or `DELETE <name>`.
///
/// The store is the persisted document table `tables/views.json` (bounded to 32
/// case-insensitively unique names). The typed grammar lives in `cad-ui-core`.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewCommand {
    /// Capture the current camera, 2D/3D mode, work plane and active space.
    Save(String),
    /// Apply a saved view (the same path as `VIEWGO`).
    Restore(String),
    /// Report the saved names; writes nothing.
    List,
    /// Drop one saved view.
    Delete(String),
    /// `NEWVIEW WINDOW <name> <x1> <y1> <x2> <y2>`: store a view that frames
    /// the world-space rectangle `first..second` in the current 2D viewport.
    ///
    /// The current camera is **not** moved: the command defines and stores a
    /// view, exactly like [`ViewCommand::Save`] stores the display as it is.
    Window {
        name: String,
        first: Point3,
        second: Point3,
    },
}
