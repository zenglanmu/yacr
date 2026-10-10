//! Asynchronous-open wiring for the Android host (F01, `docs/import-async.md`).
//!
//! The core owns the decision half of a background open (`ImportManager`,
//! `begin_async_open` / `poll_async_open` / `cancel_async_open`, the stamp guard
//! and the retained `ImportProgressSnapshot`). This module is the host half:
//!
//! * it registers the live controller/handle/view/document so a poll tick can
//!   reach them without crossing threads,
//! * it drives `poll_async_open` from a Slint timer on the UI thread and pushes
//!   the progress panel through the shared [`crate::state_push::push_import_state`]
//!   funnel,
//! * it applies a published result to the render bridge exactly once (the core
//!   publishes the database; the host only mirrors it), and
//! * it wires the resize/safe-area entry point to the pure
//!   [`crate::state_push::apply_surface_size`] helper.
//!
//! Timers are only available on the Slint/Android thread; the pure poll logic is
//! kept target-agnostic so it is testable without a window.

use super::*;

#[cfg(target_os = "android")]
use cad_app::AsyncOpenPoll;

/// How `poll_import_once` classified one controller poll.
///
/// A host needs only whether to keep polling, whether the open published, or
/// which terminal it reached. The progress payload is already mirrored into the
/// shell by the poll itself.
///
/// Only the Android host drives the poll timer, so this and the helpers below it
/// are compiled for that target (their tests compile under
/// `--target aarch64-linux-android`).
#[cfg(target_os = "android")]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImportPollOutcome {
    /// No job is running (never started, or already observed terminal).
    Idle,
    /// The job is still running.
    Running,
    /// The job finished and the core published its database.
    Opened { entities: usize },
    /// The job was cancelled/superseded; nothing was published.
    Cancelled,
    /// The import failed; the previous document is untouched.
    Failed(CadError),
}

/// Poll the controller once and mirror the result into the host/UI.
///
/// The publication itself happens inside `HostController::poll_async_open` under
/// the stamp guard; this function never re-imports or re-publishes, so a second
/// call after [`ImportPollOutcome::Opened`] is an `Idle` no-op.
///
/// The progress panel is pushed from the shared state funnel on every poll, so a
/// running job shows real progress and an idle/opened controller clears it.
#[cfg(target_os = "android")]
pub(crate) fn poll_import_once(
    controller: &Rc<RefCell<HostController>>,
    handle: &SharedHandle,
    view: &SharedView,
    incoming: &IncomingDocument,
) -> ImportPollOutcome {
    let poll = controller.borrow_mut().poll_async_open();
    // Every poll refreshes the progress panel first, including the terminal one:
    // a failed/cancelled job stays visible with its explicit text.
    if let Some(handle) = handle.borrow().as_ref() {
        push_import_state(controller, handle);
    }
    match poll {
        AsyncOpenPoll::Idle => ImportPollOutcome::Idle,
        AsyncOpenPoll::Running { .. } => ImportPollOutcome::Running,
        AsyncOpenPoll::Opened { opened, .. } => {
            apply_opened(controller, handle, view, incoming, opened.as_ref());
            ImportPollOutcome::Opened {
                entities: opened.entities,
            }
        }
        AsyncOpenPoll::Cancelled { .. } => {
            // Nothing was published; report and refresh indirectly affected
            // panels without touching the document.
            set_status(handle, "已取消打开：当前文档与未保存批注保留".to_string());
            refresh_after_terminal(controller, handle, view);
            ImportPollOutcome::Cancelled
        }
        AsyncOpenPoll::Failed { error, .. } => {
            set_status(handle, format!("打开失败（当前文档保留）：{error}"));
            refresh_after_terminal(controller, handle, view);
            ImportPollOutcome::Failed(error)
        }
    }
}

/// Mirror a core-published document into the render bridge and panels.
///
/// The core has already replaced the authoritative document (under the stamp
/// guard); this only makes the derived render state agree with it. It is called
/// exactly once per published result by the poll driver.
#[cfg(target_os = "android")]
fn apply_opened(
    controller: &Rc<RefCell<HostController>>,
    handle: &SharedHandle,
    view: &SharedView,
    incoming: &IncomingDocument,
    opened: &cad_app::host::OpenedDrawing,
) {
    let drawing = {
        let mut controller = controller.borrow_mut();
        let _ = controller.fit();
        controller.drawing()
    };
    if let Some(handle) = handle.borrow().as_ref() {
        let _ = handle.cancel_draw_capture();
    }
    *incoming.borrow_mut() = drawing;
    let viewport = controller.borrow().viewport_id;
    sync_view_camera(view, controller, viewport);
    if let Some(view) = view.borrow().as_ref() {
        view.request_redraw();
    }
    if let Some(handle) = handle.borrow().as_ref() {
        push_panel_state(controller, handle, view);
    }
    set_status(handle, format!("已打开：{}", opened.completeness_label));
    #[cfg(target_os = "android")]
    if let Some(message) = load_fonts_for_current_document(controller, view) {
        set_status(handle, message);
    }
}

/// Refresh the panels after a cancelled/failed terminal.
///
/// The document is unchanged, but the progress panel must show the terminal text
/// and the rest of the chrome stays consistent with the same snapshot.
#[cfg(target_os = "android")]
fn refresh_after_terminal(
    controller: &Rc<RefCell<HostController>>,
    handle: &SharedHandle,
    view: &SharedView,
) {
    if let Some(handle) = handle.borrow().as_ref() {
        push_panel_state(controller, handle, view);
    }
}

#[cfg(target_os = "android")]
fn set_status(handle: &SharedHandle, text: String) {
    if let Some(handle) = handle.borrow().as_ref() {
        let _ = handle.set_status(text);
    }
}

/// Load the fonts the current drawing references from packaged assets.
///
/// Android-only extension of the successful-open path: assets exist only inside
/// the activity. Returns a status message when the load attempted something.
#[cfg(target_os = "android")]
fn load_fonts_for_current_document(
    controller: &Rc<RefCell<HostController>>,
    view: &SharedView,
) -> Option<String> {
    let requested = {
        let controller = controller.borrow();
        match controller.drawing() {
            Some(drawing) => cad_platform::fonts::requested_fonts(drawing.as_ref()),
            None => Vec::new(),
        }
    };
    let view = view.borrow();
    if requested.is_empty() {
        if let Some(view) = view.as_ref() {
            view.clear_fonts();
        }
        return None;
    }
    match android_fonts::install_fonts(view.as_ref(), &requested) {
        Ok(report) => Some(android_fonts::summary(&report, requested.len())),
        Err(e) => Some(format!("字体加载未完成：{e}")),
    }
}

/// The live host objects a poll tick and the resize callback need.
///
/// Stored per-thread because every field is `Rc`-based and the Slint event loop,
/// its timers and the Android `SurfaceHolder` callbacks all run on the UI thread.
/// Keeping it `thread_local` avoids pretending these handles are `Send`.
struct Runtime {
    controller: Rc<RefCell<HostController>>,
    handle: SharedHandle,
    view: SharedView,
    /// Only the Android poll driver hands this to `poll_import_once`; the resize
    /// entry point never needs it.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    incoming: IncomingDocument,
    /// Last logical surface size + DPI scale reported by the Activity, or `None`
    /// before the first `set_surface_size`. Retained so a later inset update
    /// re-applies the same surface with the new safe area.
    surface: Cell<Option<([f64; 2], f64)>>,
    /// Last safe-area insets `[top, right, bottom, left]` in logical pixels.
    /// Zeros until a platform source provides them; never invented.
    safe_insets: Cell<[f64; 4]>,
}

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// Register the live host objects for poll ticks and the resize entry point.
///
/// Called once from [`start`] after the shell handle and view slots exist.
pub(crate) fn install_runtime(
    controller: Rc<RefCell<HostController>>,
    handle: SharedHandle,
    view: SharedView,
    incoming: IncomingDocument,
) {
    RUNTIME.with(|slot| {
        *slot.borrow_mut() = Some(Runtime {
            controller,
            handle,
            view,
            incoming,
            surface: Cell::new(None),
            safe_insets: Cell::new([0.0; 4]),
        });
    });
}

/// Run `f` with the live host objects, if the host is running on this thread.
fn with_runtime<R>(f: impl FnOnce(&Runtime) -> R) -> Option<R> {
    RUNTIME.with(|slot| slot.borrow().as_ref().map(f))
}

/// Whether this build can run the background import worker.
///
/// The worker is a plain `std::thread`, so every native target (including
/// Android) can; a `wasm32` host cannot and must keep the synchronous fallback.
pub(crate) fn worker_available() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

/// Apply a new logical surface size / DPI scale and re-push the derived state.
///
/// Wraps the pure [`apply_surface_size`] helper (which updates the authoritative
/// viewport without moving the camera) with the host-side follow-up: re-sync the
/// render bridge camera, request a redraw and re-push the panels/overlays so a
/// rotation or resize takes effect on screen.
pub(crate) fn apply_surface_resize(
    controller: &Rc<RefCell<HostController>>,
    handle: &SharedHandle,
    view: &SharedView,
    size_logical: [f64; 2],
    dpi_scale: f64,
    safe_insets: [f64; 4],
) -> CadResult<()> {
    apply_surface_size(controller, size_logical, dpi_scale, safe_insets)?;
    let viewport = controller.borrow().viewport_id;
    sync_view_camera(view, controller, viewport);
    if let Some(view) = view.borrow().as_ref() {
        view.request_redraw();
    }
    if let Some(handle) = handle.borrow().as_ref() {
        push_panel_state(controller, handle, view);
    }
    Ok(())
}

/// Android entry point for a surface-size / rotation change (audit U07).
///
/// The embedding Activity calls this from its `SurfaceHolder` size callback with
/// the new logical size and DPI scale. It updates the authoritative viewport
/// (`logical_size` / `dpi_scale`) without moving the camera, then re-syncs the
/// render bridge and re-pushes the layout/overlay state. The last reported safe
/// insets are re-applied, so a rotation keeps the current system-bar margins.
///
/// The `Runtime` is `!Send` (it holds `Rc` handles), so this must be called on
/// the Slint UI thread — the same thread the Android `SurfaceHolder` callbacks
/// run on. Called before [`start`] has installed the runtime, or from another
/// thread, it returns an explicit error rather than silently doing nothing.
pub fn set_surface_size(width: f64, height: f64, scale: f64) -> CadResult<()> {
    let size = [width, height];
    if !size[0].is_finite() || !size[1].is_finite() || !scale.is_finite() {
        return Err(CadError::InvalidInput(
            "surface size/scale must be finite".into(),
        ));
    }
    with_runtime(|runtime| {
        runtime.surface.set(Some((size, scale)));
        apply_surface_resize(
            &runtime.controller,
            &runtime.handle,
            &runtime.view,
            size,
            scale,
            runtime.safe_insets.get(),
        )
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
/// explicitly zero (see the report and `docs/validation-android.md`).
pub fn set_surface_insets(top: f64, right: f64, bottom: f64, left: f64) -> CadResult<()> {
    let insets = [top, right, bottom, left];
    if insets.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(CadError::InvalidInput(
            "safe insets must be finite and non-negative".into(),
        ));
    }
    with_runtime(|runtime| {
        runtime.safe_insets.set(insets);
        // Before any surface is known there is nothing to apply to; the insets
        // are retained and used by the first `set_surface_size`.
        match runtime.surface.get() {
            Some((size, scale)) => apply_surface_resize(
                &runtime.controller,
                &runtime.handle,
                &runtime.view,
                size,
                scale,
                insets,
            ),
            None => Ok(()),
        }
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
/// Returns `true` while the job is still running, so the driver keeps the timer;
/// any terminal stops it.
#[cfg(target_os = "android")]
pub(crate) fn poll_tick() -> bool {
    match with_runtime(|runtime| {
        poll_import_once(
            &runtime.controller,
            &runtime.handle,
            &runtime.view,
            &runtime.incoming,
        )
    }) {
        Some(ImportPollOutcome::Running) => true,
        // Idle/opened/cancelled/failed: no live job to keep polling.
        Some(_) | None => false,
    }
}

/// Start a repeated poll timer for the running asynchronous open.
///
/// Idempotent: a second call while polling does not create a second timer. The
/// timer is a no-op off the timer thread, and no-op off Android (there is no
/// Slint runtime there in this build), which is why the synchronous fallback is
/// kept in `open_drawing`.
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
