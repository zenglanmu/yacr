//! What each `surfaceMode` may construct on the native side (ADR 0024 D1/D2).
//!
//! The host has two carriers. `native-below` (native Wayland) owns a native CAD
//! session, a GTK `DrawingArea` inside a `GtkOverlay`, a `wl_subsurface` on
//! Wayland and a wgpu presenter, ticked every frame. `dom-canvas`
//! (X11/XWayland) owns **none** of them: the wasm CAD session inside the WebView
//! is the only `cad-db` authority (iron rule 8), so the native shell keeps only
//! window/WebView lifetime, files/config, the popup gate and the shell admission
//! policy.
//!
//! This module exists so that decision is **one pure value** instead of a
//! condition repeated (and eventually forgotten) at each construction site:
//! [`init`](super::init) and [`tick`](super::tick) read the same
//! [`SurfacePlan`]. A source-structure test pins that the two construction sites
//! are guarded by it, and the pure test pins what each mode allows.
//!
//! Deliberately **not** a runtime capability probe: the mode is decided once,
//! before GTK (ADR 0024 D4), and never switches.

use crate::backend::{surface_mode, SurfaceMode};

/// What the native half of this process is allowed to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfacePlan {
    /// Whether the native `CadSession`/`HostPlatform` composition may be created.
    ///
    /// `true` only for `native-below`: on `dom-canvas` a native session would be a
    /// second `cad-db` authority next to the wasm one (ADR 0024 D2 / iron rule 8).
    pub native_cad_session: bool,
    /// Whether the GTK overlay bridge may be installed (the `GtkDrawingArea`, the
    /// Wayland `wl_subsurface` shim, the wgpu presenter and the webview reparent).
    pub gtk_bridge: bool,
    /// Whether the per-frame tick may drive a native frame (surface layout, GPU
    /// init, session bind, CAD render/present).
    pub per_frame_tick: bool,
}

impl SurfacePlan {
    /// The plan of one carrier. Pure.
    pub const fn for_mode(mode: SurfaceMode) -> Self {
        match mode {
            SurfaceMode::NativeBelow => Self {
                native_cad_session: true,
                gtk_bridge: true,
                per_frame_tick: true,
            },
            SurfaceMode::DomCanvas => Self {
                native_cad_session: false,
                gtk_bridge: false,
                per_frame_tick: false,
            },
        }
    }
}

/// The plan of the surface mode this process installed.
pub fn current() -> SurfacePlan {
    SurfacePlan::for_mode(surface_mode())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom_canvas_constructs_no_native_cad_piece() {
        // ADR 0024 D2 / iron rule 8: the wasm session is the only authority, so
        // every native CAD piece is off — no session, no GTK drawing area /
        // subsurface / presenter, no per-frame render.
        let plan = SurfacePlan::for_mode(SurfaceMode::DomCanvas);
        assert!(!plan.native_cad_session);
        assert!(!plan.gtk_bridge);
        assert!(!plan.per_frame_tick);
    }

    #[test]
    fn native_below_keeps_the_whole_native_path() {
        // Wayland must stay behaviourally identical: everything stays on.
        let plan = SurfacePlan::for_mode(SurfaceMode::NativeBelow);
        assert!(plan.native_cad_session);
        assert!(plan.gtk_bridge);
        assert!(plan.per_frame_tick);
    }

    #[test]
    fn the_composition_sites_are_guarded_by_the_plan() {
        // Source-structure contract (not a runtime mount): the two places that
        // would create native CAD state read the plan instead of assuming it.
        // `include_str!` keeps this next to the code it pins.
        let init = include_str!("init.rs");
        assert!(
            init.contains("if !plan.native_cad_session"),
            "init.rs must gate the native CAD composition on the surface plan"
        );
        assert!(
            init.contains("if !plan.gtk_bridge"),
            "init.rs must gate the GTK overlay install on the surface plan"
        );
        assert!(
            !init.contains("CadSession::new("),
            "init.rs must not create a CadSession at all: the session is created by the tick"
        );

        let tick = include_str!("tick.rs");
        assert!(
            tick.contains("surface_plan().per_frame_tick"),
            "tick.rs must skip the native frame path when the plan says so"
        );
    }

    #[test]
    fn the_installed_plan_follows_the_installed_mode() {
        let plan = current();
        assert_eq!(plan, SurfacePlan::for_mode(crate::backend::surface_mode()));
    }
}
