//! The Linux host's folder-picker seam (`cad_pick_directory`).
//!
//! Moving the font directory to a local path is only meaningful on the carrier
//! that can read one. `native-below` (Wayland) owns the local document and the
//! fonts directory; `dom-canvas` (X11/XWayland) hands the document to the wasm
//! session in the WebView and has no local directory at all, so the chrome
//! disables the row and the wasm adapter refuses by name.
//!
//! This module therefore installs the XDG-portal picker **only** on
//! `native-below`. Every other path leaves
//! [`Shared::directory_picker`](cad_desktop_shell::state::Shared::directory_picker)
//! as `None`, and `cad_pick_directory` answers the stable
//! `unsupported-runtime: pick_directory` marker instead of a `null` the Options
//! footer would read as a cancellation.
//!
//! The chooser itself runs on [`DirectoryPicker`]'s worker thread; the owner tick
//! drains the pending slot (`state::drain_directory_pick`), so an open portal
//! dialog never blocks the frame loop.

use std::sync::{Arc, Mutex};

use cad_desktop_shell::directory::{DirectoryChooser, DirectoryPicker};
use cad_desktop_shell::state::Shared;

/// Install the portal folder picker when this process runs the native carrier.
pub(crate) fn register(shared: &Arc<Mutex<Shared>>) {
    if crate::backend::surface_mode() != crate::backend::SurfaceMode::NativeBelow {
        return;
    }
    let chooser: DirectoryChooser = Arc::new(crate::platform::portal_pick_directory);
    cad_desktop_shell::state::configure_directory_picker(shared, DirectoryPicker::new(chooser));
}
