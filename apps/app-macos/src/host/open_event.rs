//! Runtime "open with"/double-click dispatch for the macOS host.
//!
//! macOS delivers an OS open as `RunEvent::Opened { urls }` on the main thread.
//! A double-click **launch** can deliver it before the CAD session exists: the
//! session is created on the first tick after the React page reports its CAD
//! rect (`host::tick`). Dropping the event would silently ignore the file the
//! user double-clicked, and opening it without a session is impossible, so the
//! path is queued and drained by the next tick.
//!
//! The queue is bounded ([`PENDING_CAP`]) and ordered: a later open never jumps
//! ahead of an earlier one. An overflow drops the **oldest** pending path and
//! says so on stderr (`[probe] open event queue full`), so the host never
//! pretends to have opened a file it discarded. `Busy` (a save or load owns the
//! slot) leaves the path at the head of the queue for the next tick instead of
//! failing it.
//!
//! Everything here is main-thread-only (`thread_local`, like the session).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;

use cad_host::PathOpenOutcome;

use super::CAD;

/// Upper bound on paths waiting for the session/file slot.
///
/// One user gesture produces one path; the bound only exists so a hostile or
/// buggy event source cannot grow memory without limit. It is not a batch API.
pub const PENDING_CAP: usize = 16;

thread_local! {
    /// Paths delivered before the session existed or while it was loading.
    static PENDING: RefCell<VecDeque<PathBuf>> = const { RefCell::new(VecDeque::new()) };
}

/// What happened to one delivered path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    /// The open funnel was started for this path.
    pub started: bool,
    /// The oldest queued path displaced by this one (bounded-queue overflow).
    pub dropped: Option<PathBuf>,
}

/// Hand one local path from `RunEvent::Opened` to the CAD session.
///
/// Always enqueues first and then drains in order, so an event delivered while a
/// load/save is running keeps its place instead of being dropped. A real platform
/// refusal is reported on stderr by [`drain`] and removed from the queue; the
/// caller only needs the started/queued answer for its own evidence line.
pub fn dispatch(path: PathBuf) -> Dispatch {
    let dropped = enqueue(path.clone());
    drain();
    let started = !PENDING.with(|pending| pending.borrow().iter().any(|queued| queued == &path));
    Dispatch { started, dropped }
}

/// Start every queued open that can start now, preserving arrival order.
///
/// Called on the main thread (the event handler, and once per tick so a path
/// queued before the session existed is picked up as soon as it does). `Busy` or
/// "no session yet" stops the drain and keeps the queue intact — a later tick
/// retries; an explicit failure is logged and removed, never retried forever.
pub fn drain() {
    loop {
        let Some(path) = PENDING.with(|pending| pending.borrow().front().cloned()) else {
            return;
        };
        let outcome = CAD.with(|cad| match cad.borrow().as_ref() {
            Some(session) => session.open_path(&path).map(Some),
            None => Ok(None),
        });
        match outcome {
            Ok(Some(PathOpenOutcome::Started)) => {
                PENDING.with(|pending| {
                    pending.borrow_mut().pop_front();
                });
                eprintln!("[probe] open event started: {}", path.display());
            }
            // No session yet, or a save/load owns the slot: retry next tick.
            Ok(Some(PathOpenOutcome::Busy)) | Ok(None) => return,
            Err(error) => {
                PENDING.with(|pending| {
                    pending.borrow_mut().pop_front();
                });
                eprintln!("[probe] open event FAILED: {} ({error})", path.display());
            }
        }
    }
}

/// Append to the bounded queue, returning the oldest path if it overflowed.
fn enqueue(path: PathBuf) -> Option<PathBuf> {
    PENDING.with(|pending| {
        crate::open_event::enqueue_bounded(&mut pending.borrow_mut(), path, PENDING_CAP)
    })
}
