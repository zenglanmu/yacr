//! DOM-on-top stacking **regression** diagnostics and parent-commit timing.
//!
//! DOM-on-top is the production arrangement, not a probe: the WebView chrome is
//! the GDK toplevel surface and the app-owned CAD `wl_subsurface` is always
//! placed below it. Nothing here decides the arrangement.
//!
//! Two jobs remain:
//! - [`establish`] reports what the compositor-facing arrangement actually is,
//!   for the opt-in regression fixture (`crate::stacking_regression::enabled()`):
//!   the Wayland child ownership/alpha facts, and — on the **retained but
//!   unreachable** X11 path — whether two distinct native siblings could be
//!   ordered WebView-above-CAD. It can report `Unsupported`; it never asserts
//!   compositing success.
//! - [`child_created`] / [`note_present`] / [`note_parent_commit`] record the
//!   desync ordering across a surface generation rebuild: the stacking request is
//!   parent-relative double-buffered state, so it only takes effect on the next
//!   GTK-owned parent commit. Logging the first child commit (present) and the
//!   first parent commit after a (re)created child makes a lost `place_below`
//!   replay or an inverted first-frame order visible as a number.
//!
//! **X11 is NOT on the production path:** it runs `surfaceMode=dom-canvas` (ADR
//! 0024) and installs no bridge at all, so the native-child/restack branch below
//! is unreachable. It is retained (and unit-tested) pending an explicit cleanup
//! decision; it is not evidence of X11 capability and never a canvas fallback.

use std::cell::RefCell;
use std::ffi::{c_int, c_uint, c_ulong, c_void};
use std::time::Instant;

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;

thread_local! {
    static LAST: RefCell<Option<String>> = const { RefCell::new(None) };
    /// The surface generation whose first parent commit has not been observed yet.
    static PENDING: RefCell<Option<PendingCommit>> = const { RefCell::new(None) };
}

/// First-commit bookkeeping for one (re)created child surface.
struct PendingCommit {
    generation: u64,
    child_created: Instant,
    first_present: Option<Instant>,
}

fn report(message: String) {
    LAST.with(|last| {
        if last.borrow().as_ref() != Some(&message) {
            eprintln!("[dom-on-top] {message}");
            *last.borrow_mut() = Some(message);
        }
    });
}

/// Record that a child surface generation was (re)created and now waits for its
/// first parent commit (which is what applies `place_below` + the position).
///
/// Called for every generation, so a parent replacement cannot silently lose the
/// stacking replay.
pub(super) fn child_created(generation: u64) {
    PENDING.with(|pending| {
        *pending.borrow_mut() = Some(PendingCommit {
            generation,
            child_created: Instant::now(),
            first_present: None,
        })
    });
    eprintln!(
        "[dom-on-top] child generation={generation} place_below=parent replayed=true \
         desync=true parent_commit=pending"
    );
}

/// Record a successful child commit (the Vulkan present) while the first parent
/// commit is still pending: this is the desync order that matters.
pub(super) fn note_present() {
    let now = Instant::now();
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        let Some(state) = pending.as_mut() else {
            return;
        };
        if state.first_present.is_some() {
            return;
        }
        state.first_present = Some(now);
        eprintln!(
            "[dom-on-top] desync timing generation={} first-present before-parent-commit \
             since_child_us={} parent_commit=NOT_YET",
            state.generation,
            now.duration_since(state.child_created).as_micros(),
        );
    });
}

/// Record the first GTK-owned parent commit after a child (re)creation.
///
/// This is the **trigger** assertion only: it proves GTK committed the parent
/// (so `place_below`/position were consumed) and records whether a child present
/// happened first. Whether the compositor honoured the order is not observable
/// here (`composite=NOT_VERIFIED`).
pub(super) fn note_parent_commit() {
    let Some(state) = PENDING.with(|pending| pending.borrow_mut().take()) else {
        return;
    };
    let now = Instant::now();
    eprintln!(
        "[dom-on-top] parent-commit generation={} since_child_us={} \
         first_present_before_parent_commit={} place_below=applied-on-parent-commit \
         desync=true composite=NOT_VERIFIED",
        state.generation,
        now.duration_since(state.child_created).as_micros(),
        state.first_present.is_some(),
    );
}

/// Report the current native/alpha arrangement for the regression fixture.
///
/// On Wayland this is the app-owned child surface under the GDK toplevel; the
/// GTK parent's alpha is cleared unconditionally by the owner's draw handler.
/// The X11 branch is unreachable in production (see the module docs).
pub(super) fn establish(widget: &gtk::Widget, parent: &gtk::Widget) {
    if crate::wayland_surface::is_wayland_display(&widget.display()) {
        if let Some(window) = widget.toplevel().and_then(|top| top.window()) {
            let visual = window.visual();
            report(format!(
                "wayland GTK-parent depth={} visual={:?} webview_has_window={} webview_native={} \
                 alpha_pixels=NOT_VERIFIED opaque_region_policy=clear-via-GTK-draw \
                 actual_protocol_region=NOT_OBSERVABLE composite=NOT_VERIFIED",
                visual.depth(),
                visual.visual_type(),
                widget.has_window(),
                widget.window().is_some_and(|window| window.has_native()),
            ));
        }
        return;
    }
    // unreachable_since_dom_on_top: X11 runs `dom-canvas` and installs no bridge,
    // so this branch cannot run in the shipped host. It is kept for an explicit
    // cleanup decision and is exercised only by unit tests (the pure `above`
    // ordering helper below).
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
        report(
            "x11 Unsupported: WebView does not own a GdkWindow (shared ancestor cannot be promoted)"
                .into(),
        );
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
    fn x11_native_stacking_option_is_unreachable_since_dom_on_top() {
        // unreachable_since_dom_on_top: the helper is pure, but the branch that
        // calls it never runs on X11 (dom-canvas installs no bridge). This test
        // pins the comparison rule, not an X11 capability.
        assert!(above(&[10, 20], 20, 10));
        assert!(!above(&[10, 20], 10, 20));
        assert!(!above(&[10], 10, 10));
        assert!(!above(&[10], 20, 10));
    }

    #[test]
    fn wayland_child_is_always_placed_below_the_dom_parent() {
        // The shim contract, asserted at the source level so a reintroduced mode
        // switch fails loudly without a compositor run.
        let shim = include_str!("../../native/wayland_shim.c");
        assert!(shim.contains("wl_subsurface_place_below(subsurface, parent);"));
        assert!(
            !shim.contains("below_parent"),
            "there is no optional stacking mode any more"
        );
        assert!(shim.contains("wl_subsurface_set_desync(subsurface)"));
        assert!(shim.contains("yacr_wl_shim_set_empty_input(shim)"));
        assert!(shim.contains("if (child == parent)"));
        assert!(!shim.contains("wl_surface_commit(parent)"));
        assert!(!shim.contains("wl_surface_attach(parent"));
    }
}
