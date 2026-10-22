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
//! 3. The renderer surface is built from raw `Win32WindowHandle` /
//!    `WindowsDisplayHandle` handles ([`SurfaceProbePresenter::new_from_raw_handles`]);
//!    there are **no renderer changes**, no readback, and exactly one presenter.
//!
//! Geometry comes only from the accepted React CAD rect: `LayoutTracker` insets
//! (logical px) are scaled to physical pixels and applied to the child window.
//! A residual disagreement is never "fixed" by stretching the image.
//!
//! Lifecycle: `AwaitingMap` → `Ready` (presenter created) → `Suspended`
//! (released, re-init allowed) / `Closing` (terminal). Release order is always
//! presenter → child HWND → top-level window; `Drop` repeats the guarantee.
//!
//! Everything here runs on the host main thread (the thread that owns the native
//! windows); the value is `!Send` and lives in the host thread-local.

use std::marker::PhantomData;
use std::num::NonZeroIsize;
use std::rc::Rc;
use std::sync::OnceLock;

use cad_engine::render::RenderSessionHandle;
use cad_render_wgpu::{
    gpu_time::GpuTimeConfig, ProbeError, ProbeFrameStats, ProbePresentMode, SurfaceProbePresenter,
};
use cad_ui_core::layout::ViewportInsets;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, ERROR_SUCCESS, HINSTANCE, HWND, LPARAM, LRESULT,
    WPARAM,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumChildWindows, GetClassNameW, GetClientRect,
    GetWindow, GetWindowLongPtrW, RegisterClassExW, SetWindowLongPtrW, SetWindowPos, CS_HREDRAW,
    CS_VREDRAW, GWL_EXSTYLE, GW_HWNDNEXT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER,
    SWP_NOSIZE, WINDOW_EX_STYLE, WM_ERASEBKGND, WM_PAINT, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS,
    WS_DISABLED, WS_EX_NOREDIRECTIONBITMAP, WS_VISIBLE,
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
        let cad = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("YACR_CAD_CHILD"),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS,
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
        let container = find_webview_container(top);
        let mut surface = Self {
            _not_send_sync: PhantomData,
            top,
            cad,
            container,
            presenter: None,
            rect: (0, 0, 1, 1),
            state: BridgeState::AwaitingMap,
            generation: 0,
            session: None,
            gpu_time,
            order_logged: false,
            order_failure_logged: false,
        };
        surface.log_z_order("install");
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

    /// Apply the accepted DOM layout: scale the logical insets to physical pixels
    /// and place the child window.
    ///
    /// Placement **waits** while the wry container is unknown: `hWndInsertAfter =
    /// NULL` is `HWND_TOP`, which would put the CAD child above the DOM — the
    /// exact inversion this composition forbids. The desired rect is still
    /// recorded, so [`Self::try_init_gpu`] can wait for the same container.
    pub fn apply_layout(
        &mut self,
        insets: ViewportInsets,
        container_logical: (f32, f32),
        scale: f64,
    ) -> Result<(), String> {
        let scale = scale.max(0.1);
        let container_w = (f64::from(container_logical.0) * scale).round() as i32;
        let container_h = (f64::from(container_logical.1) * scale).round() as i32;
        let start = (f64::from(insets.start) * scale).round() as i32;
        let top = (f64::from(insets.top) * scale).round() as i32;
        let end = (f64::from(insets.end) * scale).round() as i32;
        let bottom = (f64::from(insets.bottom) * scale).round() as i32;
        let width = (container_w - start - end).max(1);
        let height = (container_h - top - bottom).max(1);
        self.rect = (start, top, width, height);
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
        .map_err(|error| format!("SetWindowPos(cad child) failed: {error}"))?;
        self.log_z_order("layout");
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
        let display = RawDisplayHandle::Windows(WindowsDisplayHandle::new());
        let window_handle = match NonZeroIsize::new(self.cad.0 as isize) {
            Some(hwnd) => RawWindowHandle::Win32(Win32WindowHandle::new(hwnd)),
            None => {
                let message = "CAD child HWND is null".to_string();
                self.state = BridgeState::Failed(message.clone());
                return Some(Err(message));
            }
        };
        // SAFETY: the caller contract of `new_from_raw_handles` is satisfied:
        // both handles identify the same live child HWND (created above on this
        // thread), the window stays alive and keeps its identity until this
        // presenter is dropped (release order in `release_gpu`/`Drop`), this is
        // the single presenter of that drawable, and construction/frame/resize/
        // drop all happen on the host main thread.
        let presenter = match unsafe {
            SurfaceProbePresenter::new_from_raw_handles(
                display,
                window_handle,
                width,
                height,
                ProbePresentMode::MailboxElseImmediate,
                self.gpu_time,
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
            "[probe] win32 surface presenter ready generation={} backend={} adapter={} device_type={} present_mode={:?} size={}x{}",
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

    /// Resize the presenter surface if the child geometry changed.
    pub fn resize_to(&mut self, width: u32, height: u32) -> Result<(), String> {
        let Some(presenter) = self.presenter.as_mut() else {
            return Ok(());
        };
        if presenter.surface_size() == (width, height) {
            return Ok(());
        }
        presenter
            .resize(width, height)
            .map_err(|error| format!("probe resize failed: {error}"))
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
    /// failures are explicit; a device loss marks the session device as failed and
    /// does not loop.
    pub fn frame_cad(
        &mut self,
        logical: [f64; 2],
        scale: f64,
    ) -> Result<Option<ProbeFrameStats>, String> {
        let Some(session) = self.session.clone() else {
            return Err("no render session attached".into());
        };
        if self.presenter.is_none() {
            return Err("no GPU presenter (surface not ready)".into());
        }
        if let Err(error) = session.apply_ready_updates() {
            session.fail_device(format!("apply_ready_updates: {error:?}"), false);
            self.clear_presenter_cache();
            return Err(format!("apply_ready_updates failed: {error:?}"));
        }
        let presented = match session.render(logical, scale) {
            Ok(presented) => presented,
            Err(error) => {
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
                Err(format!("present_texture failed: {error}"))
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

/// Apply `WS_EX_NOREDIRECTIONBITMAP` to the top-level window (best effort).
///
/// Tauri 2.11.6 has no `noRedirectionBitmap` builder option (it landed in
/// Tauri 2.12), so the host sets the ex-style explicitly before the window is
/// shown. This is a **best-effort** mitigation for the creation white flash: it
/// runs after tao created the window, so tao's own creation-time branch cannot
/// see it. The result is reported honestly (and remains NOT VERIFIED on Wine).
pub fn set_no_redirection_bitmap(top: HWND) -> Result<(), String> {
    let current = unsafe { GetWindowLongPtrW(top, GWL_EXSTYLE) };
    let requested = current | (WS_EX_NOREDIRECTIONBITMAP.0 as isize);
    if requested == current {
        return Ok(());
    }
    let previous = unsafe { SetWindowLongPtrW(top, GWL_EXSTYLE, requested) };
    if previous == 0 {
        let error = unsafe { GetLastError() };
        if error != ERROR_SUCCESS {
            return Err(format!(
                "SetWindowLongPtrW(WS_EX_NOREDIRECTIONBITMAP) failed: {error:?}"
            ));
        }
    }
    Ok(())
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
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let _ = unsafe { BeginPaint(hwnd, &mut paint) };
            let _ = unsafe { EndPaint(hwnd, &paint) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Find the wry-created `WRY_WEBVIEW` container among the top-level children.
///
/// wry 0.55 exposes the WebView2 controller/environment but not the container
/// HWND, so the container is discovered by class name.
fn find_webview_container(top: HWND) -> Option<HWND> {
    struct Search {
        found: Option<HWND>,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `&mut Search` passed by `find_webview_container`
        // and this callback only runs inside that `EnumChildWindows` call.
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        if window_class(hwnd).as_deref() == Some(WEBVIEW_CONTAINER_CLASS) {
            search.found = Some(hwnd);
            BOOL(0)
        } else {
            BOOL(1)
        }
    }

    let mut search = Search { found: None };
    unsafe {
        let _ = EnumChildWindows(
            Some(top),
            Some(callback),
            LPARAM(std::ptr::addr_of_mut!(search) as isize),
        );
    }
    search.found
}

/// The window class name of `hwnd`, if readable.
fn window_class(hwnd: HWND) -> Option<String> {
    let mut buffer = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    (len > 0).then(|| String::from_utf16_lossy(&buffer[..len as usize]))
}

/// The stable class of a render error, for explicit reporting.
fn render_error_class(error: &cad_render_wgpu::RenderError) -> &'static str {
    match error {
        cad_render_wgpu::RenderError::DeviceLost(_) => "device_lost",
        cad_render_wgpu::RenderError::Frame(_) => "frame",
        cad_render_wgpu::RenderError::NotInitialized(_) => "not_initialized",
    }
}
