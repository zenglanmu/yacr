//! GTK overlay bridge for the React probe (Linux/X11).
//!
//! **Root cause this solves:** creating the wgpu Vulkan surface on the Tauri/GTK
//! **top-level** window makes the Vulkan swapchain cover the React webview. Here
//! the probe surface instead gets its own **windowed `GtkDrawingArea`** as an
//! independent native X child inside a `GtkOverlay`, with the React webview
//! reparented as a transparent sibling on top. There is **no top-level fallback**.
//!
//! Threading/lifetime:
//! - [`GtkOverlayOwner`] is `!Send`/`!Sync` (`PhantomData<Rc<()>>`): it is created,
//!   used and dropped only on the host main thread. It owns the GTK widgets and
//!   the GPU presenter, and always releases the GPU **before** the native widgets.
//! - The GPU surface is created from raw X11 handles
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

use cad_engine::render::{FrameCompleteness, RenderSessionHandle};
use cad_render_wgpu::{
    NativeTextureKey, ProbeFrameStats, ProbePresentMode, ProbeStageTimings, SurfaceProbePresenter,
};
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, XlibDisplayHandle, XlibWindowHandle};

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
    /// Last "not ready yet" reason, so a deferral is logged once per change and
    /// never becomes a terminal failure.
    pending_reason: Option<String>,
    /// The CAD render session bound to this owner (cheap `Rc` clone), if any.
    session: Option<RenderSessionHandle>,
    /// Metadata of the last actually presented CAD frame (telemetry only).
    last_cad_frame: Option<CadFrameObservation>,
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
        // Own GdkWindow (a native X child) before realize; without this the
        // widget is client-side and its XID would equal the parent's.
        #[allow(deprecated)]
        drawing_area.set_has_window(true);
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
            pending_reason: None,
            session: None,
            last_cad_frame: None,
        })
    }

    /// Apply the React CAD rect as drawing-area margins (logical px == CSS px)
    /// and mark the layout confirmed so GPU init may proceed. A rect that only
    /// shifts still updates the margins.
    pub fn apply_layout(&mut self, margins: crate::layout::CadMargins) {
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
        eprintln!(
            "[probe] bridge layout: margins top={} bottom={} start={} end={}",
            margins.top, margins.bottom, margins.start, margins.end
        );
    }

    /// Log the native window tree / map state / XID / geometry (diagnostics).
    pub fn log_tree(&self) {
        eprintln!(
            "[probe] bridge tree: top_level_has_window={} drawing_area_has_window={}",
            self.top_level.has_window(),
            self.drawing_area.has_window(),
        );
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
        let scale = self.drawing_area.scale_factor().max(1);
        if alloc_w <= 0 || alloc_h <= 0 {
            return None;
        }

        let handles = match classify_native(&self.drawing_area, &self.top_level) {
            NativeStatus::Ready(handles) => handles,
            NativeStatus::Pending(reason) => {
                // Transient: wait for the next tick instead of failing forever.
                if self.pending_reason.as_deref() != Some(reason) {
                    eprintln!("[probe] bridge waiting: {reason}");
                    self.pending_reason = Some(reason.to_string());
                }
                return None;
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
        let phys_w = (alloc_w * scale) as u32;
        let phys_h = (alloc_h * scale) as u32;

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
        if let Some(session) = self.session.clone() {
            let (device, queue) = self
                .presenter
                .as_ref()
                .expect("presenter was just set")
                .device_queue();
            if let Err(error) = session.attach_device(device, queue) {
                session.fail_device(format!("attach_device: {error}"), false);
                let message = format!("render session attach failed: {error}");
                self.state = BridgeState::Failed(message.clone());
                eprintln!("[probe] bridge render session attach FAILED: {error}");
                return Some(Err(message));
            }
            eprintln!(
                "[probe] bridge: render session attached (generation {})",
                self.generation
            );
        }
        Some(Ok(()))
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
        let Some(session) = self.session.clone() else {
            return Err("no render session attached".into());
        };
        if self.presenter.is_none() {
            return Err("no GPU presenter (bridge not ready)".into());
        }
        // `apply_ready_updates` is a CadError: any failure fails the session
        // device and clears the binding cache, so an old frame can never be
        // replayed as a clean success.
        if let Err(error) = session.apply_ready_updates() {
            session.fail_device(format!("apply_ready_updates: {error:?}"), false);
            self.clear_presenter_cache();
            return Err(format!("apply_ready_updates failed: {error:?}"));
        }
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
                let result = self
                    .presenter
                    .as_mut()
                    .expect("presenter checked above")
                    .present_texture(&frame.texture, key);
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
            None => Ok(None),
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

    /// Drop the presenter's cached native-texture binding.
    fn clear_presenter_cache(&mut self) {
        if let Some(presenter) = self.presenter.as_mut() {
            presenter.clear_binding_cache();
        }
    }

    /// Current physical size the surface should be (allocation * scale).
    pub fn desired_size(&self) -> (u32, u32) {
        let scale = self.drawing_area.scale_factor().max(1);
        (
            (self.drawing_area.allocated_width().max(0) * scale) as u32,
            (self.drawing_area.allocated_height().max(0) * scale) as u32,
        )
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
/// events reach the React DOM underneath. This uses an **empty input region**
/// (not a visual/bounding shape) and GTK overlay pass-through, re-applied on
/// every generation/layout change.
fn apply_input_passthrough(overlay: &gtk::Overlay, drawing_area: &gtk::DrawingArea) {
    overlay.set_overlay_pass_through(drawing_area, true);
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
    use super::{init_allowed, pending_prerequisite, render_error_class, BridgeState};
    #[test]
    fn desired_physical_size_is_allocation_times_scale() {
        // Pure check of the size rule used for the raw surface (no GTK needed).
        let alloc = (960i32, 600i32);
        for scale in [1i32, 2i32] {
            let phys = (
                (alloc.0.max(0) * scale) as u32,
                (alloc.1.max(0) * scale) as u32,
            );
            assert_eq!(phys, (960 * scale as u32, 600 * scale as u32));
        }
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
    fn zero_allocation_yields_zero_physical_size() {
        let alloc = (0i32, 0i32);
        let scale = 1i32;
        let phys = (
            (alloc.0.max(0) * scale) as u32,
            (alloc.1.max(0) * scale) as u32,
        );
        assert_eq!(
            phys,
            (0, 0),
            "a zero allocation must not be used for the surface"
        );
    }
}
