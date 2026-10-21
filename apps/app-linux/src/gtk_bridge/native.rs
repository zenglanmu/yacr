//! Webview reparenting, input pass-through and the native X11 classification
//! helpers (free functions and small private types).

use std::ffi::c_void;
use std::ptr::NonNull;

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;

use crate::wayland_surface::is_wayland_display;

use super::{
    BridgeState, OVERLAYS, REPARENT_DONE, WEBVIEW_FILL_ACTIVE, WEBVIEW_SIZE_LOG, WEBVIEW_WIDGET,
};

/// Reparent the Tauri webview into the overlay as a transparent sibling.
///
/// Runs on the main thread via `with_webview`; the closure is `Send`, so it looks
/// the overlay up in the thread-local registry instead of capturing GTK objects.
///
/// Unlike a packed child, an overlay child is not laid out by anything after the
/// reparent, so the host takes over sizing it: the widget is registered in
/// [`WEBVIEW_WIDGET`], the overlay's `size-allocate` fills it to the overlay
/// (one signal per GTK layout pass, exactly like the drawing area), and the
/// current size is applied immediately.
pub fn reparent_webview(webview: &tauri::Webview, label: &str) -> tauri::Result<()> {
    let label = label.to_string();
    webview.with_webview(move |platform| {
        let widget = platform.inner().upcast::<gtk::Widget>();
        if let Some(parent) = widget.parent() {
            if let Ok(container) = parent.downcast::<gtk::Container>() {
                container.remove(&widget);
            }
        }
        // Recreate the native window in its **final** parent. GdkWindow caches
        // its geometry and skips an X configure whose size it already believes
        // is current, so a window that was created before the reparent can be
        // left at its 1x1 creation size while every later allocation silently
        // no-ops (observed: GDK reports 960x600, `xwininfo` reports 1x1, nothing
        // paints). Unrealizing here makes the widget create a fresh window after
        // `add_overlay`, so the first allocation is the one that configures it.
        if widget.is_realized() {
            widget.unrealize();
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
                // Host-side sizing (see [`sync_webview_size`]): Tauri's
                // `auto_resize`/`set_size` never reach this widget on Linux.
                WEBVIEW_WIDGET.with(|cell| *cell.borrow_mut() = Some(widget.downgrade()));
                connect_webview_fill(&overlay, &widget);
                sync_webview_size((0, 0), "reparent");
            }
            None => eprintln!("[probe] bridge: no overlay for label {label}"),
        }
    })
}

/// Deterministically size the reparented webview to the overlay it lives in.
///
/// **Why this exists.** `WebviewBuilder::auto_resize()` and `Webview::set_size()`
/// are inert for this webview on Linux: Tauri creates a child webview inside the
/// window's `GtkBox` (`pack_start(expand=true)`), so `wry`'s `set_bounds` finds
/// neither a `GtkFixed` parent nor its own X11 child window and does nothing
/// (`wry-0.55.1/src/webkitgtk/mod.rs`: `if self.x11 … / if self.is_in_fixed_parent`).
/// The host then reparents the widget out of that box into a `GtkOverlay`, so
/// nothing resizes it at all: it keeps the 1x1 requisition it was created with.
/// This applies the size directly to the widget — the webview's counterpart of
/// the drawing-area-vs-DOM layout guard.
///
/// **Call it unconditionally from the per-frame tick.** This host does not drain
/// GTK's queued layout on its own (the drawing area only moves because
/// `apply_layout` calls `check_resize` explicitly), so an event-driven contract
/// is not available: correctness must not depend on a `size-allocate` signal. The
/// call is idempotent and cheap — one allocation compare when nothing changed —
/// and every attempt logs a `skipped=` reason, deduped per distinct state.
///
/// Runs on the GTK main thread. Off the main thread the thread-local widget is
/// `None` (thread-locals are per thread), so the call only logs
/// `skipped=not-registered` and returns. `fallback_physical` is the Tauri
/// window's inner size (physical px), used only while the container has no
/// settled allocation. Returns the logical target size that was applied (or
/// would be, when the skip is `unmapped-parent`).
pub fn sync_webview_size(
    fallback_physical: (u32, u32),
    origin: &'static str,
) -> Option<(i32, i32)> {
    let Some(widget) =
        WEBVIEW_WIDGET.with(|cell| cell.borrow().as_ref().and_then(|weak| weak.upgrade()))
    else {
        emit_webview_state(
            origin,
            format!(
                "skipped={} window_phys={}x{}",
                WebviewSkip::NotRegistered.label(),
                fallback_physical.0,
                fallback_physical.1,
            ),
        );
        return None;
    };
    let Some(parent) = widget.parent() else {
        emit_webview_state(
            origin,
            format!(
                "skipped={} window_phys={}x{}",
                WebviewSkip::NoParent.label(),
                fallback_physical.0,
                fallback_physical.1,
            ),
        );
        return None;
    };
    let scale = widget.scale_factor().max(1);
    // Physical -> logical, rounded down: GTK allocations are logical px.
    let fallback = (
        i32::try_from(fallback_physical.0).unwrap_or(i32::MAX) / scale,
        i32::try_from(fallback_physical.1).unwrap_or(i32::MAX) / scale,
    );
    let container = (parent.allocated_width(), parent.allocated_height());
    let target = webview_target_size(Some(container), fallback);
    let skipped = allocate_webview(&widget, &parent, target);
    log_webview_size(
        &widget,
        origin,
        target,
        container,
        fallback_physical,
        skipped,
    );
    Some(target)
}

/// Why a sync pass did (or did not) apply a new allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WebviewSkip {
    /// The allocation (and the map step if it was needed) was applied.
    None,
    /// The live allocation matches, the widget is mapped and its native window
    /// is viewable: nothing to do.
    SameAllocation,
    /// The reparent has not registered the widget yet.
    NotRegistered,
    /// The widget has no GTK parent (not reparented yet, or being destroyed).
    NoParent,
    /// The allocation was stored, but no native window can be configured before
    /// the container is mapped (window not shown yet).
    UnmappedParent,
    /// GTK is already inside a fill pass (not re-entrant): the outer call owns it.
    Nested,
}

impl WebviewSkip {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SameAllocation => "same-allocation",
            Self::NotRegistered => "not-registered",
            Self::NoParent => "no-parent",
            Self::UnmappedParent => "unmapped-parent",
            Self::Nested => "nested",
        }
    }
}

/// The logical size the webview widget must occupy.
///
/// The container (the overlay the React DOM fills) is authoritative once GTK has
/// allocated it. `1x1` is GTK's *unsized requisition*, not a viewport: it is what
/// a not-yet-laid-out widget reports, so it counts as unsettled and the caller's
/// fallback (the window's inner size) is used instead. A widget is never sized
/// 0x0 either — an unusable size stays an explicit 1x1 "not sized yet", which the
/// next pass corrects and the size log makes greppable.
pub(super) fn webview_target_size(
    container: Option<(i32, i32)>,
    fallback: (i32, i32),
) -> (i32, i32) {
    let (width, height) = match container {
        Some((width, height)) if width > 1 && height > 1 => (width, height),
        _ if fallback.0 > 1 && fallback.1 > 1 => fallback,
        _ => (1, 1),
    };
    (width.max(1), height.max(1))
}

/// The container's current allocation, for logging.
fn connect_webview_fill(overlay: &gtk::Overlay, widget: &gtk::Widget) {
    let weak = widget.downgrade();
    overlay.connect_size_allocate(move |overlay, allocation| {
        let Some(widget) = weak.upgrade() else {
            return;
        };
        // `size-allocate` hands us the container allocation already applied, so
        // this is the authoritative target on a GTK-driven layout pass.
        let target = webview_target_size(
            Some((allocation.width(), allocation.height())),
            (overlay.allocated_width(), overlay.allocated_height()),
        );
        let skipped = allocate_webview(&widget, overlay.upcast_ref::<gtk::Widget>(), target);
        let container = (overlay.allocated_width(), overlay.allocated_height());
        log_webview_size(
            &widget,
            "overlay-layout",
            target,
            container,
            (0, 0),
            skipped,
        );
    });
}

/// Apply `target` to the widget, mapping it first when GTK has not done so.
///
/// A reparented widget can be left unmapped even though it is visible (`show()`
/// is a no-op for an already-visible widget): GTK maps a native window only with
/// the widget, and an unmapped WebKit view paints nothing. The explicit `map()`
/// plus `size_allocate` is the pair that actually configures the native window.
fn allocate_webview(widget: &gtk::Widget, parent: &gtk::Widget, target: (i32, i32)) -> WebviewSkip {
    if WEBVIEW_FILL_ACTIVE.with(|active| active.replace(true)) {
        return WebviewSkip::Nested;
    }
    let skipped = apply_allocation(widget, parent, target);
    WEBVIEW_FILL_ACTIVE.with(|active| active.set(false));
    skipped
}

fn apply_allocation(widget: &gtk::Widget, parent: &gtk::Widget, target: (i32, i32)) -> WebviewSkip {
    let allocation = widget.allocation();
    let same = allocation.x() == 0
        && allocation.y() == 0
        && allocation.width() == target.0
        && allocation.height() == target.1;
    // A **GTK-mapped** widget is not enough: its `GdkWindow` must be viewable,
    // otherwise there is no surface on screen (this is exactly the state that
    // produced a black window: `mapped=true`, native window 1x1 at -1,-1).
    // Treating it as settled would skip forever, so it is retried instead.
    let viewable = widget.window().is_none_or(|window| window.is_viewable());
    if same && widget.is_mapped() && viewable {
        return WebviewSkip::SameAllocation;
    }
    if !widget.is_visible() {
        widget.show();
    }
    if !parent.is_mapped() {
        // Store it (GTK keeps the allocation), but nothing native can be
        // configured before the container maps; the next pass retries.
        widget.size_allocate(&gtk::Allocation::new(0, 0, target.0, target.1));
        return WebviewSkip::UnmappedParent;
    }
    // Map before allocating: GTK configures a widget's `GdkWindow` from
    // `gtk_widget_size_allocate`, and only once the widget itself is mapped.
    if widget.is_realized() && !widget.is_mapped() {
        widget.map();
    }
    widget.size_allocate(&gtk::Allocation::new(0, 0, target.0, target.1));
    if !widget.is_mapped() {
        return WebviewSkip::UnmappedParent;
    }
    WebviewSkip::None
}

/// The greppable webview-size evidence line: requested target, the widget's
/// actual allocation, its own native `GdkWindow` extent, the container
/// allocation (the DOM viewport), the map state and the skip reason. A
/// regression shows up here as a number instead of a black window.
#[allow(clippy::too_many_arguments)]
fn log_webview_size(
    widget: &gtk::Widget,
    origin: &'static str,
    target: (i32, i32),
    container: (i32, i32),
    window_physical: (u32, u32),
    skipped: WebviewSkip,
) {
    let allocation = widget.allocation();
    // `Widget::window()` falls back to the nearest windowed ancestor, so a
    // non-native webview would report the overlay's window as its own. Report
    // whether the widget owns a window and, only then, that window's extent.
    let own_window = widget.has_window();
    let native = if own_window {
        widget
            .window()
            .map(|window| (window.width(), window.height()))
    } else {
        None
    };
    // GDK geometry is parent-relative: a size that is right but a position or
    // stacking that is wrong (or an unmapped window) shows up here.
    let geometry = widget
        .window()
        .map(|window| window.geometry())
        .map(|(x, y, width, height)| format!("({x},{y},{width}x{height})"))
        .unwrap_or_else(|| "none".to_string());
    let viewable = widget
        .window()
        .map(|window| window.is_viewable())
        .unwrap_or(false);
    // The X11 XID when the widget owns an X11 window (the capture scripts and
    // `xwininfo -tree` are keyed on it).
    let xid = widget
        .window()
        .and_then(|window| window.downcast::<gdkx11::X11Window>().ok())
        .map(|window| format!("0x{:x}", window.xid()))
        .unwrap_or_else(|| "none".to_string());
    let state = widget
        .window()
        .map(|window| format!("{:?}", window.state()))
        .unwrap_or_else(|| "none".to_string());
    emit_webview_state(
        origin,
        format!(
        "requested={}x{} allocated={}x{} at=({},{}) own_window={} native={}x{} native_geom={} native_viewable={} native_xid={} native_state={} mapped={} visible={} realizable={} scale={} container={}x{} window_phys={}x{} skipped={}",
        target.0,
        target.1,
        allocation.width(),
        allocation.height(),
        allocation.x(),
        allocation.y(),
        own_window,
        native.map(|size| size.0).unwrap_or(0),
        native.map(|size| size.1).unwrap_or(0),
        geometry,
        viewable,
        xid,
        state,
        widget.is_mapped(),
        widget.is_visible(),
        widget.is_realized(),
        widget.scale_factor(),
        container.0,
        container.1,
        window_physical.0,
        window_physical.1,
        skipped.label(),
    ),
    );
}

/// Print one webview-size line unless its **state** is identical to the previous
/// one. Dedup is on the state, not on the whole line, so a `tick` and an
/// `overlay-layout` pass that observe the same state do not log twice while a
/// 60 Hz tick cannot spam; a state *change* (size, mapping, container, skip
/// reason) always logs, with the origin of the pass that saw it.
fn emit_webview_state(origin: &str, body: String) {
    let unchanged = WEBVIEW_SIZE_LOG.with(|last| last.borrow().as_deref() == Some(body.as_str()));
    if unchanged {
        return;
    }
    WEBVIEW_SIZE_LOG.with(|last| *last.borrow_mut() = Some(body.clone()));
    eprintln!("[probe] webview size ({origin}): {body}");
}

/// B3: make the native drawing area transparent to input so OS pointer/wheel
/// events reach the React DOM underneath. On X11 this uses an **empty input
/// region** (not a visual/bounding shape) plus GTK overlay pass-through,
/// re-applied on every generation/layout change. On Wayland the empty input
/// region lives on the app-owned child `wl_surface` (set in the C shim), so the
/// GDK window's shared toplevel surface is left untouched.
pub(super) fn apply_input_passthrough(overlay: &gtk::Overlay, drawing_area: &gtk::DrawingArea) {
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
pub(super) fn init_allowed(state: &BridgeState) -> bool {
    matches!(state, BridgeState::AwaitingMap | BridgeState::Suspended)
}

/// Whether a new "not ready yet" reason differs from the last one logged.
///
/// A deferral is logged once per change, so a persistently pending Wayland
/// global/selection does not spam the log but is still visible.
pub(super) fn pending_reason_changed(current: Option<&str>, reason: &str) -> bool {
    current != Some(reason)
}

/// The integer buffer scale handed to the Wayland child surface: GTK3 only
/// reports integer scale, and it is never below 1.
pub(super) fn wayland_buffer_scale(scale_factor: i32) -> i32 {
    scale_factor.max(1)
}

/// The single checked rule for a physical surface size: logical allocation ×
/// integer buffer scale.
///
/// A non-positive logical size, a scale below 1, or an `i32`/`u32` overflow is an
/// explicit `Err` — never a wrapping or saturating "success". Used by both GPU
/// init and [`GtkOverlayOwner::desired_size`].
pub(super) fn checked_physical_size(
    logical_w: i32,
    logical_h: i32,
    scale: i32,
) -> Result<(u32, u32), String> {
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

/// Whether the native drawing area occupies exactly the React DOM CAD rect.
///
/// The DOM rect is relative to the margin-receiving container (the overlay), so
/// the drawing area's **toplevel-relative** origin must equal the container's
/// toplevel origin plus the DOM rect origin, and its size must equal the DOM
/// rect size. Both are logical (CSS) pixels. The surface is blitted 1:1 and
/// pointer input maps through the DOM rect, so a disagreement stretches the
/// image or offsets the pointer. A tolerance of one logical pixel absorbs GTK's
/// integer rounding.
///
/// `None` origins mean the widgets are not realized/allocated yet: not a match
/// (the caller must wait or drive the layout), never a silent success.
pub(super) fn drawing_area_matches_dom(
    container_origin: Option<(i32, i32)>,
    drawing_area_origin: Option<(i32, i32)>,
    drawing_area_size: (i32, i32),
    dom: (f32, f32, f32, f32),
) -> bool {
    let (Some((container_x, container_y)), Some((area_x, area_y))) =
        (container_origin, drawing_area_origin)
    else {
        return false;
    };
    let close = |a: i32, b: f32| (a as f32 - b).abs() <= 1.0;
    close(area_x, container_x as f32 + dom.0)
        && close(area_y, container_y as f32 + dom.1)
        && close(drawing_area_size.0, dom.2)
        && close(drawing_area_size.1, dom.3)
}

/// A bounded, human-readable summary of a caught panic payload.
///
/// The panic message from `catch_unwind` is preserved for diagnostics but capped
/// so an adversarial/large payload cannot flood the log.
pub(super) fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
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
pub(super) fn pending_prerequisite(
    has_window: bool,
    realized: bool,
    mapped: bool,
) -> Option<&'static str> {
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
pub(super) struct NativeHandles {
    pub(super) xid: core::ffi::c_ulong,
    pub(super) display: NonNull<c_void>,
    pub(super) screen: i32,
    /// Native (logical) window geometry, recorded for evidence.
    pub(super) geom: (i32, i32),
    /// Retained so the XID/display stay valid while the surface exists.
    _window: gdk::Window,
    _display: gdkx11::X11Display,
}

/// Outcome of probing the drawing area's native X window.
pub(super) enum NativeStatus {
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
pub(super) fn classify_native(
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
