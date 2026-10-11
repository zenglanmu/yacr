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
//! Timers are only available on the Slint/Android thread; the pure logic stays
//! target-agnostic so it is testable without a window.

use super::*;
use cad_host::HostCore;

/// Whether this build can run the background import worker.
///
/// The worker is a plain `std::thread`, so every native target (including
/// Android) can; a `wasm32` host cannot and must keep the synchronous fallback.
pub(crate) fn worker_available() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

/// The live host objects a poll tick and the resize callback need.
///
/// Stored per-thread because the `HostCore` handle is `Rc`-based and the Slint
/// event loop, its timers and the Android `SurfaceHolder` callbacks all run on
/// the UI thread. Keeping it `thread_local` avoids pretending the handle is
/// `Send`.
struct Runtime {
    core: HostCore,
}

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// Register the live host core for poll ticks and the resize entry point.
///
/// Called once from [`crate::start`] after the shell handle and view slots exist.
pub(crate) fn install_runtime(core: HostCore) {
    RUNTIME.with(|slot| {
        *slot.borrow_mut() = Some(Runtime { core });
    });
}

/// Run `f` with the live host core, if the host is running on this thread.
fn with_runtime<R>(f: impl FnOnce(&Runtime) -> R) -> Option<R> {
    RUNTIME.with(|slot| slot.borrow().as_ref().map(f))
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
    with_runtime(|runtime| {
        platform::set_surface_slot(size, scale);
        runtime.core.metrics()?;
        state_push::push_android_state(&runtime.core);
        Ok(())
    })
    .unwrap_or_else(|| {
        Err(CadError::InvalidInput(
            "surface hook not installed on this thread (call after start() on the UI thread)"
                .into(),
        ))
    })
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
    with_runtime(|runtime| {
        platform::set_safe_insets(insets);
        // Before any surface is known there is nothing to apply to; the insets
        // are retained and used by the first `set_surface_size`.
        let (size, _) = platform::surface_snapshot();
        if size[0] > 0.0 && size[1] > 0.0 {
            runtime.core.metrics()?;
            state_push::push_android_state(&runtime.core);
        }
        Ok(())
    })
    .unwrap_or_else(|| {
        Err(CadError::InvalidInput(
            "surface hook not installed on this thread (call after start() on the UI thread)"
                .into(),
        ))
    })
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
    with_runtime(|runtime| {
        if let Err(error) = runtime.core.tick() {
            log::warn!("android poll tick failed: {error}");
        }
        state_push::push_android_overlay(&runtime.core);
        if let Some(status) = state_push::take_pending_font_status() {
            if let Some(handle) = runtime.core.handle.borrow().as_ref() {
                let _ = handle.set_status(status);
            }
        }
        // Keep ticking while an open or an asynchronous SAF save picker is in
        // flight; both stop themselves once terminal.
        runtime.core.loading() || runtime.core.save_pending()
    })
    // No runtime on this thread: no live job to keep polling.
    .unwrap_or_default()
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
