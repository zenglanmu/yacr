//! Layer-panel plumbing for one owner tick.
//!
//! The React layer panel reads `cad_layers` and queues its mutations (the three
//! session-only ones plus the create/rename/delete table writes), and both belong
//! to the native [`CadSession`] this host owns on the main thread: the tick is the
//! only place allowed to touch them. Keeping the take/drain/mirror discipline here
//! stops `tick` from growing a second copy of it (and keeps the file under the
//! code-file size limit).
//!
//! Nothing here resolves a layer name, decides visibility or validates a new
//! name: the session's own funnel does that (`CadSession::set_layer_visibility` /
//! `restore_layers` / `set_active_layer` / `create_layer` / `rename_layer` /
//! `delete_layer`), so a refusal is the real host refusal. Every structural write
//! is a real database write, and the panel sees the result only by re-reading
//! `cad_layers` after the tick mirrored it — this host never invents a row.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cad_desktop_shell::commands::drain_layer_writes;
use cad_desktop_shell::state::{lock, Shared};
use cad_ui_core::session::{CadLayerWrite, CadSession};

use super::CAD;

/// Apply the writes in admission order through the owner-thread session.
pub(super) fn apply_writes(session: &CadSession, mut writes: VecDeque<CadLayerWrite>) {
    for error in drain_layer_writes(&mut writes, |write| write.apply(session)) {
        eprintln!("[probe] cad layer write rejected: {error}");
    }
}

/// Mirror the session's layer projection for the worker-thread `cad_layers` read.
///
/// A projection error is mirrored as the error itself, never replaced by stale
/// rows; `None` (no session) leaves the read refusing with `cad_waiting`.
pub(super) fn mirror(shared: &Arc<Mutex<Shared>>) {
    let layers = CAD.with(|cell| cell.borrow().as_ref().map(|session| session.layers()));
    if let Some(layers) = layers {
        lock(shared).cad_layers = Some(layers);
    }
}
