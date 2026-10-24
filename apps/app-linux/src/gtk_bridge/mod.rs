//! GTK overlay bridge: native Wayland CAD surface **below** the DOM/WebView.
//!
//! **Root cause this solves:** creating the wgpu Vulkan surface on the Tauri/GTK
//! **top-level** window makes the Vulkan swapchain cover the React webview.
//!
//! **Production arrangement (the only one, since 2026-10-22):** the React WebView
//! is the top layer and the native CAD surface is below it. On native Wayland the
//! app-owned CAD `wl_subsurface` is always `place_below(parent)`, so the DOM is
//! the only visible chrome layer; the GDK toplevel's alpha is cleared on every
//! GTK draw so the CAD child shows through. There is no mode switch and **no
//! top-level fallback**. Input keeps its single funnel: `.native-input` in the
//! DOM; the CAD surface keeps an empty input region as defence in depth.
//!
//! Backend paths behind the same owner:
//! - **Wayland** ([`WaylandSurface`]): the drawing area is a non-native GDK child
//!   that shares the GDK toplevel `wl_surface`, so the bridge owns a **child
//!   `wl_surface` + `wl_subsurface`** (the app-owned region) and hands *that* to
//!   the renderer. The GDK surface is never used for Vulkan, never committed by
//!   the host, and never placed above the child.
//! - **X11** (`classify_native`): a windowed `GtkDrawingArea` with its own native
//!   X child window. This path is **NOT on the production path** and is not even
//!   installed on X11: `backend::prepare_backend` resolves X11/XWayland to
//!   `surfaceMode=dom-canvas` (ADR 0024), and `host::init` skips the whole bridge
//!   there, so this code is unreachable at runtime. It is retained, and
//!   unit-tested, pending an explicit cleanup decision — never as X11 capability
//!   evidence, and never as a fallback for a failed canvas.
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
use std::marker::PhantomData;
use std::rc::Rc;

use cad_engine::render::{FrameCompleteness, RenderSessionHandle};
use cad_render_wgpu::{gpu_time::GpuTimeConfig, ProbeStageTimings, SurfaceProbePresenter};
use gtk::prelude::*;

use crate::wayland_surface::WaylandSurface;

mod native;
mod owner;
mod present;
mod stacking_regression;

#[cfg(test)]
mod tests;

use native::NativeHandles;

// Unchanged external surface (`crate::gtk_bridge::…` still resolves).
pub use native::{reparent_webview, sync_webview_size};

thread_local! {
    /// Overlays keyed by Tauri window label; `with_webview` closures must be
    /// `Send`, so they look the overlay up here instead of capturing GTK objects.
    static OVERLAYS: RefCell<HashMap<String, gtk::Overlay>> = RefCell::new(HashMap::new());
    /// Set once the webview has been reparented into the overlay.
    static REPARENT_DONE: Cell<bool> = const { Cell::new(false) };
    /// The reparented webview widget (weak; main-thread only).
    ///
    /// GTK objects are `!Send`, so this lives beside [`OVERLAYS`] instead of in
    /// the shared host state. A weak reference is used because the widget is
    /// owned by the Tauri webview: this must never keep a destroyed webview (or
    /// its WebKit process) alive after the window goes away.
    static WEBVIEW_WIDGET: RefCell<Option<gtk::glib::WeakRef<gtk::Widget>>> =
        const { RefCell::new(None) };
    /// Re-entrancy guard for "overlay size-allocate fills the webview": GTK is
    /// not re-entrant, and allocating the webview can queue another layout pass.
    static WEBVIEW_FILL_ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// The last emitted `webview size` line, so repeated layout passes with the
    /// same numbers stay silent (a change still logs).
    static WEBVIEW_SIZE_LOG: RefCell<Option<String>> = const { RefCell::new(None) };
    /// The last emitted GPU-memory refusal line, so a refused upload that is
    /// retried on the next tick cannot spam the log (a different refusal logs).
    static GPU_REFUSAL_LOG: RefCell<Option<String>> = const { RefCell::new(None) };
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
    /// Measurement-only GPU frame-time opt-in (ADR 0013 class 1), derived from
    /// the host environment and threaded to device creation and the render
    /// session. `Disabled` on the product default.
    gpu_time: GpuTimeConfig,
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
    /// Overlay origin translated into the toplevel, logical px (`None` until
    /// both widgets are realized/allocated). The DOM CAD rect is relative to
    /// this container, so the drawing area's toplevel origin must equal this
    /// plus the DOM rect origin for pointer input to line up.
    pub container_origin: Option<(i32, i32)>,
    /// Drawing area allocation, logical px.
    pub drawing_area: (i32, i32),
    /// Drawing area origin translated into the toplevel, logical px (`None` until
    /// both widgets are realized/allocated).
    pub origin: Option<(i32, i32)>,
    /// GTK integer scale factor (>= 1).
    pub scale: i32,
}

/// Install the bridge: build the overlay + windowed drawing area, realize it,
/// and register it for the webview reparent. GPU init is deferred. `gpu_time` is
/// the measurement-only opt-in (ADR 0013 class 1) the host derived from its
/// environment gate; the bridge never reads the environment itself.
pub fn install(
    window: &tauri::Window,
    label: &str,
    gpu_time: GpuTimeConfig,
) -> Result<GtkOverlayOwner, String> {
    GtkOverlayOwner::install(window, label, gpu_time)
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
