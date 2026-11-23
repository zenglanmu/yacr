//! Owner-tick glue for the vendor pass on the Linux host.
//!
//! The transport itself is shared (`cad_desktop_shell::vendor`); this module only
//! reaches the main-thread `CadSession` thread-local, which the shared crate
//! cannot name. Display-only: derived primitives never touch `cad-db`.

use std::sync::{Arc, Mutex};

use cad_desktop_shell::state::Shared;

use super::CAD;

/// Stage this tick's vendor input, install any settled snapshot/semantics, and
/// publish the real `documentReady` state (ADR-0030 D9).
pub(crate) fn tick(shared: &Arc<Mutex<Shared>>) {
    CAD.with(|cell| {
        if let Some(session) = cell.borrow().as_ref() {
            cad_desktop_shell::vendor::tick(shared, session);
        }
    });
}
