//! Android Slint poll timer and the Activity's surface entry points.
//!
//! The core owns the decision half of a background open (`ImportManager`,
//! `begin_async_open` / `poll_async_open` / `cancel_async_open`, the stamp guard
//! and the retained `ImportProgressSnapshot`). This module is the Android host
//! half:
//!
//! * it registers the live [`HostCore`] so a poll tick and the resize callback
//!   can reach it without crossing threads,
//! * it drives `core.tick()` from a Slint timer on the UI thread (drain file
//!   events → advance the import → metrics → view sync) and re-applies the
//!   Android overlay, and
//! * it wires the resize/safe-area entry points to the shared `metrics` funnel
//!   through the [`crate::platform`] surface slots.
//!
//! It also owns the P3 request-waker bridge: the engine queue calls the installed
//! waker only on an empty→nonempty edge. The waker captures a `Weak` and merely
//! schedules a single-shot wake (never a synchronous pump, never a permanent
//! timer), so a queue wake advances via the event loop rather than the 100 ms IO
//! poll.
//!
//! Timers are only available on the Slint/Android thread; the pure logic stays
//! target-agnostic so it is testable without a window.

use super::*;
use cad_host::HostCore;
use cad_ui_slint::{EngineAdapter, UiHandle};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// Whether this build can run the background import worker.
///
/// The worker is a plain `std::thread`, so every native target (including
/// Android) can; a `wasm32` host cannot and must keep the synchronous fallback.
pub(crate) fn worker_available() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

/// The live engine binding and shell handle a poll tick or a resize callback
/// needs once the UI is up.
///
/// The binding is the single owner of the engine callbacks; keeping it here lets
/// the poll driver pull updates (`flush_updates`) and refresh canvas metrics
/// without a second driver. It is absent in the host-target unit tests, which
/// exercise only the controller-level resize behaviour.
pub(crate) struct UiBinding {
    pub(crate) binding: EngineAdapter,
    pub(crate) handle: UiHandle,
    /// Kept alive so the engine's `Weak`-based wake callback stays valid for the
    /// process; it holds no reference back to this binding (no cycle).
    pub(crate) _request_waker: Rc<RequestWaker>,
}

/// Owner-thread scheduler for the engine request waker.
///
/// The engine invokes the installed waker when the queue goes empty→nonempty
/// (and re-arms while executable work remains). This type turns that signal into
/// **at most one pending** single-shot wake: it only schedules and never pumps
/// synchronously, and it holds no strong reference to the runtime/binding, so
/// there is no reference cycle and no borrow re-entry.
pub(crate) struct RequestWaker {
    /// A wake is already scheduled; further signals coalesce into it. The
    /// single-shot clears it when it runs.
    scheduled: Cell<bool>,
    /// Number of times the installed waker reached a live scheduler.
    #[cfg(test)]
    requests: Cell<usize>,
    /// Number of actual single-shot schedules (≤ requests; proves coalescing).
    #[cfg(test)]
    schedules: Cell<usize>,
}

impl RequestWaker {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            scheduled: Cell::new(false),
            #[cfg(test)]
            requests: Cell::new(0),
            #[cfg(test)]
            schedules: Cell::new(0),
        })
    }

    /// Build the `Rc<dyn Fn()>` to install via
    /// `EngineAdapter::set_request_waker`. It captures only a `Weak`, so the
    /// queue never keeps the scheduler (or the runtime) alive.
    pub(crate) fn install(self: &Rc<Self>) -> Rc<dyn Fn()> {
        let weak: Weak<RequestWaker> = Rc::downgrade(self);
        Rc::new(move || {
            if let Some(waker) = weak.upgrade() {
                waker.request();
            }
        })
    }

    /// Coalesce one wake signal into a single pending schedule.
    fn request(self: &Rc<Self>) {
        #[cfg(test)]
        self.requests.set(self.requests.get() + 1);
        if self.scheduled.replace(true) {
            return;
        }
        #[cfg(test)]
        self.schedules.set(self.schedules.get() + 1);
        #[cfg(target_os = "android")]
        {
            let weak: Weak<RequestWaker> = Rc::downgrade(self);
            slint::Timer::single_shot(std::time::Duration::from_millis(0), move || {
                if let Some(waker) = weak.upgrade() {
                    waker.scheduled.set(false);
                }
                pump_wake();
            });
        }
    }

    /// Test-only: how many signals reached a live scheduler.
    #[cfg(test)]
    pub(crate) fn request_count(&self) -> usize {
        self.requests.get()
    }

    /// Test-only: how many single-shot schedules were armed (coalescing check).
    #[cfg(test)]
    pub(crate) fn schedule_count(&self) -> usize {
        self.schedules.get()
    }
}

/// The live host objects a poll tick and the resize callback need.
///
/// Stored per-thread because the `HostCore` handle is `Rc`-based and the Slint
/// event loop, its timers and the Android `SurfaceHolder` callbacks all run on
/// the UI thread. Keeping it `thread_local` avoids pretending the handle is
/// `Send`.
pub(crate) struct Runtime {
    core: HostCore,
    ui: Option<UiBinding>,
}

thread_local! {
    static RUNTIME: RefCell<Option<Rc<Runtime>>> = const { RefCell::new(None) };
}

/// Register the live host core (and its engine binding, once the UI is up) for
/// poll ticks and the resize entry point.
///
/// Called once from [`crate::start`] after the shell handle and view slots exist.
/// A `None` binding is the host-target test configuration.
pub(crate) fn install_runtime(core: HostCore, ui: Option<UiBinding>) {
    RUNTIME.with(|slot| {
        *slot.borrow_mut() = Some(Rc::new(Runtime { core, ui }));
    });
}

/// Clone the live runtime handle, releasing the thread-local `RefCell` borrow
/// before any business runs.
///
/// Holding that borrow across `tick`/`pump`/`flush` would panic if a callback
/// (the request waker, a Slint property callback or another platform entry point
/// such as `set_surface_size`) re-enters the runtime. The handle is an `Rc`, so
/// callers run business with no borrow held while the same authoritative
/// core/binding is reused — no tool/database copy.
pub(crate) fn runtime_handle() -> Option<Rc<Runtime>> {
    RUNTIME.with(|slot| slot.borrow().as_ref().cloned())
}

/// Test-only: whether the thread-local runtime slot is free to borrow, i.e. no
/// long-lived borrow is held across business.
#[cfg(test)]
pub(crate) fn runtime_borrow_available() -> bool {
    RUNTIME.with(|slot| slot.try_borrow_mut().is_ok())
}

/// The fixed apps order: refresh canvas metrics → advance the queue → flush the
/// derived updates → re-apply the Android overlay.
///
/// `flush_updates` deliberately never pumps (so the IO-first order is explicit),
/// and an empty completion drain is not a redraw. A pump failure is surfaced as
/// the real `command.failed` catalog copy; it is never a silent success.
fn refresh_and_pump(runtime: &Runtime) {
    let Some(ui) = &runtime.ui else {
        return;
    };
    if let Err(error) = ui.binding.refresh_metrics() {
        log::warn!("android canvas metrics refresh failed: {error}");
    }
    // Keep the pump and flush errors separate: the pump error is the real
    // execution/queue failure (the UI already drained any terminal completion and
    // reports the execution error with priority). The flush only publishes; a
    // publish error must never overwrite the error already surfaced above.
    let pump_result = ui.binding.pump_requests();
    if let Err(error) = &pump_result {
        let _ = runtime
            .core
            .set_catalog_status("command.failed", &[("error", &error.to_string())]);
    }
    if let Err(flush_error) = ui.binding.flush_updates() {
        if pump_result.is_err() {
            log::warn!(
                "android update flush failed after a pump error; preserving the pump error: \
                 {flush_error}"
            );
        } else {
            log::warn!("android update flush failed: {flush_error}");
        }
    }
    state_push::push_android_overlay(&ui.handle, &runtime.core);
}

/// Run the pump sequence from a scheduled request wake (Android event loop).
///
/// The runtime handle is cloned first, so the wake runs business with no
/// thread-local borrow held (the waker never sync-pumps and never re-borrows).
#[cfg(target_os = "android")]
pub(crate) fn pump_wake() {
    let Some(runtime) = runtime_handle() else {
        return;
    };
    refresh_and_pump(&runtime);
}

/// Android entry point for a surface-size / rotation change (audit U07).
///
/// The embedding Activity calls this from its `SurfaceHolder` size callback with
/// the new logical size and DPI scale. It records the surface, then re-derives
/// the authoritative viewport (`logical_size` / `dpi_scale`) through the shared
/// `metrics` funnel — which never moves the camera — and re-pushes the panels.
/// The last reported safe insets are re-applied, so a rotation keeps the current
/// system-bar margins.
///
/// The `Runtime` is `!Send` (it holds an `Rc` handle), so this must be called on
/// the Slint UI thread — the same thread the Android `SurfaceHolder` callbacks
/// run on. Called before [`crate::start`] has installed the runtime, or from
/// another thread, it returns an explicit error rather than silently doing
/// nothing.
pub fn set_surface_size(width: f64, height: f64, scale: f64) -> CadResult<()> {
    let size = [width, height];
    if !size[0].is_finite() || !size[1].is_finite() || !scale.is_finite() {
        return Err(CadError::InvalidInput(
            "surface size/scale must be finite".into(),
        ));
    }
    let Some(runtime) = runtime_handle() else {
        return Err(CadError::InvalidInput(
            "surface hook not installed on this thread (call after start() on the UI thread)"
                .into(),
        ));
    };
    platform::set_surface_slot(size, scale);
    runtime.core.metrics()?;
    refresh_and_pump(&runtime);
    Ok(())
}

/// Android entry point for safe-area insets (`[top, right, bottom, left]`, logical px).
///
/// The embedding Activity forwards the system-bar / soft-keyboard insets here
/// when they change (the soft keyboard is the main dynamic case). The insets are
/// applied to the last reported surface, so the canvas rect is the surface minus
/// the safe area; degenerate input (non-finite or negative) is refused instead of
/// being clamped.
///
/// **Not wired to the OS in this build**: no verified window-insets API is used
/// here (and `android-activity` 0.6 exposes no safe-area method to this host), so
/// the Activity glue (outside this repository) is expected to call this from its
/// `WindowInsets`/`View.OnApplyWindowInsets` listener. Until it does, insets stay
/// explicitly zero (see `docs/validation-android.md`).
pub fn set_surface_insets(top: f64, right: f64, bottom: f64, left: f64) -> CadResult<()> {
    let insets = [top, right, bottom, left];
    if insets.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(CadError::InvalidInput(
            "safe insets must be finite and non-negative".into(),
        ));
    }
    let Some(runtime) = runtime_handle() else {
        return Err(CadError::InvalidInput(
            "surface hook not installed on this thread (call after start() on the UI thread)"
                .into(),
        ));
    };
    platform::set_safe_insets(insets);
    // Before any surface is known there is nothing to apply to; the insets are
    // retained and used by the first `set_surface_size`.
    let (size, _) = platform::surface_snapshot();
    if size[0] > 0.0 && size[1] > 0.0 {
        runtime.core.metrics()?;
        refresh_and_pump(&runtime);
    }
    Ok(())
}

/// Poll the live host once (used by the Slint timer callback).
///
/// Returns `true` while the import is still running, so the driver keeps the
/// timer; any terminal stops it. `tick` drives the whole open funnel; the
/// Android overlay is re-applied afterwards and a queued font summary (set by the
/// asset loader during the publish) is shown last, preserving the historical
/// "font summary follows the open status" ordering.
#[cfg(target_os = "android")]
pub(crate) fn poll_tick() -> bool {
    // Clone the handle first: the thread-local borrow is released before `tick`
    // and any business callback (so a re-entrant platform entry cannot panic).
    let Some(runtime) = runtime_handle() else {
        // No runtime on this thread: no live job to keep polling.
        return false;
    };
    if let Err(error) = runtime.core.tick() {
        log::warn!("android poll tick failed: {error}");
    }
    // Necessary IO first (`tick` drains SAF/import), then metrics → pump queue →
    // flush; the queued work is advanced here too, but the waker remains the
    // event-driven driver so a queue wake never depends on this 100 ms timer.
    refresh_and_pump(&runtime);
    if let Some(status) = state_push::take_pending_font_status() {
        if let Some(ui) = &runtime.ui {
            let _ = ui.handle.set_status(status);
        }
    }
    // Keep ticking while an open or an asynchronous SAF save picker is in flight;
    // both stop themselves once terminal.
    runtime.core.loading() || runtime.core.save_pending()
}

/// Start a repeated poll timer for the running asynchronous open.
///
/// Idempotent: a second call while polling does not create a second timer. The
/// timer is a no-op off Android (there is no Slint runtime there in this build),
/// which is why the synchronous fallback is kept.
pub(crate) fn ensure_polling() {
    #[cfg(target_os = "android")]
    {
        POLL_TIMER.with(|slot| {
            if slot.borrow().is_some() {
                return;
            }
            let timer = slint::Timer::default();
            timer.start(
                slint::TimerMode::Repeated,
                std::time::Duration::from_millis(POLL_INTERVAL_MS),
                || {
                    if !poll_tick() {
                        stop_polling();
                    }
                },
            );
            *slot.borrow_mut() = Some(timer);
        });
    }
    #[cfg(not(target_os = "android"))]
    {
        // No Slint timer off Android in this crate; a host on such a target uses
        // the synchronous path (see `worker_available`).
    }
}

/// Stop the poll timer, if one is running.
#[cfg(target_os = "android")]
pub(crate) fn stop_polling() {
    POLL_TIMER.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

/// Poll interval while a background open runs.
///
/// Fast enough to feel live, slow enough that a poll is negligible compared with
/// the import work itself (the poll drains real events; it never synthesizes a
/// progress tick).
#[cfg(target_os = "android")]
const POLL_INTERVAL_MS: u64 = 100;

#[cfg(target_os = "android")]
thread_local! {
    /// The single repeated poll timer, owned by the UI thread.
    static POLL_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
}
