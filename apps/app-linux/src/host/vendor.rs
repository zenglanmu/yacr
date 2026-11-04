//! Owner-tick glue for the vendor pass on the Linux host.
//!
//! The transport itself is shared (`cad_desktop_shell::vendor`); this module only
//! reaches the main-thread `CadSession` thread-local, which the shared crate
//! cannot name. Display-only: derived primitives never touch `cad-db`.

use std::sync::{Arc, Mutex};

use cad_desktop_shell::state::Shared;

use super::CAD;

/// Stage this tick's vendor records and install any settled derived snapshot.
pub(crate) fn tick(shared: &Arc<Mutex<Shared>>) {
    let records = CAD.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|session| session.take_vendor_records())
    });
    cad_desktop_shell::vendor::stage_from_tick(shared, records);
    CAD.with(|cell| {
        if let Some(session) = cell.borrow().as_ref() {
            cad_desktop_shell::vendor::apply_pending_representations(shared, session);
        }
    });
}
