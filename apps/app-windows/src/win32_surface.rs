//! The native CAD surface: an in-process child HWND **below** the WebView2
//! layer, presented by the single renderer presenter.
//!
//! ## Composition contract
//!
//! 1. wry creates a host-process container window (class `WRY_WEBVIEW`,
//!    `WS_CHILD | WS_CLIPCHILDREN`) inside the Tauri top-level window and parents
//!    the cross-process WebView2 render windows to it. That container is the only
//!    host-owned parent we can touch; the WebView2 child windows belong to
//!    `msedgewebview2.exe` and cannot be subclassed.
//! 2. This module creates a sibling `WS_CHILD` window (class `YACR_CAD_CHILD`,
//!    `WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS`) under the same
//!    top-level window, then places it **directly below** the `WRY_WEBVIEW`
//!    container with `SetWindowPos(cad, container, …)` (creation order plus an
//!    explicit re-assert). `WS_DISABLED` is the belt-and-braces counterpart of the
//!    Wayland empty input region: the CAD surface is display-only and never takes
//!    OS input; the DOM is the single input funnel.
//! 3. The renderer surface is a **DirectComposition** surface, not a
//!    HWND-targeted swapchain: this module owns an `IDCompositionDevice`, an
//!    `IDCompositionTarget` for the CAD child HWND and its root
//!    `IDCompositionVisual`; the presenter is built from that visual
//!    ([`SurfaceProbePresenter::new_from_composition_visual`]), so wgpu-hal
//!    creates the swapchain with `CreateSwapChainForComposition` and binds it with
//!    `IDCompositionVisual::SetContent`. Why: under a transparent
//!    (`WS_EX_NOREDIRECTIONBITMAP`) top-level window a HWND flip chain
//!    (`DXGI_SWAP_EFFECT_FLIP_DISCARD`, hardcoded by wgpu-hal 30) binds to the
//!    **top-level** DWM visual, which a `WS_CHILD` does not have — its pixels are
//!    produced and presented but never composed. There are **no** other renderer
//!    changes, no readback, and exactly one presenter.
//!
//! Geometry comes only from the accepted React CAD rect. The child's **size** is
//! `cad_ui_core::layout::physical_content_size` (one rounding of `round(side ·
//! scale)`, the render session's own formula), so `child size == session render
//! target` by construction at every scale — the presenter refuses to blit a
//! size-mismatched texture, making an independently rounded size a total present
//! failure. The **offset** is `round(x · scale)`, rounded independently (a
//! half-pixel placement is invisible and never reaches the size guard). A
//! residual disagreement is never "fixed" by stretching the image.
//!
//! Lifecycle: `AwaitingMap` → `Ready` (presenter created) → `Suspended`
//! (released, re-init allowed) / `Closing` (terminal). Release order is always
//! presenter → child HWND → top-level window; `Drop` repeats the guarantee.
//!
//! Everything here runs on the host main thread (the thread that owns the native
//! windows); the value is `!Send` and lives in the host thread-local.

use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::OnceLock;

use cad_engine::render::RenderSessionHandle;
use cad_render_wgpu::{
    gpu_time::GpuTimeConfig, ProbeBackendPolicy, ProbeError, ProbeFrameStats, ProbePresentMode,
    SurfaceProbePresenter,
};
use cad_ui_core::layout::{physical_content_size, CadMetrics};
use windows::core::{w, Interface, BOOL, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionRectangleClip, IDCompositionTarget,
    IDCompositionVisual,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumChildWindows, GetClassNameW, GetClientRect,
    GetWindow, GetWindowLongPtrW, RegisterClassExW, SetWindowPos, CS_HREDRAW, CS_VREDRAW,
    GWL_EXSTYLE, GW_HWNDNEXT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_ERASEBKGND, WM_PAINT, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS,
    WS_DISABLED, WS_EX_NOREDIRECTIONBITMAP, WS_VISIBLE,
};

mod composition;
mod discovery;
mod geometry;
mod present_error;
mod styles;

use crate::composition_geometry::composition_placement;
use composition::CompositionSurface;
use discovery::find_webview_container;
use geometry::reconfigure_plan;
pub use geometry::size_authority_verdict;
use present_error::render_error_class;
pub use present_error::{CadPresentClass, CadPresentError};
use styles::{
    child_ex_style, child_no_clipsiblings_requested, child_no_redirect_requested, child_style,
    gdi_marker_requested, log_child_ex_style, paint_gdi_marker, solid_frame_requested,
};
pub use styles::{
    ex_style_is_no_redirection_bitmap, no_redirection_bitmap_state, NoRedirectionBitmapState,
};

/// Class name of the CAD child window.
const CAD_CLASS: &str = "YACR_CAD_CHILD";
/// Class name of the host-process WebView2 container created by wry 0.55.
const WEBVIEW_CONTAINER_CLASS: &str = "WRY_WEBVIEW";

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

/// Main-thread-only owner of the CAD child HWND and its presenter.
pub struct Win32CadSurface {
    _not_send_sync: PhantomData<Rc<()>>,
    top: HWND,
    cad: HWND,
    /// The wry container the CAD window must stay below (discovered after the
    /// webview exists, so it may be `None` for the first ticks).
    container: Option<HWND>,
    presenter: Option<SurfaceProbePresenter>,
    /// Physical child-window rectangle `(x, y, width, height)` last applied.
    rect: (i32, i32, i32, i32),
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
    /// The DirectComposition device/target/visual that composites the CAD child.
    ///
    /// Required, not optional decoration: an HWND-targeted flip swapchain cannot
    /// composite a `WS_CHILD` under a transparent top-level window (see the
    /// module doc). `None` means composition creation failed and the surface
    /// deliberately stays uninitialised instead of falling back to that dead path.
    dcomp: Option<CompositionSurface>,
    /// The presenter's `configure_generation` last committed to the composition
    /// tree. hal re-issues `IDCompositionVisual::SetContent` only on configure, so
    /// a generation change must be followed by `IDCompositionDevice::Commit`.
    committed_generation: u64,
    /// A layout-driven visual offset/clip change that still needs a `Commit`.
    /// Layout changes commit even when the **size** (and therefore the presenter's
    /// configure generation) did not move.
    pending_placement: Option<((f32, f32), (f32, f32))>,
}

impl Win32CadSurface {
    /// Create the CAD child window under `window` and remember the top-level HWND.
    ///
    /// The presenter is **not** created here: it waits for the accepted CAD rect
    /// (see [`Self::try_init_gpu`]). The child starts at 1×1 and is placed on the
    /// first accepted layout.
    pub fn install(window: &tauri::Window, gpu_time: GpuTimeConfig) -> Result<Self, String> {
        let top = window
            .hwnd()
            .map_err(|error| format!("window hwnd unavailable: {error}"))?;
        ensure_class()?;
        let no_redirect = child_no_redirect_requested();
        let no_clipsiblings = child_no_clipsiblings_requested();
        if no_clipsiblings {
            eprintln!(
                "[probe] cad-child style: WS_CLIPSIBLINGS dropped (diagnostic; not acceptance \
                 evidence)"
            );
        }
        let cad = unsafe {
            CreateWindowExW(
                child_ex_style(no_redirect),
                w!("YACR_CAD_CHILD"),
                PCWSTR::null(),
                child_style(no_clipsiblings),
                0,
                0,
                1,
                1,
                Some(top),
                None,
                GetModuleHandleW(PCWSTR::null()).map(Into::into).ok(),
                None,
            )
            .map_err(|error| format!("CreateWindowExW({CAD_CLASS}) failed: {error}"))?
        };
        if no_redirect {
            log_child_ex_style(cad);
        }
        let container = find_webview_container(top);
        // The CAD child is composited through DirectComposition, never through a
        // HWND-targeted swapchain: under a transparent top-level window a flip
        // chain binds to the top-level DWM visual, which a `WS_CHILD` does not
        // have (the pixels are produced and presented but never composed).
        let (dcomp, state) = match CompositionSurface::new(top) {
            Ok(dcomp) => {
                eprintln!(
                    "[probe] win32 cad-composition: device+target+visual ready \
                     (composed swapchain path; surface=composition target={})",
                    dcomp.target_kind().as_token()
                );
                (Some(dcomp), BridgeState::AwaitingMap)
            }
            Err(error) => {
                eprintln!("[probe] win32 cad-composition FAILED: {error}");
                (
                    None,
                    BridgeState::Failed(format!("CAD composition unavailable: {error}")),
                )
            }
        };
        let mut surface = Self {
            _not_send_sync: PhantomData,
            top,
            cad,
            container,
            presenter: None,
            rect: (0, 0, 1, 1),
            state,
            generation: 0,
            session: None,
            gpu_time,
            order_logged: false,
            order_failure_logged: false,
            dcomp,
            committed_generation: 0,
            pending_placement: None,
        };
        surface.log_z_order("install");
        if solid_frame_requested() {
            eprintln!("[probe] solid-frame active (diagnostic; not acceptance evidence)");
        }
        if gdi_marker_requested() {
            eprintln!(
                "[probe] gdi-marker active: 40x40 red square at the CAD child's top-left \
                 (diagnostic; not acceptance evidence)"
            );
            // The child is display-only and may never receive a first paint on its
            // own; force one so the calibrator is present in the next capture.
            unsafe {
                let _ = InvalidateRect(Some(cad), None, true);
            }
        }
        Ok(surface)
    }

    /// The physical CAD rectangle last applied (also the desired surface size).
    pub fn desired_size(&self) -> (u32, u32) {
        (self.rect.2.max(0) as u32, self.rect.3.max(0) as u32)
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

    /// Discover the wry container if it was not there yet and re-assert the
    /// CAD-below-WebView order when it drifted.
    ///
    /// Idempotent and cheap: a steady order performs one `GetWindow` comparison.
    pub fn ensure_order(&mut self) {
        if self.container.is_none() {
            self.container = find_webview_container(self.top);
        }
        let Some(container) = self.container else {
            return;
        };
        let below = unsafe { GetWindow(container, GW_HWNDNEXT).ok() };
        if below == Some(self.cad) {
            return;
        }
        // Re-assert: place the CAD window directly below the webview container
        // without touching its current geometry or activation.
        let result = unsafe {
            SetWindowPos(
                self.cad,
                Some(container),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        };
        match result {
            Ok(()) => {
                self.order_failure_logged = false;
                self.log_z_order("re-assert");
            }
            Err(error) => {
                // Persistent failure (e.g. the container vanished) would
                // otherwise print on every tick; report once per failure state
                // and reset on recovery.
                if !self.order_failure_logged {
                    eprintln!(
                        "[probe] win32 z-order re-assert FAILED \
                         (suppressing repeats until recovery): {error}"
                    );
                    self.order_failure_logged = true;
                }
            }
        }
    }

    /// Apply the accepted DOM layout: size the child from the CAD content size
    /// authority and place it at the scaled DOM offset.
    ///
    /// The **size** is [`physical_content_size`] — the render session's own
    /// `round(side · scale)` formula — so the child window and the render target
    /// are the same pixel count by construction. It is deliberately **not**
    /// derived by subtracting independently rounded insets
    /// (`round(container·s) − round(start·s) − round(end·s)`): that second
    /// rounding path can disagree by a pixel at fractional DPI, and the
    /// presenter's size guard then refuses *every* frame (transparent CAD region
    /// plus a sticky `cad present failed`). Only the offset keeps its own
    /// rounding.
    ///
    /// Placement **waits** while the wry container is unknown: `hWndInsertAfter =
    /// NULL` is `HWND_TOP`, which would put the CAD child above the DOM — the
    /// exact inversion this composition forbids. The desired rect is still
    /// recorded, so [`Self::try_init_gpu`] can wait for the same container.
    pub fn apply_layout(&mut self, rect: &CadMetrics, scale: f64) -> Result<(), CadPresentError> {
        let scale = scale.max(0.1);
        let (width, height) = physical_content_size(rect, scale);
        // Independent rounding of the offset is fine: a half-pixel placement is
        // invisible and never reaches the presenter's size guard.
        let x = (f64::from(rect.x) * scale).round().max(0.0) as i32;
        let y = (f64::from(rect.y) * scale).round().max(0.0) as i32;
        self.rect = (
            x,
            y,
            i32::try_from(width).unwrap_or(i32::MAX).max(1),
            i32::try_from(height).unwrap_or(i32::MAX).max(1),
        );
        self.container = self.container.or_else(|| find_webview_container(self.top));
        let Some(container) = self.container else {
            // Wait for the wry container: passing `NULL` here would mean
            // `HWND_TOP` and invert the CAD-below-DOM contract. The child is
            // still empty/1x1 and the presenter refuses to initialize without a
            // container, so skipping the placement has no visual consequence.
            return Ok(());
        };
        let (x, y, w, h) = self.rect;
        // `hWndInsertAfter = container` puts the CAD window immediately **below**
        // the WebView container in the sibling z-order.
        unsafe {
            SetWindowPos(
                self.cad,
                Some(container),
                x,
                y,
                w,
                h,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        }
        .map_err(|error| {
            CadPresentError::new(
                CadPresentClass::SurfaceReconfigure,
                format!("SetWindowPos(cad child) failed: {error}"),
            )
        })?;
        self.log_z_order("layout");
        // The size authority must hold **at runtime**, not only at init: the
        // swapchain is reconfigured here, in the same step that places the child,
        // so the geometry snapshot taken right after this call already reports
        // `native_surface == child_desired == dom_physical`. (The frame path also
        // calls `resize_to` as an idempotent backstop.)
        self.apply_composition_placement(rect, scale)?;
        self.resize_to(width, height)?;
        // One commit for both changes (placement and/or reconfigure). A layout
        // that only *moves* the CAD rect changes no size, so the presenter's
        // configure generation does not move — hence the explicit pending
        // placement, or the visual would stay at the old offset forever.
        self.sync_composition()
    }

    /// Position/clip the composition visual to the accepted CAD rect (DIPs).
    ///
    /// Records the placement as pending so `sync_composition` commits it, even
    /// when the swapchain size (and its configure generation) is unchanged.
    fn apply_composition_placement(
        &mut self,
        rect: &CadMetrics,
        scale: f64,
    ) -> Result<(), CadPresentError> {
        let Some(dcomp) = self.dcomp.as_ref() else {
            return Err(CadPresentError::new(
                CadPresentClass::SurfaceComposition,
                "DirectComposition is unavailable for the CAD content",
            ));
        };
        let placement = composition_placement(rect, scale);
        dcomp
            .set_placement(placement.offset, placement.clip)
            .map_err(|error| {
                CadPresentError::new(
                    CadPresentClass::SurfaceComposition,
                    format!("composition placement failed: {error}"),
                )
            })?;
        self.pending_placement = Some((placement.offset, placement.clip));
        eprintln!(
            "[probe] composition placement offset=({:.1},{:.1}) clip={:.1}x{:.1} dip \
             swapchain={}x{}",
            placement.offset.0,
            placement.offset.1,
            placement.clip.0,
            placement.clip.1,
            placement.physical_size.0,
            placement.physical_size.1,
        );
        Ok(())
    }

    /// Deferred GPU init: create the presenter once a confirmed CAD rect yields a
    /// non-zero physical size and the WebView container exists.
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
        if self.container.is_none() {
            self.container = find_webview_container(self.top);
            self.container?;
        }
        // Without the composition tree there is no way to composite the CAD
        // child at all; fail explicitly instead of presenting into a swapchain
        // that can never appear (no silent fallback to the dead HWND path).
        let Some(visual) = self.dcomp.as_ref().map(CompositionSurface::visual_ptr) else {
            let message = format!(
                "CAD composition is unavailable, so the presentation surface cannot be created \
                 (the HWND flip-swapchain path cannot composite under a transparent top-level \
                 window): {}",
                self.failure().unwrap_or("composition was never created")
            );
            self.state = BridgeState::Failed(message.clone());
            return Some(Err(message));
        };
        // Durable `wgpu_backend` preference, read once per presenter build
        // (never per frame) through the host's own preference store. An absent
        // or unreadable preference resolves to the renderer's `auto`.
        let backend_policy =
            ProbeBackendPolicy::parse(crate::host::wgpu_backend_preference().as_deref());
        // SAFETY: the caller contract of `new_from_composition_visual` is
        // satisfied: `visual` is a live `IDCompositionVisual` owned by `self.dcomp`
        // whose target is this CAD child HWND (created above on this thread); the
        // device/target/visual stay alive until this presenter is dropped (release
        // order in `release_gpu`/`Drop`: presenter → composition → child HWND),
        // and construction/frame/resize/drop all happen on the host main thread.
        let presenter = match unsafe {
            SurfaceProbePresenter::new_from_composition_visual(
                visual,
                width,
                height,
                ProbePresentMode::MailboxElseImmediate,
                self.gpu_time,
                backend_policy,
            )
        } {
            Ok(presenter) => presenter,
            Err(error) => {
                let message = format!("probe presenter init failed: {error}");
                self.state = BridgeState::Failed(message.clone());
                eprintln!("[probe] win32 surface presenter FAILED: {error}");
                return Some(Err(message));
            }
        };
        eprintln!(
            "[probe] win32 surface presenter ready generation={} backend={} adapter={} device_type={} present_mode={:?} size={}x{} surface={} target={}",
            self.generation + 1,
            presenter.adapter().backend,
            presenter.adapter().name,
            presenter.adapter().device_type,
            presenter.present_mode(),
            width,
            height,
            presenter.surface_kind().as_token(),
            self.dcomp
                .as_ref()
                .map(|dcomp| dcomp.target_kind().as_token())
                .unwrap_or("none"),
        );
        self.presenter = Some(presenter);
        self.state = BridgeState::Ready;
        self.generation += 1;
        // hal already issued `SetContent` for the construction configure; the
        // composition tree only picks it up on `Commit`.
        if let Err(error) = self.sync_composition() {
            // Without a committed tree the content can never appear; release and
            // fail the surface explicitly instead of presenting into a black
            // region (no silent fallback to the dead HWND swapchain path).
            self.release_gpu();
            self.state = BridgeState::Failed(error.text.clone());
            return Some(Err(error.text));
        }
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
    /// This is the **single** place the swapchain follows the accepted CAD rect:
    /// `apply_layout` calls it after moving the child window, and the frame path
    /// calls it as an idempotent backstop. A successful change logs
    /// `[probe] surface reconfigure {from} -> {to}`; a failure is a classed
    /// [`CadPresentError`] (`surface_reconfigure`), never a silent skip — a
    /// surface frozen at the old size makes the presenter's size guard refuse
    /// every frame, which is exactly the "transparent/black CAD region with no
    /// present error" symptom.
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
        // The reconfigure created a new composition swapchain and hal re-issued
        // `SetContent`; the caller commits (`apply_layout` for layout changes,
        // `frame_cad` for the per-frame backstop path).
        Ok(())
    }

    /// Commit the composition tree when the presenter (re)configured.
    ///
    /// `Surface::configure` is where hal re-issues
    /// `IDCompositionVisual::SetContent`; the change only becomes visible after
    /// `IDCompositionDevice::Commit`. The presenter exposes its configure
    /// generation, so this is a no-op on ordinary ticks and exactly one COM call
    /// after init / resize / an internal `Outdated` re-configure. A failure is a
    /// classed `surface_composition` error (the content would never appear).
    fn sync_composition(&mut self) -> Result<(), CadPresentError> {
        let Some(dcomp) = self.dcomp.as_ref() else {
            return Err(CadPresentError::new(
                CadPresentClass::SurfaceComposition,
                "DirectComposition is unavailable for the CAD child",
            ));
        };
        let Some(presenter) = self.presenter.as_ref() else {
            return Ok(());
        };
        let generation = presenter.configure_generation();
        let placement_pending = self.pending_placement.is_some();
        if generation == self.committed_generation && !placement_pending {
            return Ok(());
        }
        dcomp.commit().map_err(|error| {
            CadPresentError::new(
                CadPresentClass::SurfaceComposition,
                format!(
                    "composition commit failed (configure generation {generation}, \
                     pending placement: {placement_pending}): {error}"
                ),
            )
        })?;
        self.committed_generation = generation;
        self.pending_placement = None;
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
    ///
    /// With the presence-based `YACR_PROBE_SOLID_FRAME` diagnostic knob this
    /// skips the texture blit **and** the size guard and presents a solid
    /// alternating frame instead (see
    /// [`cad_render_wgpu::SurfaceProbePresenter::present_solid`]).
    pub fn frame_cad(
        &mut self,
        logical: [f64; 2],
        scale: f64,
    ) -> Result<Option<ProbeFrameStats>, CadPresentError> {
        let result = self.frame_cad_inner(logical, scale);
        // A present path may have reconfigured internally (hal retries
        // `Surface::configure` on `Outdated`); commit before reporting the tick.
        match self.sync_composition() {
            Ok(()) => result,
            // Never mask a more specific frame failure with the commit error:
            // the frame error is reported now and the commit error on the next
            // tick that gets far enough to try again.
            Err(_) if result.is_err() => result,
            Err(error) => Err(error),
        }
    }

    fn frame_cad_inner(
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
        // Diagnostic discrimination knob: no texture, no size guard.
        if solid_frame_requested() {
            let presenter = self.presenter.as_mut().expect("presenter checked above");
            return match presenter.present_solid() {
                Ok(stats) => Ok(Some(stats)),
                Err(error) => Err(CadPresentError::from_probe(
                    &error,
                    format!("solid frame failed: {error}"),
                )),
            };
        }
        let Some(session) = self.session.clone() else {
            return Err(CadPresentError::new(
                CadPresentClass::ApplyUpdates,
                "no render session attached",
            ));
        };
        if let Err(error) = session.apply_ready_updates() {
            session.fail_device(format!("apply_ready_updates: {error:?}"), false);
            self.clear_presenter_cache();
            return Err(CadPresentError::new(
                CadPresentClass::ApplyUpdates,
                format!("apply_ready_updates failed: {error:?}"),
            ));
        }
        let presented = match session.render(logical, scale) {
            Ok(presented) => presented,
            Err(error) => {
                let device_loss = error.is_device_loss();
                if device_loss {
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

    /// Stop and drop the GPU presenter **before** the child window is destroyed.
    ///
    /// Ordering: detach the session → drop the presenter → destroy the child HWND.
    /// Safe to call repeatedly (there is no permanent latch; a later re-init is
    /// allowed and a repeated release is a no-op).
    pub fn release_gpu(&mut self) {
        if let Some(session) = self.session.as_ref() {
            session.detach_device();
            eprintln!("[probe] win32 surface: render session detached (before GPU drop)");
        }
        self.clear_presenter_cache();
        if let Some(mut presenter) = self.presenter.take() {
            presenter.stop();
            drop(presenter);
            eprintln!(
                "[probe] win32 surface: GPU released (generation {})",
                self.generation
            );
        }
        if !self.cad.0.is_null() {
            // Destroying the child here keeps the "presenter before HWND"
            // invariant even when the top-level window is about to go away.
            unsafe {
                let _ = DestroyWindow(self.cad);
            }
            self.cad = HWND::default();
        }
        // The composition target references the child HWND that was just
        // destroyed. Dropping the tree here means a later re-init fails with an
        // explicit `surface_composition` error instead of composing into a dead
        // window (there is no silent fallback to a HWND swapchain).
        self.dcomp = None;
        self.committed_generation = 0;
        self.pending_placement = None;
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
            eprintln!("[probe] win32 surface session attach FAILED (released): {error}");
            return Some(Err(message));
        }
        eprintln!(
            "[probe] win32 surface: render session attached (generation {})",
            self.generation
        );
        None
    }

    fn clear_presenter_cache(&mut self) {
        if let Some(presenter) = self.presenter.as_mut() {
            presenter.clear_binding_cache();
        }
    }

    /// Print the sibling order once per change (evidence only).
    fn log_z_order(&mut self, origin: &str) {
        let Some(container) = self.container else {
            if !self.order_logged {
                self.order_logged = true;
                eprintln!(
                    "[probe] win32 z-order ({origin}): container=absent cad={:?} cad_below_container=unknown",
                    self.cad
                );
            }
            return;
        };
        let below = unsafe { GetWindow(container, GW_HWNDNEXT).ok() };
        let cad_below = below == Some(self.cad);
        let body = format!(
            "[probe] win32 z-order ({origin}): top={:?} container={:?} cad={:?} below_container={cad_below} cad_rect={:?}x{:?}",
            self.top, container, self.cad, self.rect.2, self.rect.3
        );
        if self.order_logged && cad_below {
            return;
        }
        self.order_logged = true;
        eprintln!("{body}");
    }
}

impl Drop for Win32CadSurface {
    fn drop(&mut self) {
        // Guarantee ordering even if no explicit release ran: GPU first, then the
        // child HWND (the top-level window belongs to Tauri).
        self.release_gpu();
    }
}

/// The physical client-area size of `top` (window/WebView container bounds).
pub fn client_size(top: HWND) -> Option<(i32, i32)> {
    let mut rect = windows::Win32::Foundation::RECT::default();
    unsafe { GetClientRect(top, &mut rect) }.ok()?;
    Some((rect.right - rect.left, rect.bottom - rect.top))
}

/// Register the CAD window class once per process.
fn ensure_class() -> Result<(), String> {
    static CLASS: OnceLock<Result<(), String>> = OnceLock::new();
    CLASS
        .get_or_init(|| {
            let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }
                .map_err(|error| format!("GetModuleHandleW failed: {error}"))?;
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(cad_window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: HINSTANCE(instance.0),
                hIcon: Default::default(),
                hCursor: Default::default(),
                hbrBackground: Default::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: w!("YACR_CAD_CHILD"),
                hIconSm: Default::default(),
            };
            let atom = unsafe { RegisterClassExW(&class) };
            if atom == 0 {
                let error = unsafe { GetLastError() };
                if error != ERROR_CLASS_ALREADY_EXISTS {
                    return Err(format!("RegisterClassExW({CAD_CLASS}) failed: {error:?}"));
                }
            }
            Ok(())
        })
        .clone()
}

/// The child-window procedure: the presenter paints the surface, so the background
/// is never erased and `WM_PAINT` is acknowledged without GDI drawing.
unsafe extern "system" fn cad_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            // Diagnostic V1: a GDI marker proves whether OUR child-window content
            // reaches the composited output / shows up in a capture at all. It is
            // painted only under the knob and never touches the swapchain path.
            if gdi_marker_requested() {
                paint_gdi_marker(hwnd);
            }
            LRESULT(1)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let _ = unsafe { BeginPaint(hwnd, &mut paint) };
            let _ = unsafe { EndPaint(hwnd, &paint) };
            if gdi_marker_requested() {
                paint_gdi_marker(hwnd);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
