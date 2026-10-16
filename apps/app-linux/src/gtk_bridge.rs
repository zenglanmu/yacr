//! GTK overlay bridge for the React probe (Linux/X11 and native Wayland).
//!
//! **Root cause this solves:** creating the wgpu Vulkan surface on the Tauri/GTK
//! **top-level** window makes the Vulkan swapchain cover the React webview. Here
//! the probe surface instead gets its own dedicated native region inside a
//! `GtkOverlay`, with the React webview reparented as a transparent sibling on
//! top. There is **no top-level fallback**.
//!
//! Two backend-specific native paths sit behind the same owner:
//! - **X11** (`classify_native`): a windowed `GtkDrawingArea` with its own native
//!   X child window, whose Xlib XID/display are handed to the renderer. Unchanged.
//! - **Wayland** ([`WaylandSurface`]): the drawing area is a non-native GDK child
//!   that shares the GDK toplevel `wl_surface`, so the bridge owns a **child
//!   `wl_surface` + `wl_subsurface`** (the app-owned region) and hands *that* to
//!   the renderer. The GDK surface is never used for Vulkan and never committed
//!   by the host.
//!
//! Threading/lifetime:
//! - [`GtkOverlayOwner`] is `!Send`/`!Sync` (`PhantomData<Rc<()>>`): it is created,
//!   used and dropped only on the host main thread. It owns the GTK widgets and
//!   the GPU presenter, and always releases the GPU **before** the native widgets
//!   (and, on Wayland, the child surface before the GTK parent unrealizes).
//! - The GPU surface is created from raw handles
//!   ([`SurfaceProbePresenter::new_from_raw_handles`]); there is **no**
//!   `unsafe Send/Sync` provider.
//! - GTK signals that could synchronously re-enter are never called while a
//!   `RefCell` borrow is held: hooks take the borrow, drop the presenter and
//!   release it immediately.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use crate::wayland_surface::{
    is_wayland_display, EnsureChild, NativeHealth, SubsurfaceReadiness, WaylandSurface,
};
use cad_engine::render::{FrameCompleteness, RenderSessionHandle};
use cad_render_wgpu::{
    NativeTextureKey, ProbeFrameStats, ProbePresentMode, ProbeStageTimings, SurfaceProbePresenter,
};
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
    XlibDisplayHandle, XlibWindowHandle,
};

thread_local! {
    /// Overlays keyed by Tauri window label; `with_webview` closures must be
    /// `Send`, so they look the overlay up here instead of capturing GTK objects.
    static OVERLAYS: RefCell<HashMap<String, gtk::Overlay>> = RefCell::new(HashMap::new());
    /// Set once the webview has been reparented into the overlay.
    static REPARENT_DONE: Cell<bool> = const { Cell::new(false) };
}

/// Main-thread-only owner of the GTK overlay and the GPU presenter.
pub struct GtkOverlayOwner {
    _not_send_sync: PhantomData<Rc<()>>,
    top_level: gtk::ApplicationWindow,
    #[allow(dead_code)]
    vbox: gtk::Box,
    overlay: gtk::Overlay,
    drawing_area: gtk::DrawingArea,
    presenter: Option<SurfaceProbePresenter>,
    size: (u32, u32),
    /// Explicit lifecycle state; GPU init is only allowed from
    /// [`BridgeState::AwaitingMap`] / [`BridgeState::Suspended`].
    state: BridgeState,
    /// Incremented on every successful GPU (re-)init; used to discard stale
    /// layout metrics from an older generation.
    generation: u64,
    /// Verified native handles + retained GDK refs (B2), kept alive while the
    /// surface exists so the XID/display cannot be invalidated underneath it.
    native: Option<NativeHandles>,
    /// Wayland path: the app-owned child `wl_surface`/`wl_subsurface` over the
    /// borrowed GDK toplevel surface, retained for the owner's lifetime so the
    /// bound globals are not re-discovered on every generation. The child is
    /// (re)created per generation in `try_init_gpu_wayland` and released in
    /// [`Self::release_gpu`].
    wayland: Option<WaylandSurface>,
    /// Last "not ready yet" reason, so a deferral is logged once per change and
    /// never becomes a terminal failure.
    pending_reason: Option<String>,
    /// The CAD render session bound to this owner (cheap `Rc` clone), if any.
    session: Option<RenderSessionHandle>,
    /// Metadata of the last actually presented CAD frame (telemetry only).
    last_cad_frame: Option<CadFrameObservation>,
    /// Stage wall-times of the last `frame_cad` call (telemetry only).
    last_frame_stages: Option<FrameCadStages>,
}

/// Metadata of the last **actually presented** CAD frame (host telemetry only).
///
/// Only a successful `present_texture` records this; a `None` (waiting) or an
/// error never does, so a host can never infer a new camera/frame from a texture
/// revision alone. `rendered_view_revision` is the view (camera/space) revision
/// the engine actually rendered into the presented frame, independent of the
/// texture binding identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CadFrameObservation {
    /// The view revision actually rendered into the presented frame.
    pub rendered_view_revision: u64,
    /// Full current scene, or the explicit base-only navigation fallback.
    pub completeness: FrameCompleteness,
    /// Device epoch of the presented texture.
    pub device_epoch: u64,
    /// Texture revision of the presented texture.
    pub texture_revision: u64,
    /// Physical size of the presented texture.
    pub size: (u32, u32),
    /// The presenter's own present index (actual successful present count).
    pub frame_id: u64,
    /// Native presenter stage timings for this present.
    pub timings: ProbeStageTimings,
}

/// Explicit lifecycle state for the bridge (B1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeState {
    /// Waiting for the drawing area to be mapped with a real allocation.
    AwaitingMap,
    /// GPU presenter is live.
    Ready,
    /// Presenter was released (e.g. unrealize); may re-init.
    Suspended,
    /// Shutting down; never init again.
    Closing,
    /// Terminal failure (explicit; there is no fallback).
    Failed(String),
}

/// Stage wall-times of one [`GtkOverlayOwner::frame_cad`] call, in microseconds
/// (diagnostics only). `apply_us` is `apply_ready_updates`, `render_us` is the
/// engine render call (which waits/gates — it is not GPU time), `present_us` is
/// `present_texture` and is `0` when no frame was presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameCadStages {
    pub apply_us: u64,
    pub render_us: u64,
    pub present_us: u64,
    pub presented: bool,
}

/// Logical geometry of the native CAD partition, for the bounded geometry log.///
/// `container` is the GTK widget that **receives the CAD margins** (the
/// `GtkOverlay`), in logical px — the same coordinate space as the frontend's DOM
/// CAD rect, because the React WebView fills this overlay. It is the correct
/// reference bounds for the viewport insets; the Tauri window `inner_size()`
/// (which includes CSD/decorations) is **not**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutGeometry {
    /// Overlay allocation (the margin-receiving container), logical px.
    pub container: (i32, i32),
    /// Drawing area allocation, logical px.
    pub drawing_area: (i32, i32),
    /// Drawing area origin translated into the toplevel, logical px (`None` until
    /// both widgets are realized/allocated).
    pub origin: Option<(i32, i32)>,
    /// GTK integer scale factor (>= 1).
    pub scale: i32,
}

/// Install the bridge: build the overlay + windowed drawing area, realize it,
/// and register it for the webview reparent. GPU init is deferred.
pub fn install(window: &tauri::Window, label: &str) -> Result<GtkOverlayOwner, String> {
    GtkOverlayOwner::install(window, label)
}

impl GtkOverlayOwner {
    /// Build the overlay + windowed drawing area, realize it, and register the
    /// webview-reparent hook. GPU init is deferred until the drawing area is
    /// mapped with a non-zero allocation *and* the reparent has completed.
    pub fn install(window: &tauri::Window, label: &str) -> Result<Self, String> {
        let gtk_window = window
            .gtk_window()
            .map_err(|error| format!("gtk_window: {error}"))?;
        gtk_window.set_app_paintable(true);
        let vbox = window
            .default_vbox()
            .map_err(|error| format!("default_vbox: {error}"))?;

        let overlay = gtk::Overlay::new();
        let drawing_area = gtk::DrawingArea::new();
        // On X11 the swapchain needs a native X child window of its own: promote
        // the widget before realize (else it is client-side and its XID would
        // equal the parent's). On Wayland the native region is the app-owned
        // `wl_subsurface` (never a GTK child), so leave `has_window` false and
        // never call `gdk_window_ensure_native` there.
        let x11_backend = !is_wayland_display(&gtk_window.display());
        #[allow(deprecated)]
        drawing_area.set_has_window(x11_backend);
        drawing_area.set_app_paintable(true);
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        // Central CAD partition: leave the React ribbon (top) and command bar
        // (bottom) visible around the native surface.
        drawing_area.set_margin_top(101);
        drawing_area.set_margin_bottom(60);
        // Block the default class draw so GTK never paints over the surface.
        drawing_area.connect_draw(|_, _| gtk::glib::Propagation::Stop);

        overlay.add(&drawing_area);
        vbox.pack_start(&overlay, true, true, 0);
        overlay.show_all();

        drawing_area.realize();

        // B3: let OS events reach the React DOM instead of this native child.
        apply_input_passthrough(&overlay, &drawing_area);

        OVERLAYS.with(|map| {
            map.borrow_mut().insert(label.to_string(), overlay.clone());
        });
        REPARENT_DONE.with(|flag| flag.set(false));

        Ok(Self {
            _not_send_sync: PhantomData,
            top_level: gtk_window,
            vbox,
            overlay,
            drawing_area,
            presenter: None,
            size: (0, 0),
            state: BridgeState::AwaitingMap,
            generation: 0,
            native: None,
            wayland: None,
            pending_reason: None,
            session: None,
            last_cad_frame: None,
            last_frame_stages: None,
        })
    }

    /// Apply the React CAD rect as drawing-area insets (logical px == CSS px)
    /// and mark the layout confirmed so GPU init may proceed. A rect that only
    /// shifts still updates the insets.
    pub fn apply_layout(&mut self, margins: cad_ui_core::layout::ViewportInsets) {
        assert!(
            gtk::is_initialized_main_thread(),
            "gtk bridge apply_layout must run on the GTK main thread"
        );
        self.drawing_area.set_margin_top(margins.top);
        self.drawing_area.set_margin_bottom(margins.bottom);
        self.drawing_area.set_margin_start(margins.start);
        self.drawing_area.set_margin_end(margins.end);
        // Re-assert the empty input region on every (re)layout.
        apply_input_passthrough(&self.overlay, &self.drawing_area);
        // Wayland: the subsurface position follows the drawing area's logical
        // origin within the toplevel (the native region is app-owned, not a GTK
        // child window). Re-synced again per frame in `resize_to` once GTK has
        // re-allocated.
        self.sync_wayland_layout();
        eprintln!(
            "[probe] bridge layout: margins top={} bottom={} start={} end={}",
            margins.top, margins.bottom, margins.start, margins.end
        );
    }

    /// Log the native window tree / map state / surface identity (diagnostics).
    pub fn log_tree(&self) {
        eprintln!(
            "[probe] bridge tree: top_level_has_window={} drawing_area_has_window={}",
            self.top_level.has_window(),
            self.drawing_area.has_window(),
        );
        if self.display_is_wayland() {
            let wl_display = self
                .wayland
                .as_ref()
                .map(|wayland| wayland.display_ptr() as usize)
                .unwrap_or(0);
            let child = self
                .wayland
                .as_ref()
                .and_then(|wayland| wayland.child_surface())
                .map(|surface| surface.as_ptr() as usize)
                .unwrap_or(0);
            let parent = self
                .top_level
                .window()
                .and_then(|window| crate::wayland_surface::toplevel_surface_ptr(&window))
                .map(|surface| surface.as_ptr() as usize)
                .unwrap_or(0);
            eprintln!(
                "[probe] bridge window (wayland): wl_display={wl_display:#x} parent_wl_surface={parent:#x} child_wl_surface={child:#x} alloc={}x{} scale={}",
                self.drawing_area.allocated_width(),
                self.drawing_area.allocated_height(),
                self.drawing_area.scale_factor(),
            );
            return;
        }
        if let Some(win) = self.drawing_area.window() {
            let xid = win
                .clone()
                .downcast::<gdkx11::X11Window>()
                .ok()
                .map(|x| format!("0x{:x}", x.xid()))
                .unwrap_or_else(|| "non-x11".into());
            let top_xid = self
                .top_level
                .window()
                .and_then(|w| w.downcast::<gdkx11::X11Window>().ok())
                .map(|x| format!("0x{:x}", x.xid()))
                .unwrap_or_else(|| "none".into());
            eprintln!(
                "[probe] bridge window: xid={xid} top_xid={top_xid} mapped={} size={}x{} scale={} alloc={}x{}",
                win.is_visible(),
                win.width(),
                win.height(),
                win.scale_factor(),
                self.drawing_area.allocated_width(),
                self.drawing_area.allocated_height(),
            );
        } else {
            eprintln!("[probe] bridge window: drawing area has no GdkWindow yet");
        }
    }

    /// Attempt GPU init once the surface can legally exist. Returns `Some(Ok/Err)`
    /// on the transition; `None` while still waiting.
    ///
    /// `layout_confirmed` is the single source of truth for "a valid React CAD
    /// rect has been accepted" (the host passes `LayoutTracker::confirmed`); the
    /// bridge keeps no second copy of that flag.
    ///
    /// Prerequisites that are merely **not ready yet** (widget not
    /// realized/mapped, no own GdkWindow, empty or not-steady geometry, not yet a
    /// descendant of the top level) return `None` and keep the state
    /// `AwaitingMap`/`Suspended` — they are never turned into a terminal `Failed`.
    /// Only genuinely terminal native problems (not an X11 window, wrong
    /// screen/backend, invalid handles, XID equal to the top level) fail.
    pub fn try_init_gpu(&mut self, layout_confirmed: bool) -> Option<Result<(), String>> {
        // B1: only init from AwaitingMap/Suspended. Closing/Failed never re-init.
        if !init_allowed(&self.state) {
            return None;
        }
        // Do not init before a valid React CAD rect has been confirmed.
        if !layout_confirmed {
            return None;
        }
        if self.presenter.is_some() {
            return None;
        }
        let reparented = REPARENT_DONE.with(|flag| flag.get());
        if !reparented {
            return None;
        }
        let alloc_w = self.drawing_area.allocated_width();
        let alloc_h = self.drawing_area.allocated_height();
        let scale = wayland_buffer_scale(self.drawing_area.scale_factor());
        if alloc_w <= 0 || alloc_h <= 0 {
            return None;
        }
        // Single checked logical × integer-scale rule for both backends: an
        // overflow is an explicit failure, never a wrapping/saturating success.
        let (phys_w, phys_h) = match checked_physical_size(alloc_w, alloc_h, scale) {
            Ok(size) => size,
            Err(error) => {
                self.state = BridgeState::Failed(error.clone());
                eprintln!("[probe] bridge size FAILED (terminal): {error}");
                return Some(Err(error));
            }
        };

        // Backend dispatch: the Wayland path owns an app-owned child
        // `wl_surface`/`wl_subsurface` and never promotes the drawing area to a
        // native GDK window (`gdk_window_ensure_native` is X11-only).
        if self.display_is_wayland() {
            return self.try_init_gpu_wayland(phys_w, phys_h, scale);
        }

        let handles = match classify_native(&self.drawing_area, &self.top_level) {
            NativeStatus::Ready(handles) => handles,
            NativeStatus::Pending(reason) => {
                // Transient: wait for the next tick instead of failing forever.
                return self.defer(reason);
            }
            NativeStatus::Fatal(error) => {
                self.state = BridgeState::Failed(error.clone());
                eprintln!("[probe] bridge native verification FAILED (terminal): {error}");
                return Some(Err(error));
            }
        };
        self.pending_reason = None;
        let (xid, display, screen, native_geom) =
            (handles.xid, handles.display, handles.screen, handles.geom);
        let top_xid = self
            .top_level
            .window()
            .and_then(|w| w.downcast::<gdkx11::X11Window>().ok())
            .map(|x| x.xid());
        if top_xid == Some(xid) {
            let error =
                format!("drawing-area XID 0x{xid:x} equals the top-level XID (client-side window)");
            self.state = BridgeState::Failed(error.clone());
            eprintln!("[probe] bridge FAILED: {error}");
            return Some(Err(error));
        }

        eprintln!(
            "[probe] bridge surface: xid=0x{xid:x} top_xid={} alloc={alloc_w}x{alloc_h} scale={scale} native_geom={}x{} phys={phys_w}x{phys_h}",
            top_xid.map(|x| format!("0x{x:x}")).unwrap_or_else(|| "none".into()),
            native_geom.0,
            native_geom.1,
        );
        // SAFETY: the drawing area owns a live native X child window (classified
        // above: distinct XID, realized, mapped, non-zero steady geometry, a real
        // descendant of the top level). The handles stay valid while this owner
        // lives, and the owner drops the presenter before the widgets (Drop impl
        // + unrealize hooks).
        let presenter = match unsafe {
            SurfaceProbePresenter::new_from_raw_handles(
                RawDisplayHandle::Xlib(XlibDisplayHandle::new(Some(display), screen)),
                RawWindowHandle::Xlib(XlibWindowHandle::new(xid)),
                phys_w,
                phys_h,
                ProbePresentMode::MailboxElseImmediate,
            )
        } {
            Ok(presenter) => presenter,
            Err(error) => {
                let message = format!("probe presenter init failed: {error}");
                self.state = BridgeState::Failed(message.clone());
                eprintln!("[probe] bridge presenter FAILED: {error}");
                return Some(Err(message));
            }
        };
        eprintln!(
            "[probe] bridge presenter ready generation={} backend={} adapter={} device_type={} present_mode={:?} configure_us={}",
            self.generation + 1,
            presenter.adapter().backend,
            presenter.adapter().name,
            presenter.adapter().device_type,
            presenter.present_mode(),
            presenter.last_configure_us(),
        );
        self.size = (phys_w, phys_h);
        self.presenter = Some(presenter);
        self.native = Some(handles);
        self.state = BridgeState::Ready;
        self.generation += 1;
        // Attach the render session to the new device generation: the device is
        // bound before any texture generation happens.
        if let Some(result) = self.attach_render_session() {
            return Some(result);
        }
        Some(Ok(()))
    }

    /// Whether this owner is on the GDK Wayland backend.
    fn display_is_wayland(&self) -> bool {
        is_wayland_display(&self.drawing_area.display())
    }

    /// Record a transient "not ready yet" reason once per change and defer (never
    /// a terminal failure).
    fn defer(&mut self, reason: &str) -> Option<Result<(), String>> {
        if pending_reason_changed(self.pending_reason.as_deref(), reason) {
            eprintln!("[probe] bridge waiting: {reason}");
            self.pending_reason = Some(reason.to_string());
        }
        None
    }

    /// Attach the bound render session to the just-created presenter's device.
    ///
    /// `None` means "no session / attached"; `Some(Err)` is a terminal bridge
    /// failure (the session device is failed first so no clean frame can follow).
    fn attach_render_session(&mut self) -> Option<Result<(), String>> {
        let session = self.session.clone()?;
        let (device, queue) = self
            .presenter
            .as_ref()
            .expect("presenter was just set")
            .device_queue();
        if let Err(error) = session.attach_device(device, queue) {
            session.fail_device(format!("attach_device: {error}"), false);
            let message = format!("render session attach failed: {error}");
            // Ordered release BEFORE the terminal failure: the presenter owns a
            // device that could not be bound to the session, so it must not survive
            // (this is reachable on the CAD path, not only the probe path).
            self.release_gpu();
            self.state = BridgeState::Failed(message.clone());
            eprintln!("[probe] bridge render session attach FAILED (released): {error}");
            return Some(Err(message));
        }
        eprintln!(
            "[probe] bridge: render session attached (generation {})",
            self.generation
        );
        None
    }

    /// Wayland GPU init: create the app-owned child surface over the borrowed GDK
    /// toplevel surface and build the presenter on it.
    ///
    /// Transient states (display/globals/parent surface not ready yet) defer; only
    /// genuinely terminal problems (wrong backend, missing/failed globals, a
    /// replaced parent, the distinct child/parent invariant, or presenter
    /// construction) fail explicitly.
    fn try_init_gpu_wayland(
        &mut self,
        phys_w: u32,
        phys_h: u32,
        scale: i32,
    ) -> Option<Result<(), String>> {
        assert!(
            gtk::is_initialized_main_thread(),
            "gtk bridge wayland init must run on the GTK main thread"
        );
        let Some(top) = self.top_level.window() else {
            return self.defer("top-level GdkWindow is not realized yet");
        };
        // Lazily attach the shim to the borrowed GDK Wayland display. The shim's
        // registry + discovery-sync requests are answered by GTK's own dispatch of
        // the shared queue (this never dispatches or round-trips).
        if self.wayland.is_none() {
            match WaylandSurface::attach(&top.display()) {
                Ok(wayland) => {
                    eprintln!("[probe] bridge wayland: attached to the GDK Wayland display");
                    self.wayland = Some(wayland);
                }
                Err(error) => {
                    self.state = BridgeState::Failed(error.clone());
                    eprintln!("[probe] bridge wayland FAILED (terminal): {error}");
                    return Some(Err(error));
                }
            }
        }
        let readiness = self.wayland.as_ref().expect("attached above").readiness();
        match readiness {
            SubsurfaceReadiness::Pending => {
                return self.defer("waiting for wl_compositor/wl_subcompositor discovery");
            }
            SubsurfaceReadiness::Failed(error) => {
                self.state = BridgeState::Failed(error.clone());
                eprintln!("[probe] bridge wayland FAILED (terminal): {error}");
                return Some(Err(error));
            }
            SubsurfaceReadiness::Ready {
                compositor_version,
                subcompositor_version,
            } => {
                eprintln!(
                    "[probe] bridge wayland: globals ready wl_compositor=v{compositor_version} wl_subcompositor=v{subcompositor_version}"
                );
            }
        }
        // The CAD region's logical origin inside the toplevel: on Wayland the
        // drawing area is a non-native GDK child, so the app-owned subsurface is
        // what actually occupies that rectangle.
        let translation = self
            .drawing_area
            .translate_coordinates(&self.top_level, 0, 0);
        let Some((x, y)) = translation else {
            return self.defer("drawing area has no translation to the toplevel yet");
        };
        let ensure_result = {
            let wayland = self.wayland.as_mut().expect("attached above");
            wayland.ensure_child(&top, x, y, scale)
        };
        match ensure_result {
            Ok(EnsureChild::Created) | Ok(EnsureChild::Current) => {}
            Ok(EnsureChild::ParentReplaced) => {
                // No presenter exists on this (init) path, so a stale child from a
                // replaced parent can be safely destroyed and re-created once.
                self.release_wayland_child();
                let retry = {
                    let wayland = self.wayland.as_mut().expect("attached above");
                    wayland.ensure_child(&top, x, y, scale)
                };
                if let Err(error) = retry {
                    self.state = BridgeState::Failed(error.clone());
                    eprintln!("[probe] bridge wayland FAILED (terminal): {error}");
                    return Some(Err(error));
                }
            }
            Err(error) => {
                self.state = BridgeState::Failed(error.clone());
                eprintln!("[probe] bridge wayland FAILED (terminal): {error}");
                return Some(Err(error));
            }
        }
        let child_and_display = {
            let wayland = self.wayland.as_ref().expect("attached above");
            wayland
                .child_surface()
                .map(|child| (child, wayland.display_ptr()))
        };
        let Some((child, display_ptr)) = child_and_display else {
            return self.defer("child wl_surface is not created yet");
        };
        self.pending_reason = None;

        let Some(display_handle) = NonNull::new(display_ptr as *mut c_void) else {
            let error = "borrowed wl_display is null".to_string();
            self.state = BridgeState::Failed(error.clone());
            eprintln!("[probe] bridge wayland FAILED (terminal): {error}");
            return Some(Err(error));
        };
        let Some(surface_handle) = NonNull::new(child.as_ptr() as *mut c_void) else {
            let error = "child wl_surface is null".to_string();
            self.state = BridgeState::Failed(error.clone());
            eprintln!("[probe] bridge wayland FAILED (terminal): {error}");
            return Some(Err(error));
        };
        eprintln!(
            "[probe] bridge wayland surface: wl_display={:p} child_wl_surface={:p} pos=({x},{y}) scale={scale} phys={phys_w}x{phys_h}",
            display_ptr,
            child.as_ptr(),
        );

        // This boundary spans the **whole presenter constructor**. wgpu-hal 30
        // panics (`expect("WaylandSurface failed")`) when `vkCreateWaylandSurfaceKHR`
        // returns an error instead of surfacing `Err`. Any panic inside the
        // constructor unwinds its own locals before we release the child surface, and
        // its bounded payload is preserved in the diagnostic. Terminal: the state
        // becomes `Failed` and is never retried. Only effective under the default
        // unwind strategy (no `panic = "abort"`); deliberately not a general panic
        // net.
        let created = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            SurfaceProbePresenter::new_from_raw_handles(
                RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display_handle)),
                RawWindowHandle::Wayland(WaylandWindowHandle::new(surface_handle)),
                phys_w,
                phys_h,
                ProbePresentMode::MailboxElseImmediate,
            )
        }));
        let presenter = match created {
            Ok(Ok(presenter)) => presenter,
            Ok(Err(error)) => {
                let message = format!("probe presenter init failed: {error}");
                self.release_wayland_child();
                self.state = BridgeState::Failed(message.clone());
                eprintln!("[probe] bridge wayland presenter FAILED: {error}");
                return Some(Err(message));
            }
            Err(payload) => {
                let message = format!(
                    "presenter construction panicked: {}",
                    panic_payload_message(payload.as_ref())
                );
                // The constructor's locals have already unwound; now release the
                // child surface so it cannot outlive the failed presenter.
                self.release_wayland_child();
                self.state = BridgeState::Failed(message.clone());
                eprintln!("[probe] bridge wayland {message} (terminal)");
                return Some(Err(message));
            }
        };
        eprintln!(
            "[probe] bridge wayland presenter ready generation={} backend={} adapter={} device_type={} present_mode={:?} configure_us={}",
            self.generation + 1,
            presenter.adapter().backend,
            presenter.adapter().name,
            presenter.adapter().device_type,
            presenter.present_mode(),
            presenter.last_configure_us(),
        );
        self.size = (phys_w, phys_h);
        self.presenter = Some(presenter);
        self.state = BridgeState::Ready;
        self.generation += 1;
        // The sub-surface position is parent-relative double-buffered state and is
        // applied on the **parent's** next commit: request a GTK toplevel
        // `queue_draw` so GTK commits it. We never commit the GDK surface ourselves.
        self.top_level.queue_draw();
        if let Some(result) = self.attach_render_session() {
            return Some(result);
        }
        Some(Ok(()))
    }

    /// Sync the Wayland subsurface's logical position / integer buffer scale to
    /// the drawing area's current toplevel-relative origin.
    ///
    /// No-op on X11 or before the child exists. `set_position` is parent-relative
    /// double-buffered state, so when it actually changes this requests a GTK
    /// toplevel `queue_draw` (GTK commits the parent; the app never commits the GDK
    /// surface itself). Re-synced every frame from [`Self::resize_to`].
    fn sync_wayland_layout(&mut self) {
        if self.wayland.is_none() {
            return;
        }
        let scale = wayland_buffer_scale(self.drawing_area.scale_factor());
        let Some((x, y)) = self
            .drawing_area
            .translate_coordinates(&self.top_level, 0, 0)
        else {
            return;
        };
        let changed = {
            let wayland = self.wayland.as_mut().expect("checked above");
            match wayland.update_layout(x, y, scale) {
                Ok(changed) => changed,
                Err(error) => {
                    eprintln!("[probe] bridge wayland layout update deferred: {error}");
                    return;
                }
            }
        };
        if !changed {
            return;
        }
        // Re-assert the empty input region when the geometry changed (the child is
        // created with it); never per frame.
        if let Some(wayland) = self.wayland.as_mut() {
            if let Err(error) = wayland.reassert_empty_input() {
                eprintln!("[probe] bridge wayland input-region reassert deferred: {error}");
            }
        }
        self.top_level.queue_draw();
    }

    /// Common pre-presentation native-health check (Wayland only).
    ///
    /// Verifies the shim/protocol is still ready and the GDK parent `wl_surface`
    /// has not been replaced, then performs an **ordered release** (presenter
    /// first, then the child surface) so a stale frame is never submitted. A
    /// replaced parent leaves the owner `Suspended` for re-creation on the next
    /// tick; a shim/protocol failure is terminal (`Failed`). `Ok(())` on X11 or
    /// before a presenter exists.
    fn enforce_wayland_health(&mut self) -> Result<(), String> {
        if self.presenter.is_none() || self.wayland.is_none() {
            return Ok(());
        }
        let health = {
            let current = self
                .top_level
                .window()
                .and_then(|window| crate::wayland_surface::toplevel_surface_ptr(&window));
            self.wayland
                .as_ref()
                .expect("checked above")
                .health(current)
        };
        match health {
            NativeHealth::Healthy => Ok(()),
            NativeHealth::ParentReplaced => {
                // Ordered release: presenter, then child. State becomes Suspended.
                self.release_gpu();
                let message =
                    "wayland GDK parent surface replaced; released for re-create".to_string();
                eprintln!("[probe] bridge wayland: {message}");
                Err(message)
            }
            NativeHealth::ShimFailed(reason) => {
                self.release_gpu();
                self.state = BridgeState::Failed(reason.clone());
                eprintln!("[probe] bridge wayland FAILED (terminal, released): {reason}");
                Err(reason)
            }
        }
    }

    /// Release the app-owned Wayland child surface/subsurface, if any.
    fn release_wayland_child(&mut self) {
        let Some(wayland) = self.wayland.as_mut() else {
            return;
        };
        if wayland.child_surface().is_some() {
            wayland.release_child();
            eprintln!(
                "[probe] bridge: wayland child surface released (before GTK parent unrealize)"
            );
        }
    }

    /// Force the overlay to unrealize (evidence-only lifecycle driver). Runs the
    /// registered unrealize hook, which releases the GPU **before** the native
    /// widgets are destroyed. Never called on a normal product path.
    pub fn force_unrealize(&mut self) {
        self.overlay.unrealize();
        eprintln!(
            "[probe] bridge: forced overlay unrealize -> state={:?} generation={}",
            self.state, self.generation
        );
    }

    /// Re-realize and re-map the overlay after a forced unrealize, re-asserting
    /// the input pass-through so the next tick can create a new GPU generation.
    pub fn force_realize(&mut self) {
        // `realize()` alone does not map; re-map the overlay (and the native
        // drawing area) so the next tick can classify a steady native window.
        self.overlay.show_all();
        self.overlay.realize();
        self.overlay.map();
        self.drawing_area.realize();
        self.drawing_area.map();
        apply_input_passthrough(&self.overlay, &self.drawing_area);
        eprintln!(
            "[probe] bridge: forced overlay realize/map -> state={:?} generation={}",
            self.state, self.generation
        );
    }

    /// The presenter's current surface size, if initialized.
    pub fn surface_size(&self) -> Option<(u32, u32)> {
        self.presenter.as_ref().map(|p| p.surface_size())
    }

    /// Adapter description, if initialized.
    pub fn adapter(&self) -> Option<&cad_render_wgpu::ProbeAdapterInfo> {
        self.presenter.as_ref().map(|p| p.adapter())
    }

    /// Present mode, if initialized.
    pub fn present_mode(&self) -> Option<cad_render_wgpu::wgpu::PresentMode> {
        self.presenter.as_ref().map(|p| p.present_mode())
    }

    /// Resize the surface (allocation * scale).
    pub fn resize_to(&mut self, phys_w: u32, phys_h: u32) -> Result<(), String> {
        // Pre-presentation native-health check (Wayland): a released/replaced
        // surface is never resized or presented on.
        self.enforce_wayland_health()?;
        // Keep the Wayland subsurface's logical position/scale in step with the
        // drawing area even when the physical size is unchanged (a pure layout
        // shift), before the size short-circuit below.
        self.sync_wayland_layout();
        let Some(presenter) = self.presenter.as_mut() else {
            return Ok(());
        };
        if presenter.surface_size() == (phys_w, phys_h) {
            return Ok(());
        }
        presenter
            .resize(phys_w, phys_h)
            .map_err(|error| format!("probe resize failed: {error}"))?;
        self.size = (phys_w, phys_h);
        Ok(())
    }

    /// Render one frame.
    pub fn frame(
        &mut self,
        marker: f32,
    ) -> Option<Result<(u64, cad_render_wgpu::ProbeStageTimings), String>> {
        // Native health before presenting (Wayland); a released surface yields an
        // explicit error instead of a stale submit.
        if let Err(error) = self.enforce_wayland_health() {
            return Some(Err(error));
        }
        let presenter = self.presenter.as_mut()?;
        Some(match presenter.frame(marker) {
            Ok(stats) => Ok((stats.frame_index, stats.timings)),
            Err(error) => Err(format!("probe frame failed: {error}")),
        })
    }

    /// Bind (or clear) the CAD render session for this owner.
    ///
    /// A live presenter is first detached from the old session and the retained
    /// frame cache is cleared, then the new session is attached to the current
    /// device via [`SurfaceProbePresenter::device_queue`]. With no presenter the
    /// session is stored and attached on the next successful `try_init_gpu`.
    pub fn set_render_session(
        &mut self,
        session: Option<RenderSessionHandle>,
    ) -> Result<(), String> {
        if let Some(previous) = self.session.take() {
            previous.detach_device();
        }
        // Replacing the session invalidates any cached native-texture binding.
        self.clear_presenter_cache();
        let Some(session) = session else {
            return Ok(());
        };
        if let Some(presenter) = self.presenter.as_ref() {
            let (device, queue) = presenter.device_queue();
            if let Err(error) = session.attach_device(device, queue) {
                session.fail_device(format!("attach_device: {error}"), false);
                return Err(format!("render session attach failed: {error}"));
            }
        }
        self.session = Some(session);
        Ok(())
    }

    /// Draw one CAD frame through the bound render session and present it.
    ///
    /// Requires an attached session and a live presenter. `Ok(None)` means the
    /// engine has no new frame **and** no reusable retained frame: the caller
    /// waits — no blank or fake grid is drawn. Frame-state failures are explicit;
    /// a render error marks the session device as failed and does not loop.
    pub fn frame_cad(
        &mut self,
        logical: [f64; 2],
        scale: f64,
    ) -> Result<Option<ProbeFrameStats>, String> {
        // Native health before presenting (Wayland); a released/replaced surface
        // is never drawn into.
        self.enforce_wayland_health()?;
        let Some(session) = self.session.clone() else {
            return Err("no render session attached".into());
        };
        if self.presenter.is_none() {
            return Err("no GPU presenter (bridge not ready)".into());
        }
        // `apply_ready_updates` is a CadError: any failure fails the session
        // device and clears the binding cache, so an old frame can never be
        // replayed as a clean success.
        let apply_started = std::time::Instant::now();
        if let Err(error) = session.apply_ready_updates() {
            session.fail_device(format!("apply_ready_updates: {error:?}"), false);
            self.clear_presenter_cache();
            return Err(format!("apply_ready_updates failed: {error:?}"));
        }
        let apply_us = apply_started.elapsed().as_micros() as u64;
        let render_started = std::time::Instant::now();
        let presented = match session.render(logical, scale) {
            Ok(presented) => presented,
            Err(error) => {
                // Only a device loss fails the session device. Camera/size/
                // upload/scene errors keep the device attached and are returned
                // with their own class.
                let device_loss = error.is_device_loss();
                if device_loss {
                    session.fail_device(format!("render device lost: {error}"), true);
                }
                self.clear_presenter_cache();
                return Err(format!(
                    "render failed ({}) : {error}",
                    render_error_class(&error)
                ));
            }
        };

        match presented {
            Some(frame) => {
                let key = NativeTextureKey {
                    device_epoch: frame.binding.device_epoch,
                    texture_revision: frame.binding.texture_revision,
                    size: frame.binding.size,
                };
                let present_started = std::time::Instant::now();
                let result = self
                    .presenter
                    .as_mut()
                    .expect("presenter checked above")
                    .present_texture(&frame.texture, key);
                let present_us = present_started.elapsed().as_micros() as u64;
                self.last_frame_stages = Some(FrameCadStages {
                    apply_us,
                    render_us: render_started.elapsed().as_micros() as u64,
                    present_us,
                    presented: result.is_ok(),
                });
                match result {
                    Ok(stats) => {
                        // Record only the actually presented frame's metadata; a
                        // `None`/error never overwrites it, and the host consumes
                        // it once per presented frame.
                        self.last_cad_frame = Some(CadFrameObservation {
                            rendered_view_revision: frame.view_revision,
                            completeness: frame.completeness,
                            device_epoch: frame.binding.device_epoch,
                            texture_revision: frame.binding.texture_revision,
                            size: frame.binding.size,
                            frame_id: stats.frame_index,
                            timings: stats.timings,
                        });
                        Ok(Some(stats))
                    }
                    Err(error) => {
                        self.clear_presenter_cache();
                        Err(format!("present_texture failed: {error}"))
                    }
                }
            }
            // The engine returns its cached texture whenever the runtime is
            // `Ready`, so `None` means "not ready": wait. Never replay an old
            // frame and never draw a blank/fake grid.
            None => {
                // Record the waiting bracket too (a slow `render` while waiting is
                // exactly the host-frame gap to localize); no present happened.
                self.last_frame_stages = Some(FrameCadStages {
                    apply_us,
                    render_us: render_started.elapsed().as_micros() as u64,
                    present_us: 0,
                    presented: false,
                });
                Ok(None)
            }
        }
    }

    /// The last actually presented CAD frame's metadata (read-only peek).
    ///
    /// Stale (from an earlier successful present) after a `None`/error frame;
    /// the host must use [`Self::take_last_cad_frame_observation`] to log it
    /// exactly once per presented frame.
    pub fn last_cad_frame_observation(&self) -> Option<CadFrameObservation> {
        self.last_cad_frame
    }

    /// Take (consume) the last presented CAD frame's metadata, so a host logs it
    /// exactly once per presented frame and never replays a stale observation.
    pub fn take_last_cad_frame_observation(&mut self) -> Option<CadFrameObservation> {
        self.last_cad_frame.take()
    }

    /// Take (consume) the last `frame_cad` stage timings, so a host logs them at
    /// most once per call and never replays a stale value.
    pub fn take_frame_cad_stages(&mut self) -> Option<FrameCadStages> {
        self.last_frame_stages.take()
    }

    /// Drop the presenter's cached native-texture binding.
    fn clear_presenter_cache(&mut self) {
        if let Some(presenter) = self.presenter.as_mut() {
            presenter.clear_binding_cache();
        }
    }

    /// The logical size of the GTK container that **receives the CAD margins**
    /// (the overlay the React WebView fills), or `None` while its allocation is
    /// not settled (zero). These logical px are the frontend's DOM/CSS coordinate
    /// space, so the host must use them as the reference bounds for the viewport
    /// insets — never the Tauri window `inner_size()`.
    pub fn logical_container_bounds(&self) -> Option<(f32, f32)> {
        let width = self.overlay.allocated_width();
        let height = self.overlay.allocated_height();
        if width <= 0 || height <= 0 {
            return None;
        }
        Some((width as f32, height as f32))
    }

    /// Logical geometry snapshot for the bounded geometry telemetry (overlay /
    /// drawing-area allocation, translated origin, integer scale).
    pub fn layout_geometry(&self) -> LayoutGeometry {
        LayoutGeometry {
            container: (
                self.overlay.allocated_width(),
                self.overlay.allocated_height(),
            ),
            drawing_area: (
                self.drawing_area.allocated_width(),
                self.drawing_area.allocated_height(),
            ),
            origin: self
                .drawing_area
                .translate_coordinates(&self.top_level, 0, 0),
            scale: wayland_buffer_scale(self.drawing_area.scale_factor()),
        }
    }

    /// Current physical size the surface should be (allocation × integer scale).
    ///
    /// Uses the same checked rule as GPU init; an invalid/overflowing size is
    /// reported as `0x0` (the host skips the frame) with an explicit log — never a
    /// wrapping or saturating fake value.
    pub fn desired_size(&self) -> (u32, u32) {
        let scale = wayland_buffer_scale(self.drawing_area.scale_factor());
        match checked_physical_size(
            self.drawing_area.allocated_width(),
            self.drawing_area.allocated_height(),
            scale,
        ) {
            Ok(size) => size,
            Err(error) => {
                eprintln!("[probe] bridge desired_size invalid: {error}; reporting 0x0");
                (0, 0)
            }
        }
    }

    /// Whether GPU init failed (explicit, never a fallback).
    pub fn failure(&self) -> Option<&str> {
        match &self.state {
            BridgeState::Failed(message) => Some(message.as_str()),
            _ => None,
        }
    }

    /// Stop and drop the GPU presenter **before** any native widget teardown.
    /// Take any live presenter, stop and drop it, and log (B1).
    ///
    /// There is no permanent `released` latch: the slot is simply emptied, so a
    /// later re-init is safe and a repeated release is a no-op.
    pub fn release_gpu(&mut self) {
        // Detach the render session before any GPU object is dropped, then clear
        // the retained frame (it belongs to the dying device).
        if let Some(session) = self.session.as_ref() {
            session.detach_device();
            eprintln!("[probe] bridge: render session detached (before GPU drop)");
        }
        self.clear_presenter_cache();
        self.last_cad_frame = None;
        if let Some(mut presenter) = self.presenter.take() {
            presenter.stop();
            drop(presenter);
            eprintln!(
                "[probe] bridge: GPU released (generation {}; surface before native widgets)",
                self.generation
            );
        }
        // Ordering invariant: the GPU surface is already gone; now the app-owned
        // child `wl_surface`/`wl_subsurface`. This runs from the unrealize hooks,
        // i.e. before GTK destroys/unrealizes the borrowed parent surface.
        self.release_wayland_child();
        if self.state != BridgeState::Closing && !matches!(self.state, BridgeState::Failed(_)) {
            self.state = BridgeState::Suspended;
        }
    }

    /// Enter the terminal closing state; never inits again.
    pub fn close(&mut self) {
        self.state = BridgeState::Closing;
        self.release_gpu();
    }

    /// The current lifecycle state.
    pub fn state(&self) -> &BridgeState {
        &self.state
    }

    /// The current GPU generation (incremented per successful init).
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl Drop for GtkOverlayOwner {
    fn drop(&mut self) {
        // Guarantee ordering even if a hook did not fire: GPU first, then GTK.
        self.release_gpu();
    }
}

/// Reparent the Tauri webview into the overlay as a transparent sibling.
///
/// Runs on the main thread via `with_webview`; the closure is `Send`, so it looks
/// the overlay up in the thread-local registry instead of capturing GTK objects.
pub fn reparent_webview(webview: &tauri::Webview, label: &str) -> tauri::Result<()> {
    let label = label.to_string();
    webview.with_webview(move |platform| {
        let widget = platform.inner().upcast::<gtk::Widget>();
        if let Some(parent) = widget.parent() {
            if let Ok(container) = parent.downcast::<gtk::Container>() {
                container.remove(&widget);
            }
        }
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_app_paintable(true);
        // Clone the overlay out of the registry first: never hold the RefCell
        // borrow while calling GTK (which can re-enter signals).
        let overlay = OVERLAYS.with(|map| map.borrow().get(&label).cloned());
        match overlay {
            Some(overlay) => {
                overlay.add_overlay(&widget);
                overlay.set_overlay_pass_through(&widget, false);
                widget.show();
                // Variant: keep the CAD drawing area as the topmost overlay child
                // so the webview cannot cover it.
                if let Some(base) = overlay.child() {
                    overlay.remove(&base);
                    overlay.add_overlay(&base);
                    base.show();
                    // B3: re-assert the pass-through + empty input region for the
                    // new generation (the webview is now the sibling).
                    if let Ok(drawing) = base.clone().downcast::<gtk::DrawingArea>() {
                        apply_input_passthrough(&overlay, &drawing);
                    }
                }
                REPARENT_DONE.with(|flag| flag.set(true));
                eprintln!("[probe] bridge: webview reparented into the overlay");
            }
            None => eprintln!("[probe] bridge: no overlay for label {label}"),
        }
    })
}

/// B3: make the native drawing area transparent to input so OS pointer/wheel
/// events reach the React DOM underneath. On X11 this uses an **empty input
/// region** (not a visual/bounding shape) plus GTK overlay pass-through,
/// re-applied on every generation/layout change. On Wayland the empty input
/// region lives on the app-owned child `wl_surface` (set in the C shim), so the
/// GDK window's shared toplevel surface is left untouched.
fn apply_input_passthrough(overlay: &gtk::Overlay, drawing_area: &gtk::DrawingArea) {
    overlay.set_overlay_pass_through(drawing_area, true);
    if is_wayland_display(&drawing_area.display()) {
        // Never touch the GDK window input region on Wayland: the drawing area is
        // a non-native child sharing the GDK toplevel `wl_surface`, so an
        // `input_shape_combine_region` here would make the entire window
        // click-through. The child surface's own empty input region is what
        // routes CAD-region input to the DOM surfaces below.
        return;
    }
    if let Some(win) = drawing_area.window() {
        let empty =
            gdk::cairo::Region::create_rectangle(&gdk::cairo::RectangleInt::new(0, 0, 0, 0));
        win.input_shape_combine_region(&empty, 0, 0);
    }
}

/// The stable class of a render error, for explicit reporting.
pub(crate) fn render_error_class(error: &cad_render_wgpu::RenderError) -> &'static str {
    match error {
        cad_render_wgpu::RenderError::DeviceLost(_) => "device_lost",
        cad_render_wgpu::RenderError::Frame(_) => "frame",
        cad_render_wgpu::RenderError::NotInitialized(_) => "not_initialized",
    }
}

/// Whether GPU init is allowed from `state` (B1): only `AwaitingMap`/`Suspended`.
fn init_allowed(state: &BridgeState) -> bool {
    matches!(state, BridgeState::AwaitingMap | BridgeState::Suspended)
}

/// Whether a new "not ready yet" reason differs from the last one logged.
///
/// A deferral is logged once per change, so a persistently pending Wayland
/// global/selection does not spam the log but is still visible.
fn pending_reason_changed(current: Option<&str>, reason: &str) -> bool {
    current != Some(reason)
}

/// The integer buffer scale handed to the Wayland child surface: GTK3 only
/// reports integer scale, and it is never below 1.
fn wayland_buffer_scale(scale_factor: i32) -> i32 {
    scale_factor.max(1)
}

/// The single checked rule for a physical surface size: logical allocation ×
/// integer buffer scale.
///
/// A non-positive logical size, a scale below 1, or an `i32`/`u32` overflow is an
/// explicit `Err` — never a wrapping or saturating "success". Used by both GPU
/// init and [`GtkOverlayOwner::desired_size`].
fn checked_physical_size(logical_w: i32, logical_h: i32, scale: i32) -> Result<(u32, u32), String> {
    if logical_w <= 0 || logical_h <= 0 {
        return Err(format!(
            "logical size {logical_w}x{logical_h} must be positive"
        ));
    }
    if scale < 1 {
        return Err(format!("buffer scale {scale} must be >= 1"));
    }
    let width = logical_w
        .checked_mul(scale)
        .ok_or_else(|| format!("logical width {logical_w} × scale {scale} overflows i32"))?;
    let height = logical_h
        .checked_mul(scale)
        .ok_or_else(|| format!("logical height {logical_h} × scale {scale} overflows i32"))?;
    let width =
        u32::try_from(width).map_err(|_| format!("physical width {width} does not fit in u32"))?;
    let height = u32::try_from(height)
        .map_err(|_| format!("physical height {height} does not fit in u32"))?;
    Ok((width, height))
}

/// A bounded, human-readable summary of a caught panic payload.
///
/// The panic message from `catch_unwind` is preserved for diagnostics but capped
/// so an adversarial/large payload cannot flood the log.
fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    const MAX_CHARS: usize = 200;
    let text = if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "non-string panic payload".to_string()
    };
    if text.chars().count() > MAX_CHARS {
        let mut bounded: String = text.chars().take(MAX_CHARS).collect();
        bounded.push('…');
        bounded
    } else {
        text
    }
}

/// The transient prerequisites for a native surface, in order. `None` means the
/// widget is realized/mapped with its own window and native classification may
/// proceed; `Some(reason)` means "not ready yet" — the caller must **defer**
/// (keep `AwaitingMap`/`Suspended`) and never turn this into a terminal failure.
fn pending_prerequisite(has_window: bool, realized: bool, mapped: bool) -> Option<&'static str> {
    if !has_window {
        return Some("drawing area does not own a window yet");
    }
    if !realized {
        return Some("drawing area is not realized");
    }
    if !mapped {
        return Some("drawing area is not mapped");
    }
    None
}

/// Verified native X11 handles for the drawing area's own GdkWindow (B2).
struct NativeHandles {
    xid: core::ffi::c_ulong,
    display: NonNull<c_void>,
    screen: i32,
    /// Native (logical) window geometry, recorded for evidence.
    geom: (i32, i32),
    /// Retained so the XID/display stay valid while the surface exists.
    _window: gdk::Window,
    _display: gdkx11::X11Display,
}

/// Outcome of probing the drawing area's native X window.
enum NativeStatus {
    /// A real native X child window is ready for a surface.
    Ready(NativeHandles),
    /// Not ready yet (transient); keep waiting, never fail.
    Pending(&'static str),
    /// Genuinely terminal native problem; never retried.
    Fatal(String),
}

/// Classify the drawing area's native X window. Transient states (not
/// realized/mapped, no own GdkWindow, empty or not-steady geometry, not yet a
/// descendant of the top level) are [`NativeStatus::Pending`]; only terminal
/// native problems (not an X11 window, wrong screen/backend, invalid handles)
/// are [`NativeStatus::Fatal`]. `set_has_window` is not sufficient on its own.
/// There is **no screen-0 fallback**.
fn classify_native(
    drawing_area: &gtk::DrawingArea,
    top_level: &gtk::ApplicationWindow,
) -> NativeStatus {
    assert!(
        gtk::is_initialized_main_thread(),
        "gtk bridge classify_native must run on the GTK main thread"
    );
    if let Some(reason) = pending_prerequisite(
        drawing_area.has_window(),
        drawing_area.is_realized(),
        drawing_area.is_mapped(),
    ) {
        return NativeStatus::Pending(reason);
    }
    let Some(gdk_window) = drawing_area.window() else {
        return NativeStatus::Pending("drawing area has no GdkWindow");
    };
    // Explicit native promotion: `set_has_window` alone does NOT guarantee a
    // native X window. A FALSE here is terminal (the widget cannot be native).
    let native_ok = unsafe { gdk::ffi::gdk_window_ensure_native(gdk_window.to_glib_none().0) != 0 };
    if !native_ok {
        return NativeStatus::Fatal(
            "gdk_window_ensure_native returned FALSE (not a native window)".into(),
        );
    }
    if !gdk_window.is_visible() {
        return NativeStatus::Pending("drawing area GdkWindow is not visible/mapped");
    }
    let (gx, gy, gw, gh) = gdk_window.geometry();
    if gw <= 0 || gh <= 0 {
        return NativeStatus::Pending("drawing area GdkWindow geometry is empty");
    }
    // The native window must be a real descendant of the top level (parent chain)
    // and its geometry must be steady (equal to the widget allocation). If either
    // is not true yet, wait for a steady state instead of failing.
    let Some(top_gdk) = top_level.window() else {
        return NativeStatus::Pending("top-level GdkWindow is not realized yet");
    };
    let Some(top_xid) = top_gdk
        .clone()
        .downcast::<gdkx11::X11Window>()
        .ok()
        .map(|x| x.xid())
    else {
        return NativeStatus::Fatal("top-level GdkWindow is not an X11 window".into());
    };
    let mut cur = gdk_window.parent();
    let mut descendant = false;
    while let Some(parent) = cur {
        if let Ok(x) = parent.clone().downcast::<gdkx11::X11Window>() {
            if x.xid() == top_xid {
                descendant = true;
                break;
            }
        }
        cur = parent.parent();
    }
    if !descendant {
        return NativeStatus::Pending("drawing area is not a descendant of the top level yet");
    }
    let alloc_w = drawing_area.allocated_width();
    let alloc_h = drawing_area.allocated_height();
    if (gw - alloc_w).abs() > 1 || (gh - alloc_h).abs() > 1 {
        return NativeStatus::Pending("native geometry is not steady (allocation mismatch)");
    }
    let Ok(x11_window) = gdk_window.clone().downcast::<gdkx11::X11Window>() else {
        return NativeStatus::Fatal("GdkWindow is not an X11 window".into());
    };
    let xid = x11_window.xid();

    let display = gdk_window.display();
    let Ok(x11_display) = display.downcast::<gdkx11::X11Display>() else {
        return NativeStatus::Fatal("GdkDisplay is not an X11 display".into());
    };
    let Ok(screen) = gdk_window
        .screen()
        .downcast::<gdkx11::X11Screen>()
        .map(|s| s.screen_number())
    else {
        return NativeStatus::Fatal("GdkScreen is not an X11 screen (wrong backend)".into());
    };
    // SAFETY: `x11_display` is a live GdkX11Display; the returned Xlib Display
    // pointer is owned by GDK and lives as long as the display.
    let xdisplay = unsafe {
        gdkx11::ffi::gdk_x11_display_get_xdisplay(x11_display.to_glib_none().0) as *mut c_void
    };
    let Some(display) = NonNull::new(xdisplay) else {
        return NativeStatus::Fatal("Xlib Display is null (invalid handles)".into());
    };

    let _ = (gx, gy);
    NativeStatus::Ready(NativeHandles {
        xid,
        display,
        screen,
        geom: (gw, gh),
        _window: gdk_window,
        _display: x11_display,
    })
}

/// Register the GTK unrealize hooks that release the GPU before the native
/// widgets are destroyed. The overlay hook (normal stage, i.e. before
/// `gtk_overlay_unrealize` destroys its child windows) is the primary guarantee;
/// the top-level and drawing-area hooks are additional.
pub fn install_unrealize_hooks(owner: &GtkOverlayOwner, release: Rc<dyn Fn()>) {
    {
        let release = release.clone();
        owner.overlay.connect_unrealize(move |_| release());
    }
    {
        let release = release.clone();
        owner.top_level.connect_unrealize(move |_| release());
    }
    owner.drawing_area.connect_unrealize(move |_| release());
}

#[cfg(test)]
mod tests {
    use super::{
        checked_physical_size, init_allowed, panic_payload_message, pending_prerequisite,
        pending_reason_changed, render_error_class, wayland_buffer_scale, BridgeState,
    };
    #[test]
    fn desired_physical_size_is_allocation_times_scale() {
        // Pure check of the size rule used for the raw surface (no GTK needed).
        let alloc = (960i32, 600i32);
        for scale in [1i32, 2i32] {
            assert_eq!(
                checked_physical_size(alloc.0, alloc.1, scale),
                Ok((960 * scale as u32, 600 * scale as u32))
            );
        }
    }

    #[test]
    fn checked_physical_size_rejects_non_positive_and_bad_scale() {
        assert!(checked_physical_size(0, 600, 1).is_err());
        assert!(checked_physical_size(960, -1, 1).is_err());
        assert!(checked_physical_size(960, 600, 0).is_err());
        assert!(checked_physical_size(960, 600, -2).is_err());
    }

    #[test]
    fn checked_physical_size_overflow_is_explicit() {
        // A wrapping multiply would have produced a bogus but "valid-looking"
        // size; it must be an explicit error instead.
        let overflow = checked_physical_size(i32::MAX, 1, 2);
        assert!(overflow.is_err());
        assert!(overflow.unwrap_err().contains("overflows i32"));
    }

    #[test]
    fn panic_payload_is_preserved_and_bounded() {
        let short: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_payload_message(short.as_ref()), "boom");
        let other: Box<dyn std::any::Any + Send> = Box::new(42u32);
        assert_eq!(
            panic_payload_message(other.as_ref()),
            "non-string panic payload"
        );
        let long: Box<dyn std::any::Any + Send> = Box::new("x".repeat(500));
        let bounded = panic_payload_message(long.as_ref());
        assert!(bounded.chars().count() <= 201);
        assert!(bounded.ends_with('…'));
    }

    #[test]
    fn init_gate_allows_only_awaiting_or_suspended() {
        assert!(init_allowed(&BridgeState::AwaitingMap));
        assert!(init_allowed(&BridgeState::Suspended));
        assert!(!init_allowed(&BridgeState::Ready));
        assert!(!init_allowed(&BridgeState::Closing));
        assert!(!init_allowed(&BridgeState::Failed("x".into())));
    }

    #[test]
    fn closing_never_inits_and_failed_never_retries() {
        // Terminal states are never allowed to (re)init a surface.
        for state in [
            BridgeState::Closing,
            BridgeState::Failed("terminal".into()),
            BridgeState::Ready,
        ] {
            assert!(!init_allowed(&state), "{state:?} must not init");
        }
    }

    #[test]
    fn unmapped_prerequisites_defer_instead_of_failing() {
        // A not-yet-mapped widget is a transient prerequisite: the caller defers.
        assert_eq!(
            pending_prerequisite(false, false, false),
            Some("drawing area does not own a window yet")
        );
        assert_eq!(
            pending_prerequisite(true, false, false),
            Some("drawing area is not realized")
        );
        assert_eq!(
            pending_prerequisite(true, true, false),
            Some("drawing area is not mapped")
        );
        // Only when realized + mapped with its own window may native
        // classification proceed.
        assert_eq!(pending_prerequisite(true, true, true), None);
    }

    #[test]
    fn non_device_render_errors_are_not_device_loss() {
        use cad_render_wgpu::RenderError;
        // Only a device loss fails the session device; camera/size/upload/scene
        // errors keep the device attached and report their own class.
        assert!(RenderError::DeviceLost("reset".into()).is_device_loss());
        assert!(!RenderError::Frame("camera".into()).is_device_loss());
        assert!(!RenderError::NotInitialized("no device".into()).is_device_loss());
        assert_eq!(render_error_class(&RenderError::Frame("x".into())), "frame");
        assert_eq!(
            render_error_class(&RenderError::DeviceLost("x".into())),
            "device_lost"
        );
    }

    #[test]
    fn zero_allocation_is_rejected_by_the_size_helper() {
        // A zero/negative allocation must never produce a surface size; the helper
        // rejects it explicitly (and `desired_size` reports 0x0 so the host skips).
        assert!(checked_physical_size(0, 0, 1).is_err());
        assert!(checked_physical_size(0, 600, 1).is_err());
        assert!(checked_physical_size(960, 0, 2).is_err());
    }

    #[test]
    fn deferral_is_logged_once_per_reason_change() {
        // A persistently pending Wayland global/selection is logged once, and a
        // changed reason logs again.
        assert!(pending_reason_changed(None, "waiting"));
        assert!(!pending_reason_changed(Some("waiting"), "waiting"));
        assert!(pending_reason_changed(Some("waiting"), "other"));
    }

    #[test]
    fn wayland_buffer_scale_is_at_least_one() {
        // GTK3 only reports integer scale; a degenerate 0/negative factor must
        // never reach the child surface (`wl_surface.set_buffer_scale` requires
        // >= 1).
        assert_eq!(wayland_buffer_scale(2), 2);
        assert_eq!(wayland_buffer_scale(1), 1);
        assert_eq!(wayland_buffer_scale(0), 1);
        assert_eq!(wayland_buffer_scale(-3), 1);
    }

    #[test]
    fn wayland_physical_size_is_allocation_times_scale() {
        // The app-owned child surface's physical size is the CAD allocation times
        // the integer scale (the same rule as the X11 native child), so the
        // render extent stays bounded to the CAD rect.
        let alloc = (640i32, 480i32);
        for scale in [1i32, 2i32] {
            let factor = wayland_buffer_scale(scale) as u32;
            assert_eq!(
                ((alloc.0 as u32) * factor, (alloc.1 as u32) * factor),
                (640 * scale as u32, 480 * scale as u32)
            );
        }
    }
}
