//! Font-report plumbing for one owner tick.
//!
//! The read-only Font Manager reads `cad_fonts` from the worker thread while the
//! [`CadSession`] that owns the face registry is `!Send` and main-thread only.
//! The tick is the only place allowed to project the session's report, and it
//! mirrors it into [`Shared`] exactly like the layer panel does; the host never
//! invents a face row and never reads the session from the IPC thread.

use std::sync::{Arc, Mutex};

use cad_desktop_shell::state::{lock, Shared};
use cad_ui_core::session::CadSession;

use super::CAD;

/// Mirror the session's `CadSession::fonts()` projection for the `cad_fonts` read.
///
/// `None` (no session yet) leaves the read refusing with `cad_waiting`.
pub(super) fn mirror(shared: &Arc<Mutex<Shared>>) {
    let fonts = CAD.with(|cell| cell.borrow().as_ref().map(CadSession::fonts));
    if let Some(fonts) = fonts {
        lock(shared).cad_fonts = Some(fonts);
    }
}
