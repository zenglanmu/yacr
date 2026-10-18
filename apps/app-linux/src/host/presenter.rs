//! Main-thread presenter lifecycle control: stop/release/close and the
//! evidence-only force unrealize/realize hooks.

use std::sync::{Arc, Mutex};

use super::{lock, Shared, BRIDGE};

/// Ask the tick to stop and release the presenter (idempotent).
pub fn request_stop(shared: &Arc<Mutex<Shared>>) {
    let mut state = lock(shared);
    state.shell.stop_requested = true;
    state.redraw_pump = None;
}

/// Enter the terminal closing state and drop the surface **on the main thread**
/// (B1): sets `Closing` so a later `try_init_gpu` can never run, then releases.
pub fn close_presenter() {
    BRIDGE.with(|cell| {
        if let Some(owner) = cell.borrow_mut().as_mut() {
            owner.close();
        }
    });
}

/// Stop and release the GPU presenter **on the main thread**, before the window
/// is torn down. Safe to call repeatedly.
pub fn release_presenter() {
    BRIDGE.with(|cell| {
        if let Some(owner) = cell.borrow_mut().as_mut() {
            owner.release_gpu();
        }
    });
}

/// Evidence-only: force the overlay to unrealize on the main thread, running the
/// registered unrealize hook (GPU released before native teardown). The owner is
/// temporarily moved out of the thread-local cell so GTK's **synchronous**
/// unrealize signal cannot re-enter an already-borrowed `RefCell`; the GPU is
/// released here, before the native unrealize, then the owner is put back.
/// Returns whether a bridge existed.
pub fn force_unrealize() -> bool {
    let owner = BRIDGE.with(|cell| cell.borrow_mut().take());
    match owner {
        Some(mut owner) => {
            owner.release_gpu();
            owner.force_unrealize();
            BRIDGE.with(|cell| *cell.borrow_mut() = Some(owner));
            true
        }
        None => false,
    }
}

/// Evidence-only: re-realize/re-map the overlay after a forced unrealize so the
/// next tick can create a new GPU generation. Uses the same move-out pattern so
/// any synchronous GTK signal cannot re-enter a borrowed `RefCell`.
pub fn force_realize() -> bool {
    let owner = BRIDGE.with(|cell| cell.borrow_mut().take());
    match owner {
        Some(mut owner) => {
            owner.force_realize();
            BRIDGE.with(|cell| *cell.borrow_mut() = Some(owner));
            true
        }
        None => false,
    }
}
