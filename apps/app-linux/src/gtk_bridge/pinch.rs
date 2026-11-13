//! Trackpad pinch: turn WebKitGTK's **UI-process** two-finger pinch into a CAD
//! zoom instead of a page zoom.
//!
//! # Why this is not a DOM problem
//!
//! On WebKitGTK a trackpad pinch is handled in the UI process: `WebKitWebViewBase`
//! owns a private `GtkGestureZoom` (`ViewGestureControllerGtk`) that feeds
//! `pageScaleFactor`. That produces **no DOM event**, and `preventDefault()` in
//! the page cannot stop it — the frontend's `ctrl`+wheel / `gesture*` guards are a
//! safety net for the engines that do deliver a DOM event (Chromium/WebView2,
//! Safari/WKWebView), not the Linux fix. No `WebKitSettings` disables it either.
//!
//! # The two routes (chosen once, at webview creation)
//!
//! 1. **`WebKitWebView:magnification`** — the public property from WebKitGTK
//!    **>= 2.54**. Pin it at `1.0` and forward the observed incremental ratio as
//!    a centred CAD pinch ([`PinchPlan::PinMagnification`]).
//! 2. **The private gesture object** — on 2.52 (this host) WebKit stashes its
//!    zoom gesture as GObject data under the private key
//!    `"wk-view-zoom-gesture"`. Two steps, both public API:
//!    * set the gesture's propagation phase to `NONE` so WebKit's controller
//!      never sees a pinch (its own `dispose` still owns the object);
//!    * intercept `GdkEventTouchpadPinch` on the widget's **`event` signal**,
//!      queue the incremental `scale` + position, and return `Stop` so WebKit's
//!      vfunc never runs and `pageScaleFactor` cannot change
//!      ([`PinchPlan::NeutraliseAndOwn`]).
//!
//!    Why the signal and not a second `GtkGestureZoom`: measured on this host,
//!    a gesture attached to the same `WebKitWebView` never fired (`begin=0`) —
//!    GTK's gesture group still consumes the touchpad sequence. The `event`
//!    signal runs **before** the widget vfunc and returning `Stop` both takes the
//!    sample and blocks the page zoom, with no handler destruction and no
//!    controller-ordering assumptions.
//! 3. Neither object exists: WebKit still owns the pinch
//!    ([`PinchPlan::Unavailable`]). That is reported once, and **nothing** is
//!    attached — the page keeps zooming, which is the honest state rather than a
//!    claim that it does not.
//!
//! The sample itself is a [`NativePinch`] in **widget-local GTK logical px**
//! queued on the owner state; only the tick knows the CAD partition rect and the
//! CSS↔GTK ratio, so the conversion (and the one-publication input batch) stays
//! there (`host::pinch_intake`).
//!
//! # Evidence boundary
//!
//! The `event`-signal route was measured end to end on a real Wayland session
//! with a **synthetic** `GdkEventTouchpadPinch` (`YACR_PROBE_PINCH`): the handler
//! saw all four phases, the page scale stayed `1.0`, and the CAD camera scale
//! changed by the accumulated pinch factor (2.0x for 1.25 x 1.6). That bypasses
//! the kernel/libinput/compositor gesture source, so a **real trackpad** run is
//! still the product verdict. The chosen route logs its own token
//! (`[probe] pinch: …`) once, at webview creation, so a human run can say which
//! one was actually installed.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gtk::prelude::*;
use gtk::PropagationPhase;

use cad_desktop_shell::state::{lock, NativePinch, Shared};

/// The GObject data key under which WebKitGTK keeps its private zoom gesture
/// (wry#544). Private API: present in 2.52, not part of the public contract.
pub(crate) const WEBKIT_ZOOM_GESTURE_KEY: &str = "wk-view-zoom-gesture";

/// The public magnification property from WebKitGTK 2.54 on.
pub(crate) const MAGNIFICATION_PROPERTY: &str = "magnification";

/// The stable evidence token for each decision (never logged per sample).
pub(crate) const PINCH_NOTE_MAGNIFICATION: &str = "[probe] pinch: magnification-pinned";
/// WebKit's gesture is inert and this host's `event` handler owns the pinch.
pub(crate) const PINCH_NOTE_NEUTRALISED: &str = "[probe] pinch: webkit-gesture-neutralised";
/// See [`PINCH_NOTE_MAGNIFICATION`].
pub(crate) const PINCH_NOTE_UNAVAILABLE: &str = "[probe] pinch: gesture_unavailable";
/// Logged once when a carrier has no native CAD session (see [`install_on_widget`]).
pub(crate) const PINCH_NOTE_NO_INTAKE: &str =
    "[probe] pinch: no native CAD intake on this carrier (dom-canvas)";

thread_local! {
    /// Set once the sample queue overflowed, so the drop is reported once.
    static PINCH_OVERFLOW_LOGGED: Cell<bool> = const { Cell::new(false) };
}

/// How deep the widget walk looks for the object that owns the gesture/property.
///
/// The `WebKitWebView` this host holds wraps the internal `WebKitWebViewBase` one
/// level down; the bound keeps a pathological toolkit tree from being walked.
const WIDGET_SEARCH_DEPTH: usize = 8;

/// What this host can do about WebKit's own pinch handling. Pure decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinchPlan {
    /// WebKitGTK >= 2.54: pin `magnification` at `1.0` and forward the observed
    /// ratio as a centred CAD pinch.
    PinMagnification,
    /// The private zoom gesture exists: destroy its handlers and own the
    /// gesture.
    NeutraliseAndOwn,
    /// Neither route exists: WebKit keeps owning pinch, so page zoom remains.
    Unavailable,
}

/// Decide the pinch plan from what the widget actually exposes.
///
/// The public property wins when both are present: it is the supported API, and
/// pinning it keeps the page at `1.0` even if a future WebKit adds its own
/// default gesture back.
pub fn pinch_plan(has_magnification: bool, has_private_gesture: bool) -> PinchPlan {
    if has_magnification {
        PinchPlan::PinMagnification
    } else if has_private_gesture {
        PinchPlan::NeutraliseAndOwn
    } else {
        PinchPlan::Unavailable
    }
}

/// The stable evidence token for a plan.
pub(crate) fn pinch_note(plan: PinchPlan) -> &'static str {
    match plan {
        PinchPlan::PinMagnification => PINCH_NOTE_MAGNIFICATION,
        PinchPlan::NeutraliseAndOwn => PINCH_NOTE_NEUTRALISED,
        PinchPlan::Unavailable => PINCH_NOTE_UNAVAILABLE,
    }
}

/// Install the carrier's pinch handling once, at webview creation.
///
/// Idempotent by construction: it runs from `host::init` exactly once per
/// webview, and the widget it inspects is the live `WebKitWebView`.
pub fn install_pinch_gesture(webview: &tauri::Webview, shared: &Arc<Mutex<Shared>>) {
    let shared = shared.clone();
    let result = webview.with_webview(move |platform| {
        // `with_webview` runs on the GTK main thread; the GTK object is
        // `!Send`, so nothing outlives this closure except plain data.
        let widget = platform.inner().upcast::<gtk::Widget>();
        install_on_widget(&widget, &shared);
    });
    if let Err(error) = result {
        eprintln!("[probe] pinch: webview hook FAILED: {error}");
    }
}

/// The GTK-main-thread body of [`install_pinch_gesture`].
fn install_on_widget(widget: &gtk::Widget, shared: &Arc<Mutex<Shared>>) {
    let magnification = find_widget_with_property(widget, MAGNIFICATION_PROPERTY);
    let gesture_owner = if magnification.is_some() {
        None
    } else {
        find_widget_with_data(widget, WEBKIT_ZOOM_GESTURE_KEY)
    };
    let plan = pinch_plan(magnification.is_some(), gesture_owner.is_some());
    // `dom-canvas` (X11/XWayland, Web) has no native CAD session (iron rule 8), so
    // there is nothing to route a native sample into. Neutralising WebKit's page
    // zoom is still right; installing a queue nothing drains would not be.
    let intake = crate::host::surface_plan().native_cad_session;
    match plan {
        PinchPlan::PinMagnification => {
            eprintln!("{}", pinch_note(plan));
            if let Some(owner) = magnification {
                pin_magnification(&owner, shared, intake);
            }
        }
        PinchPlan::NeutraliseAndOwn => {
            eprintln!("{}", pinch_note(plan));
            if let Some(owner) = gesture_owner {
                match webkit_zoom_gesture(&owner) {
                    Some(gesture) => {
                        silence_webkit_zoom(gesture);
                        if intake {
                            own_gesture(&owner, shared);
                        } else {
                            eprintln!("{PINCH_NOTE_NO_INTAKE}");
                        }
                    }
                    None => eprintln!(
                        "{} (the `{WEBKIT_ZOOM_GESTURE_KEY}` data is not a GObject): WebKitGTK \
                         keeps owning pinch, so the page still scales",
                        pinch_note(plan)
                    ),
                }
            }
        }
        PinchPlan::Unavailable => {
            // Say it once, and do not attach a gesture that would never receive
            // the event: the page still zooms, and nothing claims otherwise.
            eprintln!(
                "{} (no `{MAGNIFICATION_PROPERTY}` property and no `{WEBKIT_ZOOM_GESTURE_KEY}` data): \
                 WebKitGTK keeps owning pinch, so the page still scales",
                pinch_note(plan)
            );
        }
    }
}

/// Make WebKit's private zoom gesture inert **without** touching its lifetime.
///
/// The gesture is a `GtkEventController` attached to the widget. Setting its
/// propagation phase to `NONE` is the public way to say "this controller must not
/// receive events": [`gtk_widget_run_controllers`] only runs the phase it was
/// called for (capture/target/bubble), so WebKit's zoom handler never sees a
/// pinch and `pageScaleFactor` never changes. The object stays attached and owned
/// by WebKit, so its own `dispose` is untouched (nothing is freed, stolen or
/// reparented — the failure mode of the older `g_signal_handlers_destroy`
/// recipe).
///
/// Why the phase and not the handlers: destroying the handlers stops the zoom,
/// but the controller **still consumes** the event (`gtk_gesture_handle_event`
/// claims the sequence inside the controller, before any signal), so a second
/// gesture attached later never sees it. Measured on this host: handler-destroy
/// gave `begin=0`, phase-`NONE` gives `begin=1` to the host's own gesture.
///
/// (gtk_widget_run_controllers is a static in GTK3, hence the plain name.)
/// Not a real symbol: documentation only.
fn silence_webkit_zoom(gesture: *mut gtk::glib::gobject_ffi::GObject) {
    // SAFETY: `gesture` is the live `GtkGestureZoom` WebKit stored under its
    // private data key, read from the widget on this thread (the GTK main
    // thread, from `with_webview`). `from_glib_none` only adds a temporary
    // reference for the call.
    use gtk::glib::translate::FromGlibPtrNone;
    let object = unsafe { gtk::glib::Object::from_glib_none(gesture) };
    match object.downcast_ref::<gtk::EventController>() {
        Some(controller) => {
            controller.set_propagation_phase(PropagationPhase::None);
            eprintln!(
                "[probe] pinch: webkit gesture type={} phase_after={:?}",
                object.type_().name(),
                controller.propagation_phase()
            );
        }
        None => eprintln!(
            "[probe] pinch: webkit zoom data is not an event controller (type={})",
            object.type_().name()
        ),
    }
}

/// The `GtkGestureZoom` WebKit stashed under its private data key, if any.
fn webkit_zoom_gesture(widget: &gtk::Widget) -> Option<*mut gtk::glib::gobject_ffi::GObject> {
    let key = std::ffi::CString::new(WEBKIT_ZOOM_GESTURE_KEY).ok()?;
    // SAFETY: `widget` is a live GObject on this thread and `key` is a valid
    // NUL-terminated string; `g_object_get_data` only reads the association.
    let value =
        unsafe { gtk::glib::gobject_ffi::g_object_get_data(widget.as_ptr().cast(), key.as_ptr()) };
    (!value.is_null()).then(|| value.cast())
}

/// Intercept the toolkit's pinch **on the widget's `event` signal** and queue it.
///
/// Why the signal and not a `GtkGestureZoom` (measured on this host, 2.52.6):
///
/// * GTK runs **capture-phase** controllers before the widget's `event` signal,
///   and WebKit's own zoom gesture is one of them. Silencing it (phase `NONE`) is
///   necessary but not sufficient: with a second `GtkGestureZoom` attached to the
///   same widget, `begin`/`scale-changed` never fired on this host (the controller
///   group still swallowed the sequence).
/// * The `event` signal is emitted **before** `GtkWidget::event` reaches WebKit's
///   vfunc, and returning `Stop` prevents that vfunc entirely. So this both takes
///   the sample and makes the page zoom impossible, with no gesture group,
///   no handler destruction and no per-widget controller ordering to reason about.
///
/// Coordinates: `GdkEventTouchpadPinch::x/y` are window-relative GTK logical px —
/// the same space the tick's `dpr / parent_scale` conversion expects.
///
/// Scale: `GdkEventTouchpadPinch::scale` is the backend's **per-event** delta
/// (libinput's `scale`, not `scale_absolute`), which is exactly the incremental
/// ratio `InputKind::Pinch` consumes. BEGIN/UPDATE carry it; END/CANCEL do not.
///
/// A touchscreen two-finger pinch is a `GDK_TOUCH_*` pointer sequence, not a
/// touchpad event: it never enters this branch and stays with the DOM's existing
/// two-contact path.
fn own_gesture(host: &gtk::Widget, shared: &Arc<Mutex<Shared>>) {
    // The signal closure outlives this call, so it owns its handle.
    let shared = shared.clone();
    // Evidence runs (`YACR_PROBE_PINCH`) need to see whether the event arrives at
    // all; a normal run is silent.
    let debug = std::env::var_os("YACR_PROBE_PINCH").is_some();
    let anchor_host = host.clone();
    host.connect_event(move |_, event| {
        if event.event_type() != gdk::EventType::TouchpadPinch {
            return gtk::glib::Propagation::Proceed;
        }
        use gdk::glib::translate::ToGlibPtr;
        // SAFETY: `event` is a live `GdkEvent` for this dispatch, and its type is
        // the pinch variant (checked above), so reading `touchpad_pinch` is the
        // active union member.
        let (phase, fingers, x, y, scale) = unsafe {
            let raw: *const gdk::ffi::GdkEvent = event.to_glib_none().0;
            let pinch = (*raw).touchpad_pinch;
            (pinch.phase, pinch.n_fingers, pinch.x, pinch.y, pinch.scale)
        };
        if debug {
            eprintln!(
                "[probe] pinch debug: event phase={phase} fingers={fingers} scale={scale} \
                 at ({x:.1},{y:.1})"
            );
        }
        // Another finger count is not a two-finger CAD pinch; leave it to WebKit.
        if fingers != 2 {
            return gtk::glib::Propagation::Proceed;
        }
        let begin = gdk::ffi::GDK_TOUCHPAD_GESTURE_PHASE_BEGIN as i8;
        let update = gdk::ffi::GDK_TOUCHPAD_GESTURE_PHASE_UPDATE as i8;
        if (phase == begin || phase == update)
            && scale.is_finite()
            && scale > 0.0
            && (scale - 1.0).abs() > f64::EPSILON
        {
            push_sample(&shared, &anchor_host, Some((x, y)), scale);
        }
        // Swallow it: WebKit must never turn a pinch into page zoom.
        gtk::glib::Propagation::Stop
    });
    eprintln!(
        "[probe] pinch: own event handler attached to {}",
        host.type_().name()
    );
}

/// The `>= 2.54` route: pin `magnification` at `1.0` and forward the ratio.
fn pin_magnification(owner: &gtk::Widget, shared: &Arc<Mutex<Shared>>, intake: bool) {
    // The notify closure outlives this call, so it owns its handle.
    let shared = shared.clone();
    // Pin the page magnification: from here on the page cannot scale, whatever
    // the gesture does.
    owner.set_property(MAGNIFICATION_PROPERTY, 1.0f64);
    if !intake {
        eprintln!("{PINCH_NOTE_NO_INTAKE}");
        return;
    }
    let pinning = Rc::new(Cell::new(false));
    let previous = Rc::new(Cell::new(1.0f64));
    let anchor_host = owner.clone();
    owner.connect_notify_local(Some(MAGNIFICATION_PROPERTY), move |owner, _| {
        // Force-back inside the property's own notify: guard the re-entry instead
        // of recursing.
        if pinning.replace(true) {
            return;
        }
        let current: f64 = owner.property(MAGNIFICATION_PROPERTY);
        owner.set_property(MAGNIFICATION_PROPERTY, 1.0f64);
        // Read it back: the force-back is only a claim until the property says so.
        let restored = (owner.property::<f64>(MAGNIFICATION_PROPERTY) - 1.0).abs() <= f64::EPSILON;
        pinning.set(false);
        if !current.is_finite() {
            return;
        }
        // If the force-back failed the page keeps its new value, so the next
        // sample must be measured against it rather than against 1.0.
        let base = if restored {
            previous.replace(1.0)
        } else {
            previous.replace(current)
        };
        if !base.is_finite() || base <= 0.0 {
            return;
        }
        let ratio = current / base;
        if (ratio - 1.0).abs() <= f64::EPSILON {
            return;
        }
        // This route is centred by construction: the public property reports no
        // anchor, so the CAD partition centre is the documented anchor.
        push_sample(&shared, &anchor_host, None, ratio);
    });
}

/// Queue one sample, anchored at the pointer (or the partition centre).
fn push_sample(
    shared: &Arc<Mutex<Shared>>,
    host: &gtk::Widget,
    pointer: Option<(f64, f64)>,
    scale: f64,
) {
    let mut state = lock(shared);
    let (x, y) = pointer.unwrap_or_else(|| {
        // No pointer sample yet: the anchor is the CAD partition's centre in
        // widget-local GTK logical px (the ratio the tick applies is
        // `dpr / parent_scale`, and `scale_factor()` is that same parent scale).
        state
            .shell
            .input
            .last_rect()
            .map(|rect| {
                let ratio = f64::from(host.scale_factor().max(1));
                (
                    (f64::from(rect.x) + f64::from(rect.width) / 2.0) * ratio,
                    (f64::from(rect.y) + f64::from(rect.height) / 2.0) * ratio,
                )
            })
            .unwrap_or_else(|| widget_centre(host))
    });
    if state.push_native_pinch(NativePinch { x, y, scale }) {
        // The queue is drained every frame, so an overflow means the tick is not
        // running (paused window, GPU init). Report it once, not per sample.
        PINCH_OVERFLOW_LOGGED.with(|logged| {
            if !logged.replace(true) {
                eprintln!(
                    "[probe] pinch: sample queue full ({} samples); dropped the oldest",
                    cad_desktop_shell::state::NATIVE_PINCH_CAP
                );
            }
        });
    }
}

/// The widget's own centre in widget-local GTK logical px (last-resort anchor).
fn widget_centre(host: &gtk::Widget) -> (f64, f64) {
    let allocation = host.allocation();
    (
        f64::from(allocation.width()) / 2.0,
        f64::from(allocation.height()) / 2.0,
    )
}

/// Every widget in the subtree, root first (breadth-first, depth-bounded).
fn walk_widgets(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = Vec::new();
    let mut frontier = vec![(root.clone(), 0usize)];
    while let Some((widget, depth)) = frontier.pop() {
        if depth < WIDGET_SEARCH_DEPTH {
            if let Some(container) = widget.downcast_ref::<gtk::Container>() {
                for child in container.children() {
                    frontier.push((child, depth + 1));
                }
            }
        }
        widgets.push(widget);
    }
    widgets
}

/// The first widget in `root`'s subtree that defines property `name`.
fn find_widget_with_property(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    walk_widgets(root)
        .into_iter()
        .find(|widget| widget.find_property(name).is_some())
}

/// The first widget in `root`'s subtree that carries GObject data `key`.
fn find_widget_with_data(root: &gtk::Widget, key: &str) -> Option<gtk::Widget> {
    walk_widgets(root)
        .into_iter()
        .find(|widget| object_has_data(widget, key))
}

/// Whether `widget` has GObject data associated under `key`.
fn object_has_data(widget: &gtk::Widget, key: &str) -> bool {
    let Ok(key) = std::ffi::CString::new(key) else {
        return false;
    };
    // SAFETY: `widget` is a live GObject on this thread and `key` is a valid
    // NUL-terminated string; `g_object_get_data` only reads the association, it
    // never takes ownership of the value.
    let value =
        unsafe { gtk::glib::gobject_ffi::g_object_get_data(widget.as_ptr().cast(), key.as_ptr()) };
    !value.is_null()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_magnification_property_wins_over_the_private_gesture() {
        // Supported API first: it keeps the page at 1.0 even if a future WebKit
        // re-adds its own default gesture.
        assert_eq!(pinch_plan(true, true), PinchPlan::PinMagnification);
        assert_eq!(pinch_plan(true, false), PinchPlan::PinMagnification);
    }

    #[test]
    fn the_private_gesture_is_the_route_below_webkitgtk_2_54() {
        // webkit2gtk-4.1 2.52.6 (this host) has no `magnification` property, so
        // the wry#544 neutralisation is the route it intends to take.
        assert_eq!(pinch_plan(false, true), PinchPlan::NeutraliseAndOwn);
    }

    #[test]
    fn neither_route_is_named_as_unavailable_never_as_a_silent_success() {
        assert_eq!(pinch_plan(false, false), PinchPlan::Unavailable);
    }

    #[test]
    fn every_plan_has_its_own_stable_evidence_token() {
        assert_eq!(
            pinch_note(PinchPlan::PinMagnification),
            "[probe] pinch: magnification-pinned"
        );
        assert_eq!(
            pinch_note(PinchPlan::NeutraliseAndOwn),
            "[probe] pinch: webkit-gesture-neutralised"
        );
        assert!(
            pinch_note(PinchPlan::Unavailable).starts_with("[probe] pinch: gesture_unavailable")
        );
        assert_ne!(
            pinch_note(PinchPlan::PinMagnification),
            pinch_note(PinchPlan::NeutraliseAndOwn)
        );
    }
}
