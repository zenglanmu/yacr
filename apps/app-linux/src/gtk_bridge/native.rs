//! Webview reparenting, input pass-through and the native X11 classification
//! helpers (free functions and small private types).

use std::ffi::c_void;
use std::ptr::NonNull;

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;

use crate::wayland_surface::is_wayland_display;

use super::{BridgeState, OVERLAYS, REPARENT_DONE};

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
