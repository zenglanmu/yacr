//! The Android native CAD surface: the frozen JNI surface callbacks and the
//! `SurfaceProbePresenter` lifecycle that presents below the WebView.
//!
//! # Composition
//!
//! The Kotlin lane inserts a `SurfaceView` at index `0` of the content frame
//! (behind the WebView, `setZOrderOnTop(false)`) and hands its
//! `android.view.Surface` to [`Java_dev_yacr_app_MainActivity_nativeSurfaceCreated`].
//! This module turns that `Surface` into an `ANativeWindow`, keeps it alive, and
//! builds the renderer surface from raw `AndroidNdkWindowHandle`s
//! ([`SurfaceProbePresenter::new_from_raw_handles`]) — the renderer's own
//! documented entry point for a host-owned, non-`Send` native widget. There is no
//! second window, no GLES path and no software fallback.
//!
//! # Threading (two threads, one hand-off)
//!
//! With Tauri `2.12.3`/`tao 0.37.1` the two sides are **not** the same thread:
//!
//! * the `SurfaceHolder` callbacks (and therefore the four JNI functions here)
//!   arrive on the **Android UI thread**;
//! * `tao`'s `onFirstActivityCreate` spawns a thread that prepares its own looper
//!   and calls the app entry point (`tao 0.37.1`,
//!   `platform_impl/android/ndk_glue.rs`), so the Tauri event loop — and the owner
//!   tick with every GPU call — runs there.
//!
//! The hand-off is one small `Mutex<PlatformSurface>`: the UI thread publishes
//! "which `Surface` is live, at what size" and the tick thread *takes* it. That
//! struct only holds plain data plus a refcounted `ANativeWindow` (`Send + Sync`),
//! so no GPU object ever crosses threads. The presenter and the render session
//! (which is `Rc`-based and `!Send`) stay in the tick thread's thread-local.
//!
//! Because the UI thread must not touch the presenter, `surfaceDestroyed` does not
//! drop it; it clears the platform slot and the next tick (≤16 ms) releases the
//! presenter **before** it stops presenting. The presenter holds its own
//! `ANativeWindow` reference meanwhile, so the pointer stays valid across that
//! window and the buffer queue's release cannot free it under the swapchain.
//!
//! # Ordering (frozen)
//!
//! A `NativeWindow` must outlive every GPU object that targets it, and the render
//! session must be detached **before** the device it borrowed disappears. Both are
//! enforced structurally:
//!
//! 1. [`LiveSurface`]'s `Drop` detaches the session first, before any field is
//!    dropped — so this holds on every release path *and* when the tick thread
//!    exits and the thread-local `LIVE` is dropped by its TLS destructor (which
//!    never calls [`release`]);
//! 2. [`LiveSurface`] declares `presenter` before `window`, so after `Drop` the
//!    fields are released in that order (Rust drops fields in declaration order) and
//!    the presenter goes before the window it targets;
//! 3. every explicit release path ([`release`], a replaced surface, a lost device)
//!    goes through that same `Drop`, and only then lets the platform's own
//!    `NativeWindow` reference go.
//!
//! # Pause / resume
//!
//! `surfaceDestroyed` clears the platform slot; the tick releases the presenter on
//! its next pass (a hidden surface must not keep drawing, and Android destroys the
//! buffer queue when the window is hidden). A new `surfaceCreated` produces a new
//! handle and the next tick builds a fresh presenter and re-attaches the session —
//! the session itself (the document) is untouched, so a rotation or a
//! background/foreground cycle never loses the drawing.
//!
//! # Refusals (never silent)
//!
//! * no usable surface / zero-size frame: no presenter, no frame, explicit
//!   `awaiting` state ([`SurfaceStep::Awaiting`]);
//! * no Vulkan adapter, or a **software** Vulkan adapter (SwiftShader/lavapipe):
//!   the stable marker [`crate::policy::SOFTWARE_VULKAN_MARKER`] (or the adapter
//!   error) is logged and [`SurfaceStep::Refused`] is returned; the refusal is
//!   remembered per surface creation so it is logged once, not every tick;
//! * `Lost`/`Outdated`/`Timeout`/`Occluded`/`Validation` from the swapchain:
//!   classified ([`FrameOutcome`]) instead of being retried blindly — `Lost`
//!   rebuilds, `Outdated` waits for the platform's resized extent (and retries the
//!   configure), `Timeout`/`Occluded` wait (a hidden surface is not an error), and
//!   a `Validation` failure is an explicit present error.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Mutex;

use cad_engine::render::RenderSessionHandle;
use cad_render_wgpu::gpu_time::GpuTimeConfig;
use cad_render_wgpu::{
    NativeTextureKey, ProbeBackendPolicy, ProbeError, ProbePresentMode, SurfaceProbePresenter,
};
use raw_window_handle::{
    AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle,
};

use crate::policy::{self, PresentClass};

/// Whether a presenter written for surface `created` was refused (and why).
///
/// Keyed by the creation counter, so a refusal is reported once per surface and a
/// new `surfaceCreated` gets a fresh attempt (a new device may be a real GPU).
type Refusal = Option<(i64, u64, String)>;

/// The surface the platform currently reports (`surfaceCreated`/`surfaceResized`).
#[derive(Default)]
struct PlatformSurface {
    /// Opaque handle returned to Kotlin; `0` means "no live surface".
    handle: i64,
    /// The owning `ANativeWindow` for `handle`.
    window: Option<ndk::native_window::NativeWindow>,
    /// Current buffer size (physical pixels) as reported by the platform.
    width: u32,
    height: u32,
    /// Monotonic creation counter (distinguishes two surfaces that happen to
    /// reuse the same JNI handle value after a wrap).
    created: u64,
}

/// The live presenter for one platform surface.
///
/// Field order is load-bearing: `presenter` is dropped before `window`, so the
/// swapchain goes before the buffer queue it targets. The attached session is
/// detached **first** by [`LiveSurface`]'s `Drop`, before any field is dropped —
/// that covers the explicit release paths *and* the tick thread's exit, where the
/// thread-local is dropped by the TLS destructor with no `release()` call.
struct LiveSurface {
    presenter: SurfaceProbePresenter,
    window: ndk::native_window::NativeWindow,
    /// The render session attached to this presenter's device (cheap `Rc` clone).
    session: Option<RenderSessionHandle>,
    handle: i64,
    created: u64,
    /// Presenter generation (incremented on every successful creation).
    generation: u64,
    /// The measurement-only GPU frame-time opt-in of this generation.
    gpu_time: GpuTimeConfig,
}

impl Drop for LiveSurface {
    /// Detach the borrowed device, then drop the fields (`presenter`, `window`).
    ///
    /// The session holds a device/queue borrowed from the presenter's wgpu device,
    /// so it must be detached while that device still exists. Doing it in `Drop`
    /// (instead of only in an explicit `release`) is what makes the thread-exit
    /// path correct: `LIVE` is a thread-local, and the last tick thread's exit drops
    /// it without calling [`release`].
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            session.detach_device();
        }
    }
}

/// Monotonic counters: the JNI handle, the surface creation id and the presenter
/// generation. The first two are allocated by the UI thread (JNI), the generation by
/// the tick thread, hence one small mutex instead of two thread-locals.
#[derive(Default)]
struct Counters {
    handle: i64,
    created: u64,
    generation: u64,
}

/// What the platform currently reports (`surfaceCreated`/`surfaceResized`/
/// `surfaceDestroyed`). Written on the Android UI thread, read on the tick thread.
static PLATFORM: Mutex<PlatformSurface> = Mutex::new(PlatformSurface {
    handle: 0,
    window: None,
    width: 0,
    height: 0,
    created: 0,
});

/// The shared counters (see [`Counters`]).
static COUNTERS: Mutex<Counters> = Mutex::new(Counters {
    handle: 0,
    created: 0,
    generation: 0,
});

thread_local! {
    /// The live presenter: created, used and dropped only on the tick thread.
    static LIVE: RefCell<Option<LiveSurface>> = const { RefCell::new(None) };
    /// The last refusal, keyed by surface creation (reported once, not per tick).
    static REFUSED: RefCell<Refusal> = const { RefCell::new(None) };
}

/// Lock the platform slot, recovering from a poisoned mutex.
fn platform_lock() -> std::sync::MutexGuard<'static, PlatformSurface> {
    PLATFORM
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Result of the tick's "make sure a presenter exists" step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceStep {
    /// No live platform surface: nothing can be presented (paused, not an error).
    Awaiting,
    /// A presenter is ready for `generation` at `width`x`height` (physical px).
    Ready {
        width: u32,
        height: u32,
        generation: u64,
    },
    /// The surface exists but no presenter may draw into it: explicit refusal.
    Refused { reason: String },
}

/// Result of one frame attempt.
#[derive(Debug)]
pub enum FrameOutcome {
    /// Nothing was presented this tick: no surface, no bound session, no new
    /// frame, or a transient acquire failure. Never a fabricated frame.
    Wait,
    /// A CAD frame was presented.
    Presented {
        frame_index: u64,
        timings: cad_render_wgpu::ProbeStageTimings,
    },
    /// The presenter/device must be rebuilt (device or surface lost). The session
    /// stays authoritative; the next tick builds a new presenter and re-attaches.
    Rebuild { reason: String },
    /// Explicit frame failure; `class` is the stable evidence class.
    Failed { class: PresentClass, text: String },
}

/// The size the JNI layer last reported, if a surface is live.
pub fn surface_size() -> Option<(u32, u32)> {
    LIVE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|live| live.presenter.surface_size())
    })
}

/// The live presenter generation (changes on every successful creation).
pub fn generation() -> Option<u64> {
    LIVE.with(|cell| cell.borrow().as_ref().map(|live| live.generation))
}

/// The generation whose device the session is attached to, if any.
pub fn bound_generation() -> Option<u64> {
    LIVE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|live| live.session.as_ref().map(|_| live.generation))
    })
}

/// The adapter description of the live presenter (`backend`, `adapter`).
pub fn adapter_info() -> Option<(String, String)> {
    LIVE.with(|cell| {
        cell.borrow().as_ref().map(|live| {
            let info = live.presenter.adapter();
            (info.backend.clone(), info.name.clone())
        })
    })
}

/// The present mode the live presenter chose (evidence token).
pub fn present_mode() -> Option<String> {
    LIVE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|live| format!("{:?}", live.presenter.present_mode()))
    })
}

/// Release the presenter (and detach the session) without forgetting the surface.
///
/// Idempotent. Called by `surfaceDestroyed`, by the exit handlers and whenever a
/// new surface replaces the old one — always on the tick thread.
pub fn release() {
    let taken = LIVE.with(|cell| cell.borrow_mut().take());
    if let Some(live) = taken {
        let generation = live.generation;
        // The `ANativeWindow`'s own buffer size is read before it is released: it is
        // the extent the swapchain was targeting (evidence for the released line).
        let window_size = (live.window.width(), live.window.height());
        // `Drop` detaches the session, then releases the presenter before the window.
        drop(live);
        log::info!(
            "native CAD surface released (presenter generation {generation}, window={}x{})",
            window_size.0,
            window_size.1
        );
    }
}

/// Make sure a presenter exists for the current platform surface.
///
/// Creates (or rebuilds) one when the surface is new, resizes it to the current
/// extent, and returns its size on the way back. A refusal is emitted **once** per
/// surface creation with the stable marker, never repeated every tick.
///
/// Must be called on the **tick thread**: the presenter is thread-local (`!Send`
/// through the render session), and only the tick owns the GPU objects.
pub fn ensure_presenter(gpu_time: GpuTimeConfig) -> SurfaceStep {
    let platform = {
        let surface = platform_lock();
        match (surface.handle, surface.window.clone()) {
            (0, _) | (_, None) => None,
            (handle, Some(window)) => Some((
                handle,
                surface.created,
                window,
                surface.width,
                surface.height,
            )),
        }
    };
    let Some((handle, created, window, width, height)) = platform else {
        // No platform surface: pause. There is nothing to refuse — the carrier is
        // simply waiting for the `SurfaceView` (a state, not an error).
        release();
        return SurfaceStep::Awaiting;
    };
    if width == 0 || height == 0 {
        // A zero-size frame is not presentable (and Vulkan refuses it): wait for
        // the next `surfaceChanged`, exactly like a minimized desktop window.
        release();
        return SurfaceStep::Awaiting;
    }
    // A refusal already recorded for this exact surface is reported unchanged.
    if let Some((refused_handle, refused_created, reason)) =
        REFUSED.with(|cell| cell.borrow().clone())
    {
        if refused_handle == handle && refused_created == created {
            return SurfaceStep::Refused { reason };
        }
    }

    let existing = LIVE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|live| (live.handle, live.created, live.generation))
    });
    if let Some((live_handle, live_created, generation)) = existing {
        if live_handle == handle && live_created == created {
            let resized = LIVE.with(|cell| {
                let mut slot = cell.borrow_mut();
                let Some(live) = slot.as_mut() else {
                    return Ok(());
                };
                if live.presenter.surface_size() == (width, height) {
                    return Ok(());
                }
                live.presenter.resize(width, height).map_err(|error| {
                    format!("surface reconfigure to {width}x{height} failed: {error}")
                })
            });
            return match resized {
                Ok(()) => SurfaceStep::Ready {
                    width,
                    height,
                    generation,
                },
                Err(reason) => SurfaceStep::Refused { reason },
            };
        }
    }

    // A new surface: release the old presenter (session detach first) before the
    // old `NativeWindow` reference goes away.
    release();

    let display = RawDisplayHandle::Android(AndroidDisplayHandle::new());
    let window_handle =
        RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(window.ptr().cast::<c_void>()));
    // SAFETY: the `NativeWindow` is alive and owned by this thread-local (and the
    // `LiveSurface` keeps its own reference), the surface identifies this view's
    // buffer queue, and presenter creation/destruction happen on the tick thread
    // (see the module docs). The renderer is the single presentation owner.
    let presenter = match unsafe {
        SurfaceProbePresenter::new_from_raw_handles(
            display,
            window_handle,
            width,
            height,
            // `Fifo` is the only mode wgpu guarantees on every backend and the
            // only v-synced product mode on Android (no mailbox/tearing).
            ProbePresentMode::Fifo,
            gpu_time,
            // Vulkan only: there is no GLES fallback and no `auto` relaxation on
            // this carrier.
            ProbeBackendPolicy::Vulkan,
        )
    } {
        Ok(presenter) => presenter,
        Err(error) => {
            let reason =
                format!("no Vulkan presenter could be created for {width}x{height}: {error}");
            // Stable evidence marker first (same prefix as the software-adapter
            // refusal): `logcat` tooling classifies GPU refusals on this line, and a
            // free-form line alone is not machine-readable.
            log::error!("{}", policy::unsupported_gpu_marker("no-vulkan-adapter"));
            log::error!("[probe] android surface presenter FAILED: {reason}");
            remember_refusal(handle, created, &reason);
            return SurfaceStep::Refused { reason };
        }
    };

    // Hardware-only contract: a software Vulkan adapter (emulator SwiftShader or
    // lavapipe) is a refusal with its own stable marker, never a result.
    let info = presenter.adapter().clone();
    if policy::is_software_adapter(&info.device_type, &info.name) {
        log::error!("{}", policy::SOFTWARE_VULKAN_MARKER);
        log::error!(
            "[probe] android surface refused: adapter={} device_type={} backend={} is software rendering",
            info.name,
            info.device_type,
            info.backend
        );
        drop(presenter);
        let reason = format!(
            "software Vulkan adapter refused ({}, device_type={}): Android v1 requires a hardware \
             Vulkan GPU",
            info.name, info.device_type
        );
        remember_refusal(handle, created, &reason);
        return SurfaceStep::Refused { reason };
    }

    let generation = next_generation();
    log::info!(
        "[probe] android surface presenter ready generation={generation} backend={} adapter={} device_type={} present_mode=fifo size={width}x{height}",
        info.backend,
        info.name,
        info.device_type,
    );
    LIVE.with(|cell| {
        *cell.borrow_mut() = Some(LiveSurface {
            presenter,
            window,
            session: None,
            handle,
            created,
            generation,
            gpu_time,
        });
    });
    REFUSED.with(|cell| *cell.borrow_mut() = None);
    SurfaceStep::Ready {
        width,
        height,
        generation,
    }
}

/// Attach `session` to the live presenter's device (one exact generation).
///
/// A failed attach fails the render session's device (no clean frame can follow)
/// and is returned verbatim; the tick records it against this generation, so the
/// bind is not retried every tick. Tick thread only.
pub fn bind_session(session: &RenderSessionHandle) -> Result<u64, String> {
    LIVE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(live) = slot.as_mut() else {
            return Err("no native surface is live".into());
        };
        if let Some(previous) = live.session.take() {
            previous.detach_device();
        }
        live.presenter.clear_binding_cache();
        let (device, queue) = live.presenter.device_queue();
        match session.attach_device_with_gpu_time(device, queue, live.gpu_time) {
            Ok(()) => {
                live.session = Some(session.clone());
                Ok(live.generation)
            }
            Err(error) => {
                session.fail_device(format!("attach_device: {error}"), false);
                Err(format!("render session attach failed: {error}"))
            }
        }
    })
}

/// Present one CAD frame through the bound session, at the platform surface size.
///
/// Tick thread only (the presenter and the session both live there).
///
/// The drawable is the `SurfaceView`, so the viewport is its **own** logical size
/// (`physical / scale`), never a DOM-derived guess: the platform surface is the
/// pixel authority and the renderer refuses to stretch a mismatched texture. The
/// camera is therefore 1:1 with the pixels that are actually on screen.
pub fn frame(scale: f64) -> FrameOutcome {
    let scale = scale.max(0.1);
    LIVE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(live) = slot.as_mut() else {
            return FrameOutcome::Wait;
        };
        let Some(session) = live.session.clone() else {
            return FrameOutcome::Wait;
        };
        let (width, height) = live.presenter.surface_size();
        if width == 0 || height == 0 {
            return FrameOutcome::Wait;
        }
        let logical = [f64::from(width) / scale, f64::from(height) / scale];
        if let Err(error) = session.apply_ready_updates() {
            // A GPU-memory refusal keeps the device (the resident scene is
            // intact); every other failure fails the device so no clean frame can
            // follow. Either way the presenter's cached binding is stale.
            let refused_for_memory = session.gpu_budget_refusal().is_some();
            if !refused_for_memory {
                session.fail_device(format!("apply_ready_updates: {error:?}"), false);
            }
            live.presenter.clear_binding_cache();
            let class = if refused_for_memory {
                PresentClass::GpuMemory
            } else {
                PresentClass::Render
            };
            return FrameOutcome::Failed {
                class,
                text: format!("apply_ready_updates failed: {error:?}"),
            };
        }
        let presented = match session.render(logical, scale) {
            Ok(presented) => presented,
            Err(error) => {
                if error.is_device_loss() {
                    session.fail_device(format!("render device lost: {error}"), true);
                    return FrameOutcome::Rebuild {
                        reason: format!("render device lost: {}", error.message()),
                    };
                }
                live.presenter.clear_binding_cache();
                return FrameOutcome::Failed {
                    class: PresentClass::Render,
                    text: format!("render failed: {error}"),
                };
            }
        };
        let Some(frame) = presented else {
            // No new frame and no reusable retained frame: wait. Never replay a
            // stale frame and never draw a blank/fake grid.
            return FrameOutcome::Wait;
        };
        let key = NativeTextureKey {
            device_epoch: frame.binding.device_epoch,
            texture_revision: frame.binding.texture_revision,
            size: frame.binding.size,
        };
        match live.presenter.present_texture(&frame.texture, key) {
            Ok(stats) => FrameOutcome::Presented {
                frame_index: stats.frame_index,
                timings: stats.timings,
            },
            // A bounded size disagreement (a mid-resize tick): wait, never stretch.
            Err(ProbeError::TextureNotReady(_)) => FrameOutcome::Wait,
            Err(ProbeError::DeviceLost(message)) => FrameOutcome::Rebuild {
                reason: format!("surface lost: {message}"),
            },
            // The platform extent moved under the presenter. The `surfaceChanged`
            // callback carries the new size; reconfigure to it now (a no-op when it
            // already matches) and wait for the next tick.
            Err(ProbeError::Acquire(kind @ ("outdated" | "validation"))) => {
                let (platform_width, platform_height) = {
                    let surface = platform_lock();
                    (surface.width, surface.height)
                };
                if platform_width != 0 && platform_height != 0 {
                    let _ = live.presenter.resize(platform_width, platform_height);
                }
                FrameOutcome::Failed {
                    class: PresentClass::Acquire,
                    text: format!("surface acquire failed: {kind} (waiting for the platform size)"),
                }
            }
            // A hidden/occluded surface or a transient timeout is not an error: the
            // next tick retries once the platform reports a frame.
            Err(ProbeError::Acquire("timeout" | "occluded")) => FrameOutcome::Wait,
            Err(error) => FrameOutcome::Failed {
                class: PresentClass::Present,
                text: format!("present_texture failed: {error}"),
            },
        }
    })
}

fn remember_refusal(handle: i64, created: u64, reason: &str) {
    REFUSED.with(|cell| *cell.borrow_mut() = Some((handle, created, reason.to_string())));
}

/// Allocate the next JNI handle and its surface creation counter (UI thread).
fn next_handle() -> (i64, u64) {
    let mut counters = COUNTERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    counters.handle = counters.handle.wrapping_add(1).max(1);
    counters.created = counters.created.wrapping_add(1);
    (counters.handle, counters.created)
}

/// Allocate the next presenter generation (tick thread).
fn next_generation() -> u64 {
    let mut counters = COUNTERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    counters.generation = counters.generation.wrapping_add(1);
    counters.generation
}

// ---------------------------------------------------------------------------
// Frozen JNI surface contract (Kotlin `dev.yacr.app.MainActivity`)
//
// All four symbols are defined once, with the exact names the Kotlin lane links;
// `policy::JNI_SYMBOLS` is the tested copy of that list. These three arrive on the
// **Android UI thread** (the `SurfaceHolder` callbacks), so they only acquire/release
// the platform object and publish one small `Mutex<PlatformSurface>`; no GPU object
// and no `CadSession` is touched here. The tick thread does that.
// ---------------------------------------------------------------------------

/// `nativeSurfaceCreated(surface, width, height): Long` — publish a new `Surface`.
///
/// Returns the opaque handle (`0` = failure: no `ANativeWindow`, or an empty
/// frame). Kotlin destroys the previous surface first when it re-creates one; a
/// replacement without a destroy simply supersedes the slot, and the tick thread
/// then releases the old presenter before building the new one.
#[no_mangle]
pub extern "system" fn Java_dev_yacr_app_MainActivity_nativeSurfaceCreated<'local>(
    env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    surface: jni::objects::JObject<'local>,
    width: jni::sys::jint,
    height: jni::sys::jint,
) -> jni::sys::jlong {
    if width <= 0 || height <= 0 {
        log::error!(
            "nativeSurfaceCreated: unusable surface frame {width}x{height}; no handle returned"
        );
        return 0;
    }
    // SAFETY: `env` is a valid JNI environment and `surface` a live
    // `android.view.Surface` (the holder callback guarantees both); the resulting
    // `NativeWindow` owns a reference, so it stays valid after the call returns
    // and is released only after the presenter that targets it is dropped.
    let window = unsafe {
        ndk::native_window::NativeWindow::from_surface(env.get_native_interface(), surface.as_raw())
    };
    let Some(window) = window else {
        log::error!("nativeSurfaceCreated: ANativeWindow_fromSurface returned null");
        return 0;
    };
    // Publish the new surface. A re-created surface without an intervening destroy
    // simply replaces the slot: the tick thread sees the new `created` id, releases
    // the old presenter (its own `ANativeWindow` reference keeps this window valid)
    // and builds a new one. No GPU object is touched on this thread.
    let (handle, created) = next_handle();
    {
        let mut surface = platform_lock();
        surface.handle = handle;
        surface.window = Some(window);
        surface.width = width as u32;
        surface.height = height as u32;
        surface.created = created;
    }
    log::info!("nativeSurfaceCreated {width}x{height} handle={handle}");
    handle
}

/// `nativeSurfaceResized(handle, width, height)` — record the new buffer size.
///
/// A handle mismatch is an explicit `logcat` warning and changes nothing: the tick
/// resizes the presenter from the platform size on the next frame.
#[no_mangle]
pub extern "system" fn Java_dev_yacr_app_MainActivity_nativeSurfaceResized<'local>(
    _env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    handle: jni::sys::jlong,
    width: jni::sys::jint,
    height: jni::sys::jint,
) {
    if width <= 0 || height <= 0 {
        log::warn!("nativeSurfaceResized: ignoring unusable frame {width}x{height}");
        return;
    }
    {
        let mut surface = platform_lock();
        if surface.handle != handle || surface.window.is_none() {
            log::warn!(
                "nativeSurfaceResized: handle={handle} does not match the live surface ({}); ignored",
                surface.handle
            );
            return;
        }
        surface.width = width as u32;
        surface.height = height as u32;
    }
    log::info!("nativeSurfaceResized {width}x{height} handle={handle}");
}

/// `nativeSurfaceDestroyed(handle)` — mark the surface gone; the tick stops.
///
/// This runs on the Android UI thread and must not touch the presenter (which
/// lives on the tick thread and is `!Send`). It clears the platform slot; the next
/// tick sees "no surface", detaches the session and drops the presenter — with the
/// presenter's own `ANativeWindow` reference still alive until it is dropped, which
/// is the frozen presenter-before-window order.
#[no_mangle]
pub extern "system" fn Java_dev_yacr_app_MainActivity_nativeSurfaceDestroyed<'local>(
    _env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    handle: jni::sys::jlong,
) {
    {
        let mut surface = platform_lock();
        if surface.handle == handle {
            // Drop this thread's `ANativeWindow` reference (the presenter keeps its
            // own until the tick releases it).
            surface.handle = 0;
            surface.window = None;
            surface.width = 0;
            surface.height = 0;
        } else {
            log::warn!(
                "nativeSurfaceDestroyed: handle={handle} does not match the live surface ({}); ignored",
                surface.handle
            );
        }
    }
    log::info!("nativeSurfaceDestroyed handle={handle}");
}

#[cfg(test)]
mod tests {
    // The source-structure assertions of this file (the JNI symbols and the
    // presenter-before-window drop order) live in `crate::contract`, which is
    // compiled on the development host — this module is `cfg(target_os = "android")`,
    // so tests declared here never run on a gate. The two below are behavioural and
    // need the Android-only presenter/JNI types; they are Android-target tests.
    use super::*;

    #[test]
    fn a_frame_without_a_surface_waits_and_never_presents_a_placeholder() {
        // No JNI callback ran in this test process, so there is no platform
        // surface: the only honest answer is `Wait` (no fabricated frame).
        assert!(matches!(frame(1.0), FrameOutcome::Wait));
        assert!(surface_size().is_none());
        assert!(generation().is_none());
        assert!(bound_generation().is_none());
    }

    #[test]
    fn a_tick_before_any_surface_awaits_instead_of_refusing() {
        assert_eq!(
            ensure_presenter(GpuTimeConfig::disabled()),
            SurfaceStep::Awaiting
        );
    }
}
