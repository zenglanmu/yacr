//! Native Wayland subsurface bridge (app-owned child `wl_surface`).
//!
//! **Why this exists:** the GPU-rendered CAD partition lives *below* the React
//! chrome, but the GDK toplevel `wl_surface` is owned and committed by GDK, so a
//! Vulkan swapchain must never target it. Instead this module owns a
//! **child `wl_surface` + `wl_subsurface`** whose parent is the GDK toplevel
//! surface, and hands the child surface to the renderer as a generic
//! raw-window-handle pair.
//!
//! **Stacking is unconditional:** the child is always placed below the parent
//! (`wl_subsurface.place_below`), so the DOM/WebView chrome is the top layer on
//! every native Wayland generation. The request is re-applied whenever the child
//! is created (including after a parent replacement), and takes effect on the
//! next GTK-owned parent commit — which is why creation also requests a GDK
//! invalidation. There is no probe/env switch and no dual arrangement.
//!
//! Everything protocol-level lives in the private C shim
//! (`native/wayland_shim.c`); this module:
//! - borrows the GDK Wayland `wl_display` and toplevel `wl_surface` (via
//!   `gdkwayland-sys`) and holds strong GDK refs;
//! - turns the shim's raw facts into an explicit readiness/health decision (the
//!   decision itself is [pure](classify_readiness) and unit-tested without a live
//!   protocol connection);
//! - creates/updates/destroys the child surface with an explicit lifecycle.
//!
//! Safety/lifetime invariants:
//! - [`WaylandSurface`] is `!Send`/`!Sync` (`PhantomData<Rc<()>>` plus raw
//!   pointers): it is created, used and dropped only on the GTK main thread.
//! - Strong GDK refs keep the borrowed `GdkWindow`/`GdkDisplay` **objects** alive,
//!   but they do **not** stop an explicit GTK `unrealize`/destroy from replacing
//!   or tearing down the native `wl_surface`. That case is detected by identity
//!   ([`WaylandSurface::health`]) and handled by an ordered release, never by
//!   destroying a live child underneath a presenter.
//! - The child surface pointer is distinct from the parent's; the C shim fails
//!   explicitly if they would alias.
//! - teardown order is fixed: the GPU presenter is released **first**, then
//!   [`WaylandSurface::release_child`] destroys the child surface/subsurface, and
//!   only then is the GDK parent allowed to unrealize.
//!
//! Protocol behavior (stacking order, parent-commit timing) is only proven by a
//! real compositor run; see `docs/validation-*.md` for the bounded evidence that
//! was actually executed and its limits.

use std::ffi::{c_char, CStr};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use gdk::glib::translate::{FromGlib, ToGlibPtr};
use gdk::prelude::*;

/// Minimum `wl_compositor` version (`wl_surface.set_buffer_scale` needs v3).
const MIN_COMPOSITOR_VERSION: u32 = 3;

/// Opaque `struct wl_display` (borrowed from GDK; never created/destroyed here).
#[repr(C)]
pub struct wl_display {
    _private: [u8; 0],
}

/// Opaque `struct wl_surface`.
#[repr(C)]
pub struct wl_surface {
    _private: [u8; 0],
}

/// Opaque C shim state (`yacr_wl_shim` in `native/wayland_shim.h`).
#[repr(C)]
pub struct yacr_wl_shim {
    _private: [u8; 0],
}

extern "C" {
    fn yacr_wl_shim_new(display: *mut wl_display) -> *mut yacr_wl_shim;
    fn yacr_wl_shim_free(shim: *mut yacr_wl_shim);
    fn yacr_wl_shim_hard_failed(shim: *const yacr_wl_shim) -> i32;
    fn yacr_wl_shim_failure(shim: *const yacr_wl_shim) -> *const c_char;
    fn yacr_wl_shim_discovery_done(shim: *const yacr_wl_shim) -> i32;
    fn yacr_wl_shim_display_error(shim: *const yacr_wl_shim) -> i32;
    fn yacr_wl_shim_has_compositor(shim: *const yacr_wl_shim) -> i32;
    fn yacr_wl_shim_has_subcompositor(shim: *const yacr_wl_shim) -> i32;
    fn yacr_wl_shim_compositor_version(shim: *const yacr_wl_shim) -> u32;
    fn yacr_wl_shim_subcompositor_version(shim: *const yacr_wl_shim) -> u32;
    fn yacr_wl_shim_create_child(
        shim: *mut yacr_wl_shim,
        parent: *mut wl_surface,
        x: i32,
        y: i32,
        buffer_scale: i32,
    ) -> i32;
    fn yacr_wl_shim_child_surface(shim: *const yacr_wl_shim) -> *mut wl_surface;
    fn yacr_wl_shim_destroy_child(shim: *mut yacr_wl_shim);
    fn yacr_wl_shim_set_position(shim: *mut yacr_wl_shim, x: i32, y: i32) -> i32;
    fn yacr_wl_shim_set_buffer_scale(shim: *mut yacr_wl_shim, scale: i32) -> i32;
    fn yacr_wl_shim_set_empty_input(shim: *mut yacr_wl_shim) -> i32;
}

/// The raw facts the C shim reports. The readiness/health decision is made here,
/// not in C, so it can be unit-tested without a live protocol connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimFacts {
    /// A C/protocol-level failure was recorded (bind/listener/global removed).
    pub hard_failed: bool,
    /// The shim's failure reason (empty while not failed).
    pub failure: String,
    /// The initial global burst has been delivered (the discovery sync fired).
    pub discovery_done: bool,
    /// The borrowed `wl_display`'s error state (`wl_display_get_error`); 0 when
    /// fine, nonzero after a fatal protocol/display error.
    pub display_error: i32,
    /// `wl_compositor` was bound.
    pub has_compositor: bool,
    /// `wl_subcompositor` was bound.
    pub has_subcompositor: bool,
    /// Bound `wl_compositor` version (0 while unbound).
    pub compositor_version: u32,
    /// Bound `wl_subcompositor` version (0 while unbound).
    pub subcompositor_version: u32,
}

/// Readiness of the borrowed Wayland display for the app-owned child surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubsurfaceReadiness {
    /// Discovery has not completed yet (GTK has not dispatched the registry/sync
    /// events): the owner must defer.
    Pending,
    /// Both globals are bound with the negotiated versions.
    Ready {
        /// Bound `wl_compositor` version (>= 3 for `set_buffer_scale`).
        compositor_version: u32,
        /// Bound `wl_subcompositor` version (1).
        subcompositor_version: u32,
    },
    /// A terminal global/protocol failure; never retried on this shim.
    Failed(String),
}

/// Result of the pre-presentation native-health check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeHealth {
    /// Safe to present another frame.
    Healthy,
    /// The shim/protocol failed: release (ordered) and do not retry.
    ShimFailed(String),
    /// The GDK parent `wl_surface` was replaced: release (ordered) and re-create
    /// on the next tick.
    ParentReplaced,
}

/// Outcome of [`WaylandSurface::ensure_child`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureChild {
    /// A child already exists for the current GDK parent.
    Current,
    /// A new child surface was created.
    Created,
    /// A child exists for a **different** (replaced) parent; nothing was destroyed.
    ParentReplaced,
}

/// Why a Wayland parent surface could not be read from the GDK window.
#[derive(Debug)]
pub enum WaylandHandleError {
    /// The GDK display is not the Wayland backend.
    NotWayland,
    /// The GDK toplevel `GdkWindow` has no `wl_surface` yet (transient).
    NoSurface,
}

impl std::fmt::Display for WaylandHandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaylandHandleError::NotWayland => {
                write!(f, "GDK display is not the Wayland backend")
            }
            WaylandHandleError::NoSurface => {
                write!(f, "GDK toplevel has no wl_surface yet")
            }
        }
    }
}

impl std::error::Error for WaylandHandleError {}

/// Whether `display` is the GDK Wayland backend (`GDK_IS_WAYLAND_DISPLAY`).
pub fn is_wayland_display(display: &gdk::Display) -> bool {
    // SAFETY: `gdk_wayland_display_get_type` is a pure type getter; it only
    // registers/returns the GType and touches no instance state.
    let wayland_type = unsafe {
        gdk::glib::types::Type::from_glib(gdk_wayland_sys::gdk_wayland_display_get_type())
    };
    display.type_().is_a(wayland_type)
}

/// The borrowed `wl_display` behind a GDK Wayland display.
///
/// # Safety
/// `display` must be a live GDK Wayland display; the returned pointer is owned by
/// GDK and is only valid while that display is alive.
unsafe fn borrowed_wl_display(display: &gdk::Display) -> Option<NonNull<wl_display>> {
    // A fully-qualified target type is required: `to_glib_none` is generic over
    // the pointer type and the raw `as` cast below would leave it ambiguous.
    let display_ptr: *mut gdk::ffi::GdkDisplay = display.to_glib_none().0;
    let raw = gdk_wayland_sys::gdk_wayland_display_get_wl_display(
        display_ptr as *mut gdk_wayland_sys::GdkWaylandDisplay,
    );
    NonNull::new(raw as *mut wl_display)
}

/// The borrowed `wl_surface` behind a GDK Wayland window.
///
/// # Safety
/// `window` must be a live GDK Wayland window; the returned pointer is owned by
/// GDK and is only valid while that window (and its display) are alive.
unsafe fn borrowed_wl_surface(window: &gdk::Window) -> Option<NonNull<wl_surface>> {
    let window_ptr: *mut gdk::ffi::GdkWindow = window.to_glib_none().0;
    let raw = gdk_wayland_sys::gdk_wayland_window_get_wl_surface(
        window_ptr as *mut gdk_wayland_sys::GdkWaylandWindow,
    );
    NonNull::new(raw as *mut wl_surface)
}

/// The borrowed toplevel `wl_surface` for a GDK window, if it is Wayland.
///
/// Diagnostic/telemetry and identity helper (the bridge compares this against the
/// retained child parent). Returns `None` on a non-Wayland display or before the
/// surface exists; never panics and never creates anything.
pub fn toplevel_surface_ptr(window: &gdk::Window) -> Option<NonNull<wl_surface>> {
    if !is_wayland_display(&window.display()) {
        return None;
    }
    // SAFETY: `window` is a live GDK Wayland window (checked just above).
    unsafe { borrowed_wl_surface(window) }
}

/// Decide readiness from the shim's raw facts (pure; unit-tested).
fn classify_readiness(facts: &ShimFacts) -> SubsurfaceReadiness {
    if facts.hard_failed {
        return SubsurfaceReadiness::Failed(facts.failure.clone());
    }
    if facts.display_error != 0 {
        return SubsurfaceReadiness::Failed(format!(
            "wayland display error (errno {})",
            facts.display_error
        ));
    }
    if !facts.discovery_done {
        return SubsurfaceReadiness::Pending;
    }
    if !facts.has_compositor {
        return SubsurfaceReadiness::Failed(
            "wl_compositor was not advertised before discovery completed".into(),
        );
    }
    if !facts.has_subcompositor {
        return SubsurfaceReadiness::Failed(
            "wl_subcompositor was not advertised before discovery completed".into(),
        );
    }
    if facts.compositor_version < MIN_COMPOSITOR_VERSION {
        return SubsurfaceReadiness::Failed(format!(
            "wl_compositor version {} < {} (no wl_surface.set_buffer_scale)",
            facts.compositor_version, MIN_COMPOSITOR_VERSION
        ));
    }
    SubsurfaceReadiness::Ready {
        compositor_version: facts.compositor_version,
        subcompositor_version: facts.subcompositor_version,
    }
}

/// Decide whether another frame may be presented on the native surface (pure).
///
/// Compares the retained child parent address against the **current** GDK parent
/// surface address; a mismatch (including a vanished parent) means the GDK surface
/// was replaced, so a fresh child must be created after an ordered release.
fn classify_health(
    readiness: &SubsurfaceReadiness,
    retained_parent: Option<usize>,
    current_parent: Option<usize>,
) -> NativeHealth {
    match readiness {
        SubsurfaceReadiness::Ready { .. } => match retained_parent {
            Some(retained) if Some(retained) != current_parent => NativeHealth::ParentReplaced,
            _ => NativeHealth::Healthy,
        },
        SubsurfaceReadiness::Failed(reason) => NativeHealth::ShimFailed(reason.clone()),
        SubsurfaceReadiness::Pending => {
            NativeHealth::ShimFailed("wayland globals not ready".into())
        }
    }
}

/// The child `wl_surface`/`wl_subsurface` tree and the GDK parent it borrows.
struct ChildSurface {
    /// Borrowed GDK toplevel `wl_surface` at creation time (identity captured so a
    /// later replacement can be detected).
    parent: NonNull<wl_surface>,
    surface: NonNull<wl_surface>,
    position: (i32, i32),
    buffer_scale: i32,
    /// Keeps the borrowed parent's GDK window **object** alive. It does not stop
    /// an explicit GTK unrealize from replacing the native `wl_surface`; that is
    /// detected via [`WaylandSurface::health`].
    _window: gdk::Window,
}

/// Main-thread-only owner of the app's own child `wl_surface`/`wl_subsurface`
/// over a borrowed GDK Wayland toplevel surface.
pub struct WaylandSurface {
    _not_send_sync: PhantomData<Rc<()>>,
    shim: NonNull<yacr_wl_shim>,
    display: NonNull<wl_display>,
    /// Keeps the borrowed `wl_display`'s GDK display **object** alive. It does not
    /// prevent an explicit teardown of the native display/surface.
    _display: gdk::Display,
    child: Option<ChildSurface>,
}

impl WaylandSurface {
    /// Attach to a GDK Wayland display: borrow its `wl_display` and create the
    /// registry + discovery shim. The globals are learned asynchronously (see
    /// [`Self::readiness`]); this never dispatches, flushes or round-trips.
    pub fn attach(display: &gdk::Display) -> Result<Self, String> {
        if !is_wayland_display(display) {
            return Err(WaylandHandleError::NotWayland.to_string());
        }
        // SAFETY: `display` is a live GDK Wayland display (checked just above).
        let Some(display_ptr) = (unsafe { borrowed_wl_display(display) }) else {
            return Err("GDK Wayland display has a null wl_display".into());
        };
        // SAFETY: `display_ptr` is a live borrowed wl_display; the shim only
        // queues a registry request + discovery sync and installs listeners.
        let Some(shim) = NonNull::new(unsafe { yacr_wl_shim_new(display_ptr.as_ptr()) }) else {
            return Err("failed to allocate the Wayland shim".into());
        };
        Ok(Self {
            _not_send_sync: PhantomData,
            shim,
            display: display_ptr,
            _display: display.clone(),
            child: None,
        })
    }

    /// The borrowed `wl_display` (for the renderer's raw display handle).
    pub fn display_ptr(&self) -> *mut wl_display {
        self.display.as_ptr()
    }

    /// Read the shim's raw facts (no decision).
    pub fn facts(&self) -> ShimFacts {
        let shim = self.shim.as_ptr();
        // SAFETY: `shim` is live for the duration of these pure accessor reads.
        unsafe {
            let failure = {
                let raw = yacr_wl_shim_failure(shim);
                if raw.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(raw).to_string_lossy().into_owned()
                }
            };
            ShimFacts {
                hard_failed: yacr_wl_shim_hard_failed(shim) != 0,
                failure,
                discovery_done: yacr_wl_shim_discovery_done(shim) != 0,
                display_error: yacr_wl_shim_display_error(shim),
                has_compositor: yacr_wl_shim_has_compositor(shim) != 0,
                has_subcompositor: yacr_wl_shim_has_subcompositor(shim) != 0,
                compositor_version: yacr_wl_shim_compositor_version(shim),
                subcompositor_version: yacr_wl_shim_subcompositor_version(shim),
            }
        }
    }

    /// Current global readiness (explicit pending / ready / failed).
    pub fn readiness(&self) -> SubsurfaceReadiness {
        classify_readiness(&self.facts())
    }

    /// Pre-presentation health: is the native surface still safe to draw into?
    ///
    /// `current_parent` is the GDK toplevel `wl_surface` read *now* (via
    /// [`toplevel_surface_ptr`]); it is compared against the retained child parent.
    pub fn health(&self, current_parent: Option<NonNull<wl_surface>>) -> NativeHealth {
        let retained = self
            .child
            .as_ref()
            .map(|child| child.parent.as_ptr() as usize);
        classify_health(
            &self.readiness(),
            retained,
            current_parent.map(|parent| parent.as_ptr() as usize),
        )
    }

    /// Ensure an app-owned child surface exists under `window`'s `wl_surface`.
    ///
    /// **Non-destructive:** if a child already exists for a *different* (replaced)
    /// parent, this destroys nothing and returns [`EnsureChild::ParentReplaced`].
    /// The bridge must detect that via [`Self::health`] and perform the ordered
    /// release (presenter first) before calling again. Requires
    /// [`SubsurfaceReadiness::Ready`].
    pub fn ensure_child(
        &mut self,
        window: &gdk::Window,
        x: i32,
        y: i32,
        buffer_scale: i32,
    ) -> Result<EnsureChild, String> {
        if !matches!(self.readiness(), SubsurfaceReadiness::Ready { .. }) {
            return Err("wayland globals are not ready".into());
        }
        // SAFETY: the caller holds a live GDK Wayland toplevel window; the
        // returned surface is borrowed and kept alive by the `ChildSurface` ref.
        let parent = unsafe { borrowed_wl_surface(window) }
            .ok_or_else(|| WaylandHandleError::NoSurface.to_string())?;
        if let Some(child) = self.child.as_ref() {
            if child.parent == parent {
                return Ok(EnsureChild::Current);
            }
            // A presenter may still be drawing into the retained child: never
            // destroy it here.
            return Ok(EnsureChild::ParentReplaced);
        }
        let scale = buffer_scale.max(1);
        // SAFETY: `self.shim` is live; `parent` is a live borrowed wl_surface.
        // The shim always places the child below the parent (DOM-on-top is the
        // only production arrangement; there is no mode switch).
        let rc =
            unsafe { yacr_wl_shim_create_child(self.shim.as_ptr(), parent.as_ptr(), x, y, scale) };
        if rc != 0 {
            return Err(shim_failure(self.shim));
        }
        // SAFETY: the shim just created the child surface.
        let Some(surface) = NonNull::new(unsafe { yacr_wl_shim_child_surface(self.shim.as_ptr()) })
        else {
            return Err("child wl_surface is null after creation".into());
        };
        debug_assert_ne!(
            surface.as_ptr(),
            parent.as_ptr(),
            "child and parent wl_surface must be distinct"
        );
        self.child = Some(ChildSurface {
            parent,
            surface,
            position: (x, y),
            buffer_scale: scale,
            _window: window.clone(),
        });
        // Queue invalidation through GDK so the GTK-owned parent commits; the
        // parent commit is what applies `place_below` + parent-relative position.
        // This re-runs for every generation (a replaced parent recreates the
        // child), so the stacking request is never lost on rebuild.
        window.invalidate_rect(None, true);
        eprintln!(
            "[dom-on-top] wayland arrangement=apply child={:p} parent={:p} place_below=parent desync=true empty_input=true parent_commit=GTK-invalidation composite=NOT_VERIFIED",
            surface.as_ptr(), parent.as_ptr(),
        );
        Ok(EnsureChild::Created)
    }

    /// The app-owned child `wl_surface`, if it exists.
    pub fn child_surface(&self) -> Option<NonNull<wl_surface>> {
        self.child.as_ref().map(|child| child.surface)
    }

    /// Update the logical position / integer buffer scale of the existing child.
    ///
    /// Returns `Ok(true)` when something actually changed. `set_buffer_scale` is
    /// child state applied on the child's next commit (the Vulkan present);
    /// `set_position` is **parent-relative double-buffered state applied on the
    /// parent's next commit**, so the caller must request a GTK toplevel
    /// `queue_draw` (the app never commits the GDK surface itself).
    pub fn update_layout(&mut self, x: i32, y: i32, buffer_scale: i32) -> Result<bool, String> {
        let shim = self.shim;
        let Some(child) = self.child.as_mut() else {
            return Ok(false);
        };
        let mut changed = false;
        if child.position != (x, y) {
            // SAFETY: `shim` is live and owns the child.
            if unsafe { yacr_wl_shim_set_position(shim.as_ptr(), x, y) } != 0 {
                return Err(shim_failure(shim));
            }
            child.position = (x, y);
            changed = true;
        }
        let scale = buffer_scale.max(1);
        if child.buffer_scale != scale {
            // SAFETY: `shim` is live and owns the child.
            if unsafe { yacr_wl_shim_set_buffer_scale(shim.as_ptr(), scale) } != 0 {
                return Err(shim_failure(shim));
            }
            child.buffer_scale = scale;
            changed = true;
        }
        Ok(changed)
    }

    /// Re-assert the empty input region on the existing child (click-through to
    /// the DOM surfaces below). A missing child is a no-op.
    pub fn reassert_empty_input(&mut self) -> Result<(), String> {
        let shim = self.shim;
        if self.child.is_none() {
            return Ok(());
        }
        // SAFETY: `shim` is live and owns the child.
        if unsafe { yacr_wl_shim_set_empty_input(shim.as_ptr()) } != 0 {
            return Err(shim_failure(shim));
        }
        Ok(())
    }

    /// Destroy the child `wl_surface`/`wl_subsurface` (idempotent).
    ///
    /// Must be called **after** the GPU presenter is released and **before** the
    /// GDK parent is unrealized.
    pub fn release_child(&mut self) {
        self.destroy_child();
    }

    fn destroy_child(&mut self) {
        if self.child.is_none() {
            return;
        }
        // SAFETY: `self.shim` is live; the shim owns/destroys the child tree.
        unsafe { yacr_wl_shim_destroy_child(self.shim.as_ptr()) };
        self.child = None;
    }
}

impl Drop for WaylandSurface {
    fn drop(&mut self) {
        // The shim destroys a pending discovery callback and any child before
        // freeing itself, so nothing can reach the freed userdata.
        // SAFETY: `self.shim` is a live shim owned by this struct, freed once.
        unsafe { yacr_wl_shim_free(self.shim.as_ptr()) };
    }
}

/// Read the shim's failure string (never fails; empty until a failure).
fn shim_failure(shim: NonNull<yacr_wl_shim>) -> String {
    // SAFETY: `shim` is live; the returned pointer is a NUL-terminated string
    // owned by the shim and valid for the duration of the call.
    let raw = unsafe { yacr_wl_shim_failure(shim.as_ptr()) };
    if raw.is_null() {
        return "wayland shim failure (no detail)".into();
    }
    // SAFETY: non-null, NUL-terminated per the shim contract.
    unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        classify_health, classify_readiness, is_wayland_display, NativeHealth, ShimFacts,
        SubsurfaceReadiness, WaylandHandleError,
    };

    fn base_facts() -> ShimFacts {
        ShimFacts {
            hard_failed: false,
            failure: String::new(),
            discovery_done: true,
            display_error: 0,
            has_compositor: true,
            has_subcompositor: true,
            compositor_version: 4,
            subcompositor_version: 1,
        }
    }

    #[test]
    fn not_wayland_error_is_explicit() {
        // The Wayland attach path must fail explicitly on a non-Wayland display,
        // never silently succeed.
        assert_eq!(
            WaylandHandleError::NotWayland.to_string(),
            "GDK display is not the Wayland backend"
        );
        assert!(WaylandHandleError::NoSurface
            .to_string()
            .contains("wl_surface"));
    }

    #[test]
    fn readiness_is_pending_until_discovery_completes() {
        let facts = ShimFacts {
            discovery_done: false,
            ..base_facts()
        };
        assert_eq!(classify_readiness(&facts), SubsurfaceReadiness::Pending);
    }

    #[test]
    fn hard_failure_reason_is_surfaced() {
        let facts = ShimFacts {
            hard_failed: true,
            failure: "wl_registry_bind(wl_compositor) failed".into(),
            discovery_done: false,
            ..base_facts()
        };
        assert_eq!(
            classify_readiness(&facts),
            SubsurfaceReadiness::Failed("wl_registry_bind(wl_compositor) failed".into())
        );
    }

    #[test]
    fn display_error_is_explicit_failure() {
        let facts = ShimFacts {
            display_error: 71, // EPROTO
            ..base_facts()
        };
        match classify_readiness(&facts) {
            SubsurfaceReadiness::Failed(message) => assert!(message.contains("errno 71")),
            other => panic!("expected explicit failure, got {other:?}"),
        }
        // A display error also fails the pre-presentation health check.
        assert!(matches!(
            classify_health(&classify_readiness(&facts), Some(1), Some(1)),
            NativeHealth::ShimFailed(_)
        ));
    }

    #[test]
    fn missing_globals_after_discovery_are_explicit_failure() {
        // The pre-remediation bug: a compositor without a subcompositor stayed
        // Pending forever. It must be an explicit failure once discovery is done.
        let no_compositor = ShimFacts {
            has_compositor: false,
            ..base_facts()
        };
        match classify_readiness(&no_compositor) {
            SubsurfaceReadiness::Failed(message) => assert!(message.contains("wl_compositor")),
            other => panic!("expected explicit failure, got {other:?}"),
        }
        let no_subcompositor = ShimFacts {
            has_subcompositor: false,
            ..base_facts()
        };
        match classify_readiness(&no_subcompositor) {
            SubsurfaceReadiness::Failed(message) => assert!(message.contains("wl_subcompositor")),
            other => panic!("expected explicit failure, got {other:?}"),
        }
    }

    #[test]
    fn old_compositor_is_explicit_failure() {
        let facts = ShimFacts {
            compositor_version: 2,
            ..base_facts()
        };
        match classify_readiness(&facts) {
            SubsurfaceReadiness::Failed(message) => assert!(message.contains("version 2")),
            other => panic!("expected explicit failure, got {other:?}"),
        }
    }

    #[test]
    fn ready_requires_both_globals_and_v3() {
        assert_eq!(
            classify_readiness(&base_facts()),
            SubsurfaceReadiness::Ready {
                compositor_version: 4,
                subcompositor_version: 1,
            }
        );
    }

    #[test]
    fn health_flags_parent_replacement_when_ready() {
        let ready = SubsurfaceReadiness::Ready {
            compositor_version: 4,
            subcompositor_version: 1,
        };
        assert_eq!(
            classify_health(&ready, Some(1), Some(2)),
            NativeHealth::ParentReplaced
        );
        // A vanished current parent (unrealized) is also a replacement.
        assert_eq!(
            classify_health(&ready, Some(1), None),
            NativeHealth::ParentReplaced
        );
        // Same parent (or no child yet) is healthy.
        assert_eq!(
            classify_health(&ready, Some(1), Some(1)),
            NativeHealth::Healthy
        );
        assert_eq!(classify_health(&ready, None, None), NativeHealth::Healthy);
    }

    #[test]
    fn health_shim_failure_is_terminal() {
        assert_eq!(
            classify_health(
                &SubsurfaceReadiness::Failed("boom".into()),
                Some(1),
                Some(1)
            ),
            NativeHealth::ShimFailed("boom".into())
        );
        assert_eq!(
            classify_health(&SubsurfaceReadiness::Pending, Some(1), Some(1)),
            NativeHealth::ShimFailed("wayland globals not ready".into())
        );
    }

    #[test]
    fn wayland_marker_function_is_available() {
        // A pure function item: no display is created, so this cannot touch GTK.
        let _marker: fn(&gdk::Display) -> bool = is_wayland_display;
    }
}
