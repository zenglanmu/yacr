//! The typed operations, each a thin sequence of upstream calls.
//!
//! Every kernel-facing operation passes the same gate: neutral body →
//! [`lift`] (record) → `Body::from_record(.., false)` → the upstream call →
//! `Body::to_record` → [`unlift`] (neutral body). A refusal at the gate is an
//! `Unsupported` outcome with a stable code, never a made-up body.

pub(crate) mod boolean;
pub(crate) mod chamfer;
pub(crate) mod extrude_profile;
pub(crate) mod mass;
pub(crate) mod offset_face;
pub(crate) mod primitive;
pub(crate) mod transform;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cad_domain::{BrepData, Diagnostic};
use cad_kernel_adapter::{SolidOutcome, SolidProduct};
use solidcore_ops::cancel::{within, CancelToken};
use solidcore_record::brep::Brep as Record;

use crate::error::LiftRefusal;
use crate::record;

/// A pick's tolerance as a fraction of the body's bounding-box diagonal.
///
/// Shared by every point selector (a chamfer edge, a face to offset) so the same
/// body and the same point resolve with the same resolution.
pub(crate) const PICK_RELATIVE: f64 = 1e-3;

/// The pick tolerance floor, so a degenerate/zero-extent record still accepts a
/// point exactly on the geometry it names.
pub(crate) const PICK_FLOOR: f64 = 1e-9;

/// The diagonal of the record's vertex bounding box, or `0.0` when a vertex is
/// not finite (the floor then supplies the tolerance).
pub(crate) fn bbox_diagonal(record: &Record) -> f64 {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for vertex in &record.vertices {
        for (axis, value) in vertex.point.iter().enumerate() {
            min[axis] = min[axis].min(*value);
            max[axis] = max[axis].max(*value);
        }
    }
    let mut squared = 0.0;
    for axis in 0..3 {
        let extent = max[axis] - min[axis];
        if !extent.is_finite() {
            return 0.0;
        }
        squared += extent * extent;
    }
    squared.sqrt()
}

/// How often a watcher thread polls the seam's cancellation callback.
///
/// This is the **detection granularity** of cooperative cancellation while a
/// kernel call runs, not a claim of preemption.
pub(crate) const POLL: Duration = Duration::from_millis(50);

/// A neutral body as the seam's success product (no non-fatal diagnostics in P1).
pub(crate) fn body_outcome(brep: BrepData) -> SolidOutcome {
    SolidOutcome::Success(SolidProduct::Body {
        brep,
        diagnostics: Vec::<Diagnostic>::new(),
    })
}

/// Neutral body → kernel record: the single lift gate of this crate.
///
/// The record's tolerance is the body's own when the source states one (a length,
/// `BrepData::tolerance`), else the largest tolerance a face states, else
/// [`record::DEFAULT_TOLERANCE`] — **our** default, not a number any producer
/// stated (see `to_record::record_tolerance`). It is never derived from the
/// geometry.
pub(crate) fn lift(brep: &BrepData) -> Result<Record, LiftRefusal> {
    record::to_record(brep, brep.tolerance.unwrap_or(record::DEFAULT_TOLERANCE))
}

/// Kernel record → neutral body.
pub(crate) fn unlift(record: &Record) -> Result<BrepData, LiftRefusal> {
    record::from_record(record)
}

/// Run one kernel call under a cancellation watcher.
///
/// The seam's `cancelled` callback is polled every [`POLL`] on a scoped watcher
/// thread that cancels the kernel's [`CancelToken`]; the call itself runs inside
/// `solidcore_ops::cancel::within`, so the kernel stops at its next check. The
/// watcher is joined before this function returns, and the [`StopGuard`] is
/// created *before* the call so a panic inside it still stops the watcher while
/// `thread::scope` unwinds instead of hanging on a sleeper. Finishing wakes the
/// watcher immediately (`unpark`), so a completed call never waits a whole poll.
pub(crate) fn watched<T>(
    cancelled: &(dyn Fn() -> bool + Sync),
    call: impl FnOnce() -> solidcore_ops::Result<T>,
) -> solidcore_ops::Result<T> {
    let token = CancelToken::new();
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let watcher_token = token.clone();
        let watcher_stop = &stop;
        let watcher = scope.spawn(move || {
            while !watcher_stop.load(Ordering::Relaxed) {
                if cancelled() {
                    watcher_token.cancel();
                    return;
                }
                // `StopGuard` unparks us when the call ends, so finishing never
                // waits a whole `POLL`; `POLL` is the cancellation-detection
                // granularity while the kernel runs.
                std::thread::park_timeout(POLL);
            }
        });
        let watcher_thread = watcher.thread().clone();
        let guard = StopGuard {
            stop: &stop,
            thread: &watcher_thread,
        };
        let result = within(&token, call);
        drop(guard);
        // Joined here (not at the end of the scope) so the caller's post-check
        // is about the call, not about the watcher.
        let _ = watcher.join();
        result
    })
}

/// Stops the watcher thread, including while unwinding.
///
/// Sets the flag and unparks the thread on `Drop`: a panic inside the kernel call
/// must not leave `thread::scope` waiting for a parked or sleeping watcher (the
/// scope joins before it resumes the unwind).
struct StopGuard<'a> {
    stop: &'a AtomicBool,
    thread: &'a std::thread::Thread,
}

impl Drop for StopGuard<'_> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.unpark();
    }
}
