//! GPU-init and Wayland lifecycle methods of [`GtkOverlayOwner`].

use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::wayland_surface::{
    is_wayland_display, EnsureChild, NativeHealth, SubsurfaceReadiness, WaylandSurface,
};
use cad_render_wgpu::{gpu_time::GpuTimeConfig, ProbePresentMode, SurfaceProbePresenter};
use gtk::prelude::*;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
    XlibDisplayHandle, XlibWindowHandle,
};

use super::native::{
    apply_input_passthrough, checked_physical_size, classify_native, drawing_area_matches_dom,
    init_allowed, panic_payload_message, pending_reason_changed, wayland_buffer_scale,
    NativeStatus,
};
use super::{BridgeState, GtkOverlayOwner, OVERLAYS, REPARENT_DONE};

/// Production top-level alpha policy for DOM-on-top (no probe gate).
///
/// The GDK toplevel **is** the WebView surface, and it is opaque by default.
/// Since the native CAD child is always placed *below* it, an opaque parent would
/// hide the CAD rendering completely. GTK owns this drawable and every commit, so
/// the clear happens inside GTK's own draw cycle — never by attaching or
/// submitting a Vulkan buffer to the borrowed parent `wl_surface` — and the
/// opaque region is dropped on the same pass.
///
/// The same handler records the first GTK-owned parent commit after a child
/// (re)creation: `place_below` + the position are parent-relative double-buffered
/// state and only take effect on that commit. X11 never reaches this function at
/// runtime (`backend::prepare_backend` refuses it), so it is unconditional.
fn install_parent_alpha_clear(top: &gtk::ApplicationWindow) {
    top.connect_draw(|widget, context| {
        super::stacking_regression::note_parent_commit();
        if let Some(window) = widget.window() {
            window.set_opaque_region(None);
        }
        context.save().ok();
        context.set_operator(gdk::cairo::Operator::Source);
        context.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        context.paint().ok();
        context.restore().ok();
        gtk::glib::Propagation::Proceed
    });
    eprintln!(
        "[dom-on-top] parent GTK clear=transparent opaque_region=none-on-draw app_paintable={} rgba_visual_available={} composite=NOT_VERIFIED",
        top.is_app_paintable(),
        gtk::prelude::WidgetExt::screen(top).is_some_and(|screen| screen.rgba_visual().is_some()),
    );
}

impl GtkOverlayOwner {
    pub fn install(
        window: &tauri::Window,
        label: &str,
        gpu_time: GpuTimeConfig,
    ) -> Result<Self, String> {
        let gtk_window = window
            .gtk_window()
            .map_err(|error| format!("gtk_window: {error}"))?;
        gtk_window.set_app_paintable(true);
        // DOM-on-top production path: the GDK toplevel must be transparent so the
        // CAD child placed below it is visible. This is not probe-gated and there
        // is no alternative arrangement.
        install_parent_alpha_clear(&gtk_window);
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
        // unreachable_since_dom_on_top: this host refuses X11 in
        // `backend::prepare_backend` before GTK initializes, so `x11_backend` is
        // always false in the shipped product; retained for the deferred X11
        // web+canvas route decision.
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
            gpu_time,
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
        // Drive the layout explicitly. Setting a margin only *queues* a GTK
        // resize, and that queued resize is never drained in this host: the
        // drawing area kept its install-time geometry and no `resize configure`
        // ever arrived. The webview reparent also leaves the drawing area as a
        // GtkOverlay **overlay** child, for which GtkOverlay ignores the
        // position margins, so `set_margin_*` alone can never place it on the
        // DOM CAD rect. `GtkWindow::check_resize` runs GTK's size negotiation
        // synchronously, which is what actually re-lays out the drawing area
        // (and the widget window that carries its margins) onto the DOM rect.
        let origin = self.drive_drawing_area_layout();
        // Wayland: the subsurface position follows the drawing area's logical
        // origin within the toplevel (the native region is app-owned, not a GTK
        // child window). Read it *after* the explicit layout above. Re-synced
        // again per frame in `resize_to`.
        self.sync_wayland_layout();
        eprintln!(
            "[probe] bridge layout: margins top={} bottom={} start={} end={} origin={:?}",
            margins.top, margins.bottom, margins.start, margins.end, origin
        );
    }

    /// Drive GTK's size negotiation so the drawing area lands on the DOM CAD
    /// rect, and return its resulting toplevel-relative origin.
    ///
    /// The queued resize that `set_margin_*` schedules is never drained in this
    /// host, so the margins must be applied by an explicit synchronous
    /// `GtkWindow::check_resize` (after queueing the resize up the ancestor
    /// chain). Returns `None` while the widgets are not realized/allocated.
    pub fn drive_drawing_area_layout(&mut self) -> Option<(i32, i32)> {
        assert!(
            gtk::is_initialized_main_thread(),
            "gtk bridge drive_drawing_area_layout must run on the GTK main thread"
        );
        self.drawing_area.queue_resize();
        self.overlay.queue_resize();
        self.vbox.queue_resize();
        self.top_level.queue_resize();
        self.top_level.check_resize();
        self.drawing_area_origin()
    }

    /// The drawing area's origin translated into the toplevel, in logical px
    /// (`None` until both widgets are realized/allocated). The DOM CAD rect is
    /// relative to the container, so this must equal the container origin plus
    /// the DOM rect origin for pointer input to line up.
    pub fn drawing_area_origin(&self) -> Option<(i32, i32)> {
        self.drawing_area
            .translate_coordinates(&self.top_level, 0, 0)
    }

    /// The margin-receiving container's (the overlay's) origin in the toplevel.
    pub fn container_origin(&self) -> Option<(i32, i32)> {
        self.overlay.translate_coordinates(&self.top_level, 0, 0)
    }

    /// The drawing area's current allocation `(x, y, width, height)`, relative
    /// to its parent (the overlay == the DOM/CSS container), in logical px.
    pub fn drawing_area_allocation(&self) -> (i32, i32, i32, i32) {
        let allocation = self.drawing_area.allocation();
        (
            allocation.x(),
            allocation.y(),
            allocation.width(),
            allocation.height(),
        )
    }

    /// Whether the drawing area currently occupies exactly the React DOM CAD
    /// rect (within one logical pixel). See [`drawing_area_matches_dom`].
    pub fn drawing_area_matches_dom(&self, dom: (f32, f32, f32, f32)) -> bool {
        let size = (
            self.drawing_area.allocated_width(),
            self.drawing_area.allocated_height(),
        );
        drawing_area_matches_dom(
            self.container_origin(),
            self.drawing_area_origin(),
            size,
            dom,
        )
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

        // unreachable_since_dom_on_top: the X11 native-child path below cannot run
        // in the shipped host because `backend::prepare_backend` refuses X11
        // (marker line + exit code 4) before GTK exists. Retained for the deferred
        // X11 web+canvas route decision; unit tests keep it from rotting.
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
                self.gpu_time,
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
        if let Err(error) = session.attach_device_with_gpu_time(device, queue, self.gpu_time) {
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
            Ok(EnsureChild::Created) => {
                // Every (re)created child re-applies `place_below`; record the
                // generation whose first parent commit is now pending.
                super::stacking_regression::child_created(self.generation + 1);
            }
            Ok(EnsureChild::Current) => {}
            Ok(EnsureChild::ParentReplaced) => {
                // No presenter exists on this (init) path, so a stale child from a
                // replaced parent can be safely destroyed and re-created once.
                self.release_wayland_child();
                let retry = {
                    let wayland = self.wayland.as_mut().expect("attached above");
                    wayland.ensure_child(&top, x, y, scale)
                };
                match retry {
                    Ok(EnsureChild::Created) => {
                        super::stacking_regression::child_created(self.generation + 1);
                    }
                    Ok(EnsureChild::Current) => {}
                    Ok(EnsureChild::ParentReplaced) => {
                        let error = "wayland GDK parent surface replaced twice in one init tick"
                            .to_string();
                        self.state = BridgeState::Failed(error.clone());
                        eprintln!("[probe] bridge wayland FAILED (terminal): {error}");
                        return Some(Err(error));
                    }
                    Err(error) => {
                        self.state = BridgeState::Failed(error.clone());
                        eprintln!("[probe] bridge wayland FAILED (terminal): {error}");
                        return Some(Err(error));
                    }
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
                self.gpu_time,
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
    pub(super) fn sync_wayland_layout(&mut self) {
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
    pub(super) fn enforce_wayland_health(&mut self) -> Result<(), String> {
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
    pub(super) fn release_wayland_child(&mut self) {
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
}
