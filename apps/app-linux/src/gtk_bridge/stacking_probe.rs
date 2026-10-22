//! Bounded native stacking attempt, not a compositor-success assertion.

use std::cell::RefCell;
use std::ffi::{c_int, c_uint, c_ulong, c_void};

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;

thread_local! {
    static LAST: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn report(message: String) {
    LAST.with(|last| {
        if last.borrow().as_ref() != Some(&message) {
            eprintln!("[dom-on-top] {message}");
            *last.borrow_mut() = Some(message);
        }
    });
}

pub(super) fn prepare_parent(top: &gtk::ApplicationWindow) {
    // GTK still owns this drawable and all commits. Clear via GTK's draw cycle,
    // never by submitting Vulkan to the borrowed parent surface.
    top.connect_draw(|widget, context| {
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
        top.is_app_paintable(), gtk::prelude::WidgetExt::screen(top).is_some_and(|screen| screen.rgba_visual().is_some()),
    );
}

pub(super) fn establish(widget: &gtk::Widget, parent: &gtk::Widget) {
    if crate::wayland_surface::is_wayland_display(&widget.display()) {
        if let Some(window) = widget.toplevel().and_then(|top| top.window()) {
            let visual = window.visual();
            report(format!(
                "wayland GTK-parent depth={} visual={:?} webview_has_window={} webview_native={} alpha_pixels=NOT_VERIFIED opaque_region_policy=clear-via-GTK-draw actual_protocol_region=NOT_OBSERVABLE composite=NOT_VERIFIED",
                visual.depth(), visual.visual_type(), widget.has_window(),
                widget.window().is_some_and(|window| window.has_native()),
            ));
        }
        return;
    }
    let Some(overlay) = parent.downcast_ref::<gtk::Overlay>() else {
        report("x11 Unsupported: WebView parent is not the intended GTK overlay".into());
        return;
    };
    let Some(cad) = overlay
        .children()
        .into_iter()
        .find_map(|child| child.downcast::<gtk::DrawingArea>().ok())
    else {
        report("x11 Unsupported: no CAD sibling in GTK overlay".into());
        return;
    };
    if !widget.is_realized() || !cad.is_realized() {
        report("x11 arrangement=pending unrealized widgets composite=NOT_VERIFIED".into());
        return;
    }
    if !widget.has_window() {
        report("x11 Unsupported: WebView does not own a GdkWindow (shared ancestor cannot be promoted)".into());
        return;
    }
    let (Some(web), Some(cad)) = (widget.window(), cad.window()) else {
        report("x11 arrangement=pending missing GdkWindow composite=NOT_VERIFIED".into());
        return;
    };
    // Promote only the existing valid WebKit-owned GdkWindow. Never fabricate a
    // window or move WebKit back into wry's now-orphaned container.
    if web == cad || unsafe { gdk::ffi::gdk_window_ensure_native(web.to_glib_none().0) } == 0 {
        report("x11 Unsupported: distinct native WebView sibling cannot be established".into());
        return;
    }
    let (Ok(web_x), Ok(cad_x), Ok(display)) = (
        web.clone().downcast::<gdkx11::X11Window>(),
        cad.clone().downcast::<gdkx11::X11Window>(),
        web.display().downcast::<gdkx11::X11Display>(),
    ) else {
        report("x11 Unsupported: native identities are not X11".into());
        return;
    };
    // SAFETY: borrowed live GDK X11 display, used only on the GTK owner thread.
    let raw = unsafe {
        gdkx11::ffi::gdk_x11_display_get_xdisplay(display.to_glib_none().0).cast::<c_void>()
    };
    let web_id = web_x.xid();
    let cad_id = cad_x.xid();
    let (Some((web_parent, _)), Some((cad_parent, _))) =
        (query_tree(raw, web_id), query_tree(raw, cad_id))
    else {
        report("x11 Unsupported: XQueryTree could not inspect native parents".into());
        return;
    };
    let visuals = format!(
        "web_xid={web_id:#x} web_parent={web_parent:#x} web_native={} web_depth={} web_visual={:?} cad_xid={cad_id:#x} cad_parent={cad_parent:#x} cad_native={} cad_depth={} cad_visual={:?}",
        web.has_native(), web.visual().depth(), web.visual().visual_type(),
        cad.has_native(), cad.visual().depth(), cad.visual().visual_type(),
    );
    if web_parent != cad_parent || web_id == cad_id {
        report(format!(
            "x11 Unsupported: native windows are not distinct siblings {visuals}"
        ));
        return;
    }
    // This changes native stacking, not just GtkOverlay/CSS child order.
    web.restack(Some(&cad), true);
    let established =
        query_tree(raw, web_parent).is_some_and(|(_, children)| above(&children, web_id, cad_id));
    report(format!(
        "x11 arrangement={} {visuals} alpha_pixels=NOT_VERIFIED composite=NOT_VERIFIED",
        if established {
            "established-native-siblings-web-above-cad"
        } else {
            "Unsupported-native-stacking-not-established"
        },
    ));
}

fn above(children: &[c_ulong], web: c_ulong, cad: c_ulong) -> bool {
    matches!(
        (children.iter().position(|id| *id == web), children.iter().position(|id| *id == cad)),
        (Some(web), Some(cad)) if web > cad
    )
}

#[link(name = "X11")]
extern "C" {
    fn XQueryTree(
        display: *mut c_void,
        window: c_ulong,
        root: *mut c_ulong,
        parent: *mut c_ulong,
        children: *mut *mut c_ulong,
        count: *mut c_uint,
    ) -> c_int;
    fn XFree(data: *mut c_void) -> c_int;
}

fn query_tree(display: *mut c_void, window: c_ulong) -> Option<(c_ulong, Vec<c_ulong>)> {
    if display.is_null() || window == 0 {
        return None;
    }
    let (mut root, mut parent, mut count) = (0, 0, 0);
    let mut children = std::ptr::null_mut();
    // SAFETY: live GDK-owned display/window; Xlib initializes the output slots.
    let ok = unsafe {
        XQueryTree(
            display,
            window,
            &mut root,
            &mut parent,
            &mut children,
            &mut count,
        )
    };
    let result = if ok == 0 {
        None
    } else if children.is_null() {
        Some((parent, Vec::new()))
    } else {
        // SAFETY: XQueryTree returns count entries, copied before XFree.
        Some((
            parent,
            unsafe { std::slice::from_raw_parts(children, count as usize) }.to_vec(),
        ))
    };
    if !children.is_null() {
        // SAFETY: this allocation came from XQueryTree and is freed exactly once.
        unsafe { XFree(children.cast()) };
    }
    result
}

#[cfg(test)]
mod tests {
    use super::above;

    #[test]
    fn native_stacking_requires_both_distinct_children_in_server_order() {
        assert!(above(&[10, 20], 20, 10));
        assert!(!above(&[10, 20], 10, 20));
        assert!(!above(&[10], 10, 10));
        assert!(!above(&[10], 20, 10));
    }

    #[test]
    fn wayland_probe_preserves_child_ownership_and_default_stacking() {
        let shim = include_str!("../../native/wayland_shim.c");
        assert!(
            shim.contains("if (below_parent)\n\t\twl_subsurface_place_below(subsurface, parent);")
        );
        assert!(shim.contains("wl_subsurface_set_desync(subsurface)"));
        assert!(shim.contains("yacr_wl_shim_set_empty_input(shim)"));
        assert!(shim.contains("if (child == parent)"));
        assert!(!shim.contains("wl_surface_commit(parent)"));
        assert!(!shim.contains("wl_surface_attach(parent"));
    }
}
