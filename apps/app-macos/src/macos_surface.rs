//! The native CAD surface: an in-process child `NSView` **below** the WKWebView
//! layer, presented by the single renderer presenter.
//!
//! ## Composition contract
//!
//! 1. Tauri/wry creates the WKWebView as a subview of the window's content view
//!    (`window.add_child`). The content view is the only host-owned parent we
//!    need; the WKWebView is the ordering anchor.
//! 2. This module creates a sibling `NSView` (**not** layer-hosting, no
//!    responder overrides, no gesture recognizers) and inserts it with
//!    `addSubview:positioned:relativeTo:` **below** the WKWebView
//!    (`NSWindowOrderingMode::Below`). The CAD view is display-only and never
//!    takes OS input; the DOM is the single input funnel.
//! 3. The renderer surface is created from raw `AppKitWindowHandle`s for **that
//!    child view**. wgpu-hal (through `raw-window-metal`) makes the view
//!    layer-backed and inserts/observes a `CAMetalLayer`, so resize/Retina
//!    tracking is automatic and no `CAMetalLayer` is created here.
//!
//! Geometry comes only from the accepted React CAD rect. The **surface size** is
//! `cad_ui_core::layout::physical_content_size` (the render session's own
//! `round(side · scale)` formula), so the drawable and the render target are the
//! same pixel count by construction. The **view frame** is the logical rect in
//! AppKit points (bottom-left origin), which is what lets the observed layer
//! derive the same physical bounds.
//!
//! Lifecycle: `AwaitingMap` → `Ready` (presenter created) → `Suspended`
//! (released; re-init allowed) / `Closing` (terminal). Release order is always
//! presenter → render session detach; the view itself belongs to the window's
//! content view and stays attached until [`Drop`].
//!
//! Everything here runs on the host main thread (the thread that owns the
//! native views); the value is `!Send` and lives in the host thread-local.
//! Creating the Metal surface off the main thread is a hard panic in
//! `raw-window-metal`, so [`MacosCadSurface::try_init_gpu`] refuses explicitly
//! instead.

use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use cad_engine::render::RenderSessionHandle;
use cad_render_wgpu::{
    gpu_time::GpuTimeConfig, ProbeBackendPolicy, ProbeError, ProbeFrameStats, ProbePresentMode,
    SurfaceProbePresenter,
};
use cad_ui_core::layout::{physical_content_size, CadMetrics};
use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindowOrderingMode};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize};
use raw_window_handle::{
    AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
};

mod geometry;
mod present_error;

use geometry::reconfigure_plan;
pub use geometry::size_authority_verdict;
use present_error::render_error_class;
pub use present_error::{CadPresentClass, CadPresentError};

/// Explicit lifecycle state for the native surface.
///
/// GPU init is only allowed from [`BridgeState::AwaitingMap`] /
/// [`BridgeState::Suspended`]; `Closing`/`Failed` never re-init.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeState {
    /// Waiting for a confirmed React CAD rect and a live presenter.
    AwaitingMap,
    /// The presenter is live.
    Ready,
    /// The presenter was released (teardown/re-create); re-init is allowed.
    Suspended,
    /// Shutting down; never init again.
    Closing,
    /// Terminal failure (explicit; there is no fallback).
    Failed(String),
}

/// Main-thread-only owner of the CAD child `NSView` and its presenter.
pub struct MacosCadSurface {
    _not_send_sync: PhantomData<Rc<()>>,
    /// The window's content view (the WKWebView's superview).
    content: NonNull<NSView>,
    /// The WKWebView view, kept as the ordering anchor.
    webview: NonNull<NSView>,
    /// The display-only CAD child view (retained by the content view; one owned
    /// reference is kept here so the surface can remove it on teardown).
    cad: Retained<NSView>,
    presenter: Option<SurfaceProbePresenter>,
    /// Physical CAD size (the size authority).
    desired: (u32, u32),
    /// Logical frame last applied, in points `(x, y, width, height)`.
    frame: (f64, f64, f64, f64),
    state: BridgeState,
    /// Incremented on every successful presenter creation.
    generation: u64,
    /// The bound CAD render session (cheap `Rc` clone), if any.
    session: Option<RenderSessionHandle>,
    gpu_time: GpuTimeConfig,
    /// Last emitted z-order line, so a steady order does not log per tick.
    order_logged: bool,
    /// Whether a persistent `ensure_order` failure was already reported, so a
    /// broken re-assert does not flood the log every tick.
    order_failure_logged: bool,
}

impl MacosCadSurface {
    /// Create the CAD child view below the child WKWebView and remember both
    /// anchors.
    ///
    /// The presenter is **not** created here: it waits for the accepted CAD rect
    /// (see [`Self::try_init_gpu`]). The child starts hidden at 1×1 and is shown
    /// on the first accepted layout.
    pub fn install(window: &tauri::Window, gpu_time: GpuTimeConfig) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            "the macOS CAD child view must be installed on the host main thread".to_string()
        })?;
        let content = window
            .ns_view()
            .map_err(|error| format!("window ns_view unavailable: {error}"))?;
        let content = NonNull::new(content.cast::<NSView>())
            .ok_or_else(|| "window ns_view is null".to_string())?;
        // The child webview is a direct subview of the content view (wry's
        // `build_as_child` path). It is located by class rather than through
        // `tauri::Webview::inner()`: that accessor lives on `PlatformWebview`,
        // which is only reachable from the asynchronous `with_webview` callback.
        // SAFETY: live content view on the host main thread (see `install`).
        let content_ref: &NSView = unsafe { content.as_ref() };
        let webview = find_webview_view(content_ref)
            .ok_or_else(|| "WKWebView subview not found after add_child".to_string())?;

        let cad = NSView::new(mtm);
        cad.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0)));
        // Hidden until the first accepted DOM rect: a 1×1 visible Metal surface
        // would otherwise flash at the window's origin.
        cad.setHidden(true);
        // No autoresizing mask: the frame is driven only by the accepted React
        // CAD rect, never by the window. AppKit coordinates are bottom-left and
        // the view is not flipped, so `apply_layout` converts the DOM top-left
        // origin.
        cad.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
        // SAFETY: both pointers were just obtained from live Tauri objects on the
        // main thread; the window outlives this surface (the host releases the
        // surface before the window is torn down).
        let webview_ref: &NSView = unsafe { webview.as_ref() };
        content_ref.addSubview_positioned_relativeTo(
            &cad,
            NSWindowOrderingMode::Below,
            Some(webview_ref),
        );

        Ok(Self {
            _not_send_sync: PhantomData,
            content,
            webview,
            cad,
            presenter: None,
            desired: (0, 0),
            frame: (0.0, 0.0, 0.0, 0.0),
            state: BridgeState::AwaitingMap,
            generation: 0,
            session: None,
            gpu_time,
            order_logged: false,
            order_failure_logged: false,
        })
    }

    /// The physical CAD rectangle last applied (also the desired surface size).
    pub fn desired_size(&self) -> (u32, u32) {
        self.desired
    }

    /// The presenter's current surface size, if initialized.
    pub fn surface_size(&self) -> Option<(u32, u32)> {
        self.presenter
            .as_ref()
            .map(SurfaceProbePresenter::surface_size)
    }

    /// Adapter description, if initialized.
    pub fn adapter(&self) -> Option<&cad_render_wgpu::ProbeAdapterInfo> {
        self.presenter.as_ref().map(SurfaceProbePresenter::adapter)
    }

    /// Present mode, if initialized.
    pub fn present_mode(&self) -> Option<cad_render_wgpu::wgpu::PresentMode> {
        self.presenter
            .as_ref()
            .map(SurfaceProbePresenter::present_mode)
    }

    /// The current lifecycle state.
    pub fn state(&self) -> &BridgeState {
        &self.state
    }

    /// Whether GPU init failed (explicit, never a fallback).
    pub fn failure(&self) -> Option<&str> {
        match &self.state {
            BridgeState::Failed(message) => Some(message.as_str()),
            _ => None,
        }
    }

    /// The current presenter generation (incremented per successful init).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Re-assert the CAD-view-below-WKWebView order when it drifted.
    ///
    /// Idempotent and cheap: a steady order only enumerates the content view's
    /// subviews (a few pointers). wry adds its webview with `addSubview:` and
    /// any later re-add appends on top, so the CAD view is re-inserted below the
    /// webview whenever the sibling order disagrees.
    pub fn ensure_order(&mut self) {
        // SAFETY: the content view and WKWebView are live for the surface's
        // lifetime (the host releases the surface before window teardown).
        let (content, webview) = unsafe { (self.content.as_ref(), self.webview.as_ref()) };
        if self.order_holds(content, webview) {
            if !self.order_logged {
                self.order_logged = true;
                eprintln!(
                    "[probe] macos z-order: cad child directly below the WKWebView \
                     (anchor=webview, rect={:.0}x{:.0})",
                    self.frame.2, self.frame.3
                );
            }
            return;
        }
        // Remove first so a repeated re-assert cannot accumulate duplicates.
        self.cad.removeFromSuperview();
        content.addSubview_positioned_relativeTo(
            &self.cad,
            NSWindowOrderingMode::Below,
            Some(webview),
        );
        if !self.order_holds(content, webview) {
            // A persistent failure (e.g. the WKWebView left the hierarchy) would
            // otherwise print every tick; report once per failure state.
            if !self.order_failure_logged {
                eprintln!(
                    "[probe] macos z-order re-assert FAILED \
                     (suppressing repeats until recovery)"
                );
                self.order_failure_logged = true;
            }
            return;
        }
        self.order_failure_logged = false;
        self.order_logged = true;
        eprintln!(
            "[probe] macos z-order (re-assert): cad child directly below the WKWebView \
             (rect={:.0}x{:.0})",
            self.frame.2, self.frame.3
        );
    }

    /// Whether the CAD view is the sibling immediately below `webview`.
    fn order_holds(&self, content: &NSView, webview: &NSView) -> bool {
        let subviews = content.subviews();
        let cad = Retained::as_ptr(&self.cad);
        let mut webview_index = None;
        let mut cad_index = None;
        for index in 0..subviews.count() {
            let view = subviews.objectAtIndex(index);
            let ptr = Retained::as_ptr(&view);
            if std::ptr::eq(ptr, webview as *const NSView) {
                webview_index = Some(index);
            } else if std::ptr::eq(ptr, cad) {
                cad_index = Some(index);
            }
        }
        match (cad_index, webview_index) {
            // `subviews` is back-to-front: index 0 is the bottom-most view.
            (Some(cad), Some(webview)) => cad + 1 == webview,
            _ => false,
        }
    }

    /// Apply the accepted DOM layout: size the surface from the CAD content size
    /// authority and frame the child view at the DOM rect in AppKit points.
    ///
    /// The **surface size** is [`physical_content_size`] so the drawable and the
    /// render target are the same pixel count by construction. The **frame** is
    /// logical (points); AppKit's bottom-left origin means the DOM's top-left
    /// `y` becomes `content_height - (y + height)`.
    pub fn apply_layout(&mut self, rect: &CadMetrics, scale: f64) -> Result<(), CadPresentError> {
        let scale = scale.max(0.1);
        let (width, height) = physical_content_size(rect, scale);
        // SAFETY: live content view for this surface's lifetime (see `install`).
        let content = unsafe { self.content.as_ref() };
        let content_height: f64 = content.bounds().size.height;
        let x = f64::from(rect.x).max(0.0);
        let y = (content_height - (f64::from(rect.y) + f64::from(rect.height))).max(0.0);
        let frame = (x, y, f64::from(rect.width), f64::from(rect.height));
        self.frame = frame;
        self.cad.setFrame(NSRect::new(
            NSPoint::new(frame.0, frame.1),
            NSSize::new(frame.2, frame.3),
        ));
        self.cad.setHidden(false);
        self.desired = (width.max(1), height.max(1));
        // The size authority must hold at runtime, not only at init: reconfigure
        // the swapchain in the same step that frames the view, so the geometry
        // snapshot taken right after this call already reports
        // `native_surface == child_desired == dom_physical`.
        self.resize_to(width, height)?;
        Ok(())
    }

    /// Deferred GPU init: create the presenter once a confirmed CAD rect yields a
    /// non-zero physical size.
    ///
    /// `None` means "wait"; `Some(Err)` is a terminal failure.
    pub fn try_init_gpu(&mut self, layout_confirmed: bool) -> Option<Result<(), String>> {
        if !matches!(
            self.state,
            BridgeState::AwaitingMap | BridgeState::Suspended
        ) {
            return None;
        }
        if !layout_confirmed || self.presenter.is_some() {
            return None;
        }
        let (width, height) = self.desired_size();
        if width == 0 || height == 0 {
            return None;
        }
        // `raw-window-metal::Layer::from_ns_view` panics off the main thread.
        // Refuse explicitly instead of letting the presenter panic.
        if MainThreadMarker::new().is_none() {
            let message =
                "the macOS CAD presenter must be created on the host main thread".to_string();
            self.state = BridgeState::Failed(message.clone());
            return Some(Err(message));
        }
        // Durable `wgpu_backend` preference, read once per presenter build (never
        // per frame) through the host's own preference store. An absent or
        // unreadable preference resolves to the renderer's `auto`, which is
        // Metal on macOS.
        let backend_policy =
            ProbeBackendPolicy::parse(crate::host::wgpu_backend_preference().as_deref());
        let display = RawDisplayHandle::AppKit(AppKitDisplayHandle::new());
        let window_handle = RawWindowHandle::AppKit(AppKitWindowHandle::new(
            NonNull::from(&*self.cad).cast::<std::ffi::c_void>(),
        ));
        // Metal advertises `Fifo` (and `Immediate` only when display sync is
        // available); it does not advertise `Mailbox`, so the Windows host's
        // `MailboxElseImmediate` request would select a mode this backend may not
        // support. `Fifo` is the v-synced product mode.
        //
        // SAFETY: `self.cad` is a live `NSView` owned (retained) by this surface,
        // inserted into a live window content view on the main thread. The
        // presenter and this surface both live in the host main thread's
        // thread-local; the view is not removed until the surface is dropped,
        // after the presenter has been dropped.
        let presenter = match unsafe {
            SurfaceProbePresenter::new_from_raw_handles(
                display,
                window_handle,
                width,
                height,
                ProbePresentMode::Fifo,
                self.gpu_time,
                backend_policy,
            )
        } {
            Ok(presenter) => presenter,
            Err(error) => {
                let message = format!("probe presenter init failed: {error}");
                self.state = BridgeState::Failed(message.clone());
                eprintln!("[probe] macos surface presenter FAILED: {error}");
                return Some(Err(message));
            }
        };
        eprintln!(
            "[probe] macos surface presenter ready generation={} backend={} adapter={} device_type={} present_mode={:?} size={}x{}",
            self.generation + 1,
            presenter.adapter().backend,
            presenter.adapter().name,
            presenter.adapter().device_type,
            presenter.present_mode(),
            width,
            height,
        );
        self.presenter = Some(presenter);
        self.state = BridgeState::Ready;
        self.generation += 1;
        if let Some(result) = self.attach_render_session() {
            return Some(result);
        }
        Some(Ok(()))
    }

    /// Bind (or clear) the CAD render session for this surface.
    ///
    /// A live presenter is first detached from the old session and the retained
    /// frame cache is cleared, then the new session is attached to the current
    /// device. With no presenter the session is stored and attached on the next
    /// successful [`Self::try_init_gpu`].
    pub fn set_render_session(
        &mut self,
        session: Option<RenderSessionHandle>,
    ) -> Result<(), String> {
        if let Some(previous) = self.session.take() {
            previous.detach_device();
        }
        if let Some(presenter) = self.presenter.as_mut() {
            presenter.clear_binding_cache();
        }
        let Some(session) = session else {
            return Ok(());
        };
        if let Some(presenter) = self.presenter.as_ref() {
            let (device, queue) = presenter.device_queue();
            if let Err(error) = session.attach_device_with_gpu_time(device, queue, self.gpu_time) {
                session.fail_device(format!("attach_device: {error}"), false);
                return Err(format!("render session attach failed: {error}"));
            }
        }
        self.session = Some(session);
        Ok(())
    }

    /// Reconfigure the presenter surface to `width`x`height` (the size authority).
    ///
    /// This is the **single** place the drawable follows the accepted CAD rect:
    /// `apply_layout` calls it after framing the view, and the frame path calls
    /// it as an idempotent backstop. A successful change logs
    /// `[probe] surface reconfigure {from} -> {to}`; a failure is a classed
    /// [`CadPresentError`] (`surface_reconfigure`), never a silent skip.
    pub fn resize_to(&mut self, width: u32, height: u32) -> Result<(), CadPresentError> {
        let Some(presenter) = self.presenter.as_mut() else {
            return Ok(());
        };
        let current = presenter.surface_size();
        let Some((from, to)) = reconfigure_plan(Some(current), (width, height)) else {
            return Ok(());
        };
        presenter.resize(to.0, to.1).map_err(|error| {
            CadPresentError::new(
                CadPresentClass::SurfaceReconfigure,
                format!(
                    "surface reconfigure {}x{} -> {}x{} failed: {error}",
                    from.0, from.1, to.0, to.1
                ),
            )
        })?;
        eprintln!(
            "[probe] surface reconfigure {}x{} -> {}x{}",
            from.0, from.1, to.0, to.1
        );
        Ok(())
    }

    /// Draw the probe marker (the `--probe` path: no CAD session).
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

    /// Draw one CAD frame through the bound render session and present it.
    ///
    /// `Ok(None)` means the engine has no new frame **and** no reusable retained
    /// frame: the caller waits — no blank or fake grid is drawn. Frame-state
    /// failures are explicit and **classified** ([`CadPresentError`]) so the host
    /// can emit a stable `[probe] cad-present-error class=…` marker instead of
    /// guessing from free text; a device loss marks the session device as failed
    /// and does not loop.
    pub fn frame_cad(
        &mut self,
        logical: [f64; 2],
        scale: f64,
    ) -> Result<Option<ProbeFrameStats>, CadPresentError> {
        if self.presenter.is_none() {
            return Err(CadPresentError::new(
                CadPresentClass::ApplyUpdates,
                "no GPU presenter (surface not ready)",
            ));
        }
        let Some(session) = self.session.clone() else {
            return Err(CadPresentError::new(
                CadPresentClass::ApplyUpdates,
                "no render session attached",
            ));
        };
        if let Err(error) = session.apply_ready_updates() {
            // A **GPU-memory refusal** is the one exception to failing the
            // device: the renderer's resident scene is intact and still renders
            // (`cad_render_wgpu::RenderError::GpuMemory`), so the device is kept
            // and the refusal carries its own `gpu_memory` class.
            let refused_for_memory = session.gpu_budget_refusal().is_some();
            if !refused_for_memory {
                session.fail_device(format!("apply_ready_updates: {error:?}"), false);
            }
            self.clear_presenter_cache();
            let class = if refused_for_memory {
                CadPresentClass::GpuMemory
            } else {
                CadPresentClass::ApplyUpdates
            };
            return Err(CadPresentError::new(
                class,
                format!("apply_ready_updates failed: {error:?}"),
            ));
        }
        let presented = match session.render(logical, scale) {
            Ok(presented) => presented,
            Err(error) => {
                if error.is_device_loss() {
                    session.fail_device(format!("render device lost: {error}"), true);
                }
                self.clear_presenter_cache();
                return Err(CadPresentError::new(
                    CadPresentClass::from_render(&error),
                    format!("render failed ({}) : {error}", render_error_class(&error)),
                ));
            }
        };
        let Some(frame) = presented else {
            // The engine returns its cached texture whenever the runtime is
            // `Ready`, so `None` means "not ready": wait. Never replay an old
            // frame and never draw a blank/fake grid.
            return Ok(None);
        };
        let key = cad_render_wgpu::NativeTextureKey {
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
            Ok(stats) => Ok(Some(stats)),
            // A bounded size disagreement: the presenter refuses to stretch a
            // mismatched texture and treats the first few consecutive ticks as a
            // transient (a mid-resize frame, or a DOM-derived size rounded one
            // pixel off). This tick simply waits.
            Err(ProbeError::TextureNotReady(_)) => Ok(None),
            Err(error) => {
                self.clear_presenter_cache();
                Err(CadPresentError::from_probe(
                    &error,
                    format!("present_texture failed: {error}"),
                ))
            }
        }
    }

    /// Stop and drop the GPU presenter (the child view stays attached so a later
    /// re-init can reuse it).
    ///
    /// Ordering: detach the session → drop the presenter. Safe to call
    /// repeatedly (there is no permanent latch; a later re-init is allowed and a
    /// repeated release is a no-op).
    pub fn release_gpu(&mut self) {
        if let Some(session) = self.session.as_ref() {
            session.detach_device();
            eprintln!("[probe] macos surface: render session detached (before GPU drop)");
        }
        self.clear_presenter_cache();
        if let Some(mut presenter) = self.presenter.take() {
            presenter.stop();
            drop(presenter);
            eprintln!(
                "[probe] macos surface: GPU released (generation {})",
                self.generation
            );
        }
        self.cad.setHidden(true);
        if !matches!(self.state, BridgeState::Closing | BridgeState::Failed(_)) {
            self.state = BridgeState::Suspended;
        }
    }

    /// Enter the terminal closing state; never inits again.
    pub fn close(&mut self) {
        self.state = BridgeState::Closing;
        self.release_gpu();
    }

    /// Attach the stored session to a just-created presenter.
    ///
    /// `None` means "no session / attached"; `Some(Err)` is a terminal failure
    /// (the session device is failed first so no clean frame can follow).
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
            // device that could not be bound to the session, so it must not
            // survive.
            self.release_gpu();
            self.state = BridgeState::Failed(message.clone());
            eprintln!("[probe] macos surface session attach FAILED (released): {error}");
            return Some(Err(message));
        }
        eprintln!(
            "[probe] macos surface: render session attached (generation {})",
            self.generation
        );
        None
    }

    fn clear_presenter_cache(&mut self) {
        if let Some(presenter) = self.presenter.as_mut() {
            presenter.clear_binding_cache();
        }
    }
}

impl Drop for MacosCadSurface {
    fn drop(&mut self) {
        // Guarantee ordering even if no explicit release ran: GPU first, then the
        // child view (the content view and window belong to Tauri).
        self.release_gpu();
        self.cad.removeFromSuperview();
    }
}

/// Locate the child WKWebView among the content view's subviews.
///
/// Tauri exposes the WKWebView pointer only through the asynchronous
/// `Webview::with_webview` callback (`PlatformWebview::inner`), which cannot be
/// used while `setup` needs the pointer synchronously. The class check is exact
/// (`isKindOfClass:WKWebView`), so an unrelated subview is never used as the
/// ordering anchor.
fn find_webview_view(content: &NSView) -> Option<NonNull<NSView>> {
    let class = AnyClass::get(c"WKWebView")?;
    let subviews = content.subviews();
    for index in 0..subviews.count() {
        let view = subviews.objectAtIndex(index);
        if view.isKindOfClass(class) {
            return NonNull::new(Retained::as_ptr(&view) as *mut NSView);
        }
    }
    None
}
